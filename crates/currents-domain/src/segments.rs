//! Evidence-supported openings and one-component segment replacement.
use crate::{
    associations::IdentityEngine,
    catalog::incomplete,
    dependents::{self, Change, Dependents, OrganizationQuestionV1},
    lookup::Lookup,
    recognition::{adopt_drafts, fact_in, restore},
};
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};
fn civil(value: &TemporalValue) -> Result<Date, EngineError> {
    match value {
        TemporalValue::Date { civil, .. } => Ok(*civil),
        _ => Err(incomplete()),
    }
}
fn source_time(
    lookup: &Lookup,
    binding: &SourceFacetBindingV1,
    kind: ObservationKind,
) -> Result<Date, EngineError> {
    fact_in(lookup, binding)?;
    let Some(AuthoritativeRecordV1::AddressedParseProduct(p)) =
        lookup.state.records.get(&binding.parse_product)
    else {
        return Err(incomplete());
    };
    let f = p
        .product
        .facets
        .iter()
        .find(|f| f.lineage_key == binding.facet)
        .ok_or_else(incomplete)?;
    if f.kind != kind {
        return Err(EngineError::invalid());
    }
    civil(&f.temporal)
}
type Basis<'s, 'r> = (
    &'s AccountComponentV1,
    &'r RecognitionTermsV1,
    &'r OpeningBasisV1,
    Date,
);
/// The supported opening terms that initial adoption and replacement share.
fn opening_basis<'s, 'r>(
    lookup: &Lookup<'s>,
    request: &'r ActivityRequestV1,
) -> Result<Option<Basis<'s, 'r>>, EngineError> {
    let state = lookup.state;
    let Some(terms) = &request.financial else {
        return Ok(None);
    };
    let Some(opening) = &terms.opening else {
        return Ok(None);
    };
    if request.status != ActivityStatusV1::Posted
        || request.prior_pending.is_some()
        || terms.unresolved_account.is_some()
        || request.legs.len() != 1
        || opening.included_activity.len() > 4096
        || opening.segment_id.0.is_nil()
    {
        return Err(EngineError::invalid());
    }
    let leg = &request.legs[0];
    let Some(AuthoritativeRecordV1::AccountComponent(c)) = state.records.get(&opening.component_id)
    else {
        return Err(incomplete());
    };
    if c.id != opening.component_id
        || c.account_id != leg.leg.account_id
        || c.unit_id != opening.amount.unit_id
        || !matches!(c.opening_boundary, SourceValue::Unknown)
        || lookup.current(c.id)?.origin != AccountOriginV1::External
        || opening.amount.normalized()? != leg.leg.movement.normalized()?
        || opening.evidence.amount.normalized()? != opening.amount.normalized()?
        || leg.sources != vec![opening.evidence.clone()]
    {
        return Err(EngineError::invalid());
    }
    let cutoff = civil(&opening.cutoff)?;
    if source_time(lookup, &opening.evidence, ObservationKind::Balance)? != cutoff
        || civil(&leg.leg.effective)? != cutoff
        || civil(&terms.bookkeeping_date)? != cutoff
    {
        return Err(EngineError::invalid());
    }
    // C05 freezes posting.effective as the native value date, independently of
    // the adopted leg date. Initial anchors support explicit civil boundaries;
    // an instant needing policy conversion is not silently compared as a date.
    for posting in &terms.postings {
        if civil(&posting.effective)? != cutoff {
            return Err(EngineError::invalid());
        }
    }
    let mut facts = BTreeSet::new();
    for source in &opening.included_activity {
        if source.amount.unit_id != c.unit_id
            || source_time(lookup, source, ObservationKind::Movement)? > cutoff
            || !facts.insert(fact_in(lookup, source)?)
        {
            return Err(EngineError::invalid());
        }
    }
    Ok(Some((c, terms, opening, cutoff)))
}
pub(crate) fn initial(
    lookup: &Lookup,
    request: &ActivityRequestV1,
    admission: bool,
) -> Result<Option<AccountingSegmentV1>, EngineError> {
    let state = lookup.state;
    let Some((c, terms, opening, _)) = opening_basis(lookup, request)? else {
        return Ok(None);
    };
    if admission
        && state.records.values().any(|r| match r {
            AuthoritativeRecordV1::AccountingSegment(s) => s.component == c.id,
            AuthoritativeRecordV1::Recognition(r) => {
                r.postings.iter().any(|p| p.account_id == c.account_id)
            }
            _ => false,
        })
    {
        return Err(incomplete());
    }
    if !admission {
        // Retained history must still hold one linear chain per component.
        active(state, c.id)?;
        let Some(AuthoritativeRecordV1::AccountingSegment(anchor)) =
            state.records.get(&opening.segment_id)
        else {
            return Err(incomplete());
        };
        if anchor.component != c.id || anchor.opening != terms.recognition_id {
            return Err(EngineError::invalid());
        }
        if let Some(prior) = anchor.prior_segment {
            // Replay admitted this successor by deriving its whole plan again.
            // The native index proves that the opening it replaced is reversed.
            let Some(AuthoritativeRecordV1::AccountingSegment(prior)) = state.records.get(&prior)
            else {
                return Err(incomplete());
            };
            let index = state.accounting.as_ref().ok_or_else(incomplete)?;
            if anchor.replacement_closure.first() != Some(&prior.opening)
                || !index.reversed.contains(&prior.opening)
            {
                return Err(EngineError::invalid());
            }
            return Ok(Some(anchor.clone()));
        }
    }
    Ok(Some(AccountingSegmentV1 {
        id: opening.segment_id,
        component: c.id,
        prior_segment: None,
        opening: terms.recognition_id,
        included_activity: vec![],
        replacement_closure: vec![],
        approved_accounts: terms.postings.iter().map(|p| p.account_id).collect(),
        residual_disposition: SourceValue::Absent,
    }))
}
/// One component's segments form one chain back to its root, with no forks
/// or self links. The only active segment is the tip that no successor names.
pub(crate) fn active(
    state: &DomainState,
    component: RecordId,
) -> Result<Option<&AccountingSegmentV1>, EngineError> {
    let mut segments = BTreeMap::new();
    let mut named = BTreeSet::new();
    for (id, record) in &state.records {
        let AuthoritativeRecordV1::AccountingSegment(s) = record else {
            continue;
        };
        if s.component != component {
            continue;
        }
        if s.id != *id
            || s.prior_segment == Some(s.id)
            || s.prior_segment.is_some_and(|prior| !named.insert(prior))
        {
            return Err(EngineError::invalid());
        }
        segments.insert(s.id, s);
    }
    let mut tips = segments.values().filter(|s| !named.contains(&s.id));
    let tip = match (tips.next(), tips.next()) {
        (None, _) if segments.is_empty() => return Ok(None),
        (Some(tip), None) => *tip,
        _ => return Err(EngineError::invalid()),
    };
    let mut current = tip;
    let mut chain = BTreeSet::from([tip.id]);
    while let Some(prior) = current.prior_segment {
        current = segments.get(&prior).ok_or_else(EngineError::invalid)?;
        if !chain.insert(current.id) {
            return Err(EngineError::invalid());
        }
    }
    if chain.len() != segments.len() {
        return Err(EngineError::invalid());
    }
    Ok(Some(tip))
}
/// Activity at/before a supported opening needs explicit reconciliation, not a
/// second monetary entry. The exact included facts are also ownership checked.
pub(crate) fn activity_boundary(
    state: &DomainState,
    request: &ActivityRequestV1,
) -> Result<(), EngineError> {
    if request
        .financial
        .as_ref()
        .is_some_and(|f| f.opening.is_some())
    {
        return Ok(());
    }
    movement_boundary(
        state,
        request
            .financial
            .iter()
            .flat_map(|terms| &terms.postings)
            .map(|p| (p.account_id, &p.effective))
            .chain(
                request
                    .legs
                    .iter()
                    .map(|l| (l.leg.account_id, &l.leg.effective)),
            ),
    )
}
/// Check new movements against the exact prior prefix. Association remaps do
/// not replace opening segments and must obey the same strict cutoff.
pub(crate) fn movement_boundary<'a>(
    state: &DomainState,
    movements: impl Iterator<Item = (AccountId, &'a TemporalValue)> + Clone,
) -> Result<(), EngineError> {
    for record in state.records.values() {
        let AuthoritativeRecordV1::ActivityAdoption(a) = record else {
            continue;
        };
        let Some(opening) = &a.opening else { continue };
        let Some(AuthoritativeRecordV1::AccountComponent(c)) =
            state.records.get(&opening.component_id)
        else {
            return Err(incomplete());
        };
        let cutoff = civil(&opening.cutoff)?;
        for (account, effective) in movements.clone() {
            if account == c.account_id && civil(effective)? <= cutoff {
                return Err(incomplete());
            }
        }
    }
    Ok(())
}

/// Replace one component's active segment with an earlier or same-date opening
/// and the movements between the two cutoffs. This is a C08 preview input, not
/// a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentReplacementV1 {
    /// The active segment whose opening is retired.
    pub prior_segment: RecordId,
    /// A posted opening draft backed by balance evidence. Its `segment_id`
    /// names the successor segment.
    pub opening: ActivityRequestV1,
    /// Posted movement drafts after the new cutoff and through the old one,
    /// in plan order.
    pub backfill: Vec<ActivityRequestV1>,
    /// Exactly the accounts the plan touches.
    pub approved_accounts: Vec<AccountId>,
    /// Transfer families that a backfilled leg joins, one new revision each.
    pub transfers: Vec<TransferJoinV1>,
    /// One re-emission for each current recognition of each joined family.
    /// With the old opening, their originals are the closure the request names.
    pub reemissions: Vec<ReemissionV1>,
}
/// A backfilled leg that joins an existing transfer family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferJoinV1 {
    /// The fresh ID of the family's new revision.
    pub revision: RecordId,
    /// The family's current revision, which the new one follows.
    pub prior_revision: RecordId,
    /// The backfilled leg that the new revision adds.
    pub leg: LegId,
    /// The relationship fact that places the leg in this family: a posted
    /// movement in the family's own capture source and group.
    pub source: CapturedFacetBindingV1,
}
/// A transfer recognition reversed once and issued again with fresh
/// recognition and posting IDs and unchanged economics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReemissionV1 {
    /// The fresh ID of the history record that links the two.
    pub id: RecordId,
    pub original: RecognitionId,
    pub replacement: RecognitionV1,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SegmentPreviewV1 {
    Ready(Box<DecisionPlan>),
    /// The new opening plus backfill at the old cutoff, minus the old opening.
    /// No plan exists: the owner must resolve the difference.
    UnresolvedResidual(Amount),
    /// Organization records target a leg the replacement retires. No plan
    /// exists: the owner must decide what each record now organizes.
    NeedsOrganizationDecision(Vec<OrganizationQuestionV1>),
    /// The closure needs more native operations than one atomic C05 closure
    /// may hold. No plan exists, and the limit does not change.
    TooLarge {
        operations: usize,
        limit: usize,
    },
    /// The commit would hold more records, its frozen manifest included, than
    /// one C04 commit may hold. No plan exists, and the limit does not change.
    TooManyRecords {
        records: usize,
        limit: usize,
    },
}
fn clearing(lookup: &Lookup, account: AccountId) -> Result<bool, EngineError> {
    let component = lookup.components(account).first().ok_or_else(incomplete)?;
    Ok(matches!(
        lookup.current(component.id)?.origin,
        AccountOriginV1::Internal(control) if control.role == ControlRoleV1::Clearing
    ))
}
/// The joined leg's relationship evidence decides its family. The fact is a
/// posted movement with the leg's date and amount, from the same capture
/// source and group as every fact the family already holds, and no current
/// family revision allocates it.
fn join_evidence(
    state: &DomainState,
    heads: &BTreeMap<RecordId, AssociationRevisionV1>,
    head: &AssociationRevisionV1,
    join: &TransferJoinV1,
    leg: &FinancialLegV1,
) -> Result<(), EngineError> {
    let invalid = EngineError::invalid;
    let unit = leg.movement.unit_id;
    let fact = IdentityEngine::captured_facet(state, &join.source, unit)?;
    if fact.status != RelationshipSourceStatusV1::Posted
        || fact.role != RelationshipFacetRoleV1::Movement
        || fact.effective != leg.effective
        || fact.amount != leg.movement.normalized()?
        || head.captured_allocations.is_empty()
    {
        return Err(invalid());
    }
    let source = |capture: RecordId| match state.records.get(&capture) {
        Some(AuthoritativeRecordV1::RelationshipCapture(c)) => Ok(&c.source),
        _ => Err(invalid()),
    };
    let joined_source = source(join.source.capture)?;
    for edge in &head.captured_allocations {
        let held = IdentityEngine::retained_facet(state, &edge.source, edge.allocated.unit_id)?;
        if source(edge.source.capture)? != joined_source || held.group != fact.group {
            return Err(invalid());
        }
    }
    if crate::associations::held_facts(state, heads.values())?.contains(&fact.fact_key) {
        return Err(invalid());
    }
    Ok(())
}
/// The new revision of a joined family: the backfilled leg, its evidence and
/// its recognition added, every current recognition replaced by its
/// re-emission.
fn joined_revision(
    head: &AssociationRevisionV1,
    join: &TransferJoinV1,
    leg: &FinancialLegV1,
    recognition: RecognitionId,
    reemissions: &[ReemissionV1],
) -> Result<AssociationRevisionV1, EngineError> {
    let mut residuals = BTreeMap::new();
    for amount in &head.residuals {
        if residuals.insert(amount.unit_id, *amount).is_some() {
            return Err(EngineError::invalid());
        }
    }
    let unit = leg.movement.unit_id;
    let residual = residuals.entry(unit).or_insert(Amount::new(0, 0, unit)?);
    *residual = residual.checked_add(&leg.movement)?;
    let again = |id: &RecognitionId| {
        reemissions
            .iter()
            .find(|r| r.original == *id)
            .map(|r| r.replacement.id)
            .ok_or_else(EngineError::invalid)
    };
    Ok(AssociationRevisionV1 {
        id: join.revision,
        schema_version: 1,
        root: head.root,
        prior_revision: Some(head.id),
        kind: head.kind,
        legs: head.legs.iter().copied().chain([join.leg]).collect(),
        allocations: head.allocations.clone(),
        captured_allocations: head
            .captured_allocations
            .iter()
            .cloned()
            .chain([CapturedFacetAllocationV1 {
                source: join.source.clone(),
                leg: join.leg,
                allocated: leg.movement,
                mode: FacetAllocationModeV1::Additive,
            }])
            .collect(),
        recognitions: head
            .recognitions
            .iter()
            .map(again)
            .chain([Ok(recognition)])
            .collect::<Result<_, _>>()?,
        residuals: residuals
            .values()
            .map(|a| a.normalized())
            .collect::<Result<_, _>>()?,
        superseded_adoptions: head.superseded_adoptions.clone(),
        survivor: head.survivor,
        retired_recognitions: head.recognitions.clone(),
        interpretation: head.interpretation.clone(),
        governing_policy: head.governing_policy.clone(),
    })
}
pub struct SegmentEngine;
impl SegmentEngine {
    /// Preview a replacement against an exact validated prefix. The engine
    /// derives the closure: the old opening, and the current recognitions of
    /// each transfer family that a backfilled leg joins. A ready plan reverses
    /// the old opening, adds the new opening and each backfill, then reverses
    /// each transfer recognition once and issues it again.
    pub fn replace(
        state: &DomainState,
        request: &SegmentReplacementV1,
    ) -> Result<SegmentPreviewV1, EngineError> {
        let invalid = EngineError::invalid;
        // Every draft is planned against this one prefix, so its lookups are
        // shared rather than repeated per draft.
        let lookup = Lookup::new(state);
        let index = state.accounting.as_ref().ok_or_else(incomplete)?;
        if index.prefix.through != state.through
            || index.prefix.domain_digest != state.domain_digest
            || index.prefix.native_head.semantics_version != 1
            || request.transfers.len() > 1024
            || request.reemissions.len() > 4096
        {
            return Err(invalid());
        }
        let Some(AuthoritativeRecordV1::AccountingSegment(prior)) =
            state.records.get(&request.prior_segment)
        else {
            return Err(incomplete());
        };
        // A replaced segment is no longer active, so it cannot be replaced again.
        if active(state, prior.component)? != Some(prior) {
            return Err(invalid());
        }
        let Some(AuthoritativeRecordV1::AccountComponent(c)) = state.records.get(&prior.component)
        else {
            return Err(incomplete());
        };
        let mut owners = state.records.values().filter_map(|r| match r {
            AuthoritativeRecordV1::ActivityAdoption(a) if a.recognition == Some(prior.opening) => {
                Some(a)
            }
            _ => None,
        });
        let (Some(old), None) = (owners.next(), owners.next()) else {
            return Err(invalid());
        };
        let Some(old_basis) = old.opening.as_ref().filter(|o| o.segment_id == prior.id) else {
            return Err(invalid());
        };
        let old_cutoff = civil(&old_basis.cutoff)?;
        let Some(AuthoritativeRecordV1::Recognition(old_opening)) =
            state.records.get(&RecordId(prior.opening.0))
        else {
            return Err(incomplete());
        };
        if index.reversed.contains(&prior.opening)
            || !index.recognitions.contains_key(&prior.opening)
        {
            return Err(invalid());
        }
        // C05 reverses each native entry of a retired recognition and adds
        // one entry per replacement: the old opening for the new opening and
        // each backfill, then each re-emitted original for its re-emission.
        let entries = |id| index.recognitions.get(id).map_or(0, Vec::len);
        let operations = request.reemissions.iter().fold(
            entries(&prior.opening)
                .saturating_add(1)
                .saturating_add(request.backfill.len()),
            |n, again| n.saturating_add(entries(&again.original)).saturating_add(1),
        );
        if operations > MAX_CLOSURE_OPERATIONS {
            return Ok(SegmentPreviewV1::TooLarge {
                operations,
                limit: MAX_CLOSURE_OPERATIONS,
            });
        }
        let Some((component, _, basis, new_cutoff)) = opening_basis(&lookup, &request.opening)?
        else {
            return Err(invalid());
        };
        if component.id != c.id
            || new_cutoff > old_cutoff
            || state.records.contains_key(&basis.segment_id)
        {
            return Err(invalid());
        }
        let within = |day: Date| {
            if day <= new_cutoff || day > old_cutoff {
                Err(invalid())
            } else {
                Ok(())
            }
        };
        for draft in &request.backfill {
            let terms = draft.financial.as_ref().ok_or_else(invalid)?;
            // A backfill's legs are on the anchored account. Its money reaches
            // another external account only by joining a transfer family.
            if draft.legs.iter().any(|l| l.leg.account_id != c.account_id) {
                return Err(EngineError::version());
            }
            if terms.opening.is_some() {
                return Err(invalid());
            }
            for day in draft
                .legs
                .iter()
                .map(|l| &l.leg.effective)
                .chain(terms.postings.iter().map(|p| &p.effective))
            {
                within(civil(day)?)?;
            }
            for source in draft.legs.iter().flat_map(|l| &l.sources) {
                within(source_time(&lookup, source, ObservationKind::Movement)?)?;
            }
        }
        // Ordinary adoption refuses activity at or before the cutoff, so the
        // old opening is the only recognition there that the closure takes.
        for (id, entries) in &index.recognitions {
            if *id == prior.opening || index.reversed.contains(id) {
                continue;
            }
            for posting in entries.iter().flat_map(|e| &e.postings) {
                let posting = &posting.original;
                if posting.account_id == c.account_id
                    && civil(&posting.effective).is_ok_and(|day| day <= old_cutoff)
                {
                    return Err(EngineError::version());
                }
            }
        }
        // Each joined family adds its current recognitions to the closure.
        let heads = crate::associations::current_heads(state)?;
        let mut joined = Vec::new();
        let mut derived = BTreeSet::new();
        let mut roots = BTreeSet::new();
        for join in &request.transfers {
            let head = heads
                .values()
                .find(|h| h.id == join.prior_revision)
                .ok_or_else(invalid)?;
            // Payroll and duplicate families are outside this replacement.
            if head.kind != RelationshipKind::Transfer {
                return Err(EngineError::version());
            }
            let mut drafts = request
                .backfill
                .iter()
                .filter(|d| d.legs.iter().any(|l| l.leg.id == join.leg));
            let (Some(draft), None) = (drafts.next(), drafts.next()) else {
                return Err(invalid());
            };
            let [leg] = draft.legs.as_slice() else {
                return Err(invalid());
            };
            if !roots.insert(head.root) || joined.iter().any(|(_, l, _)| *l == &leg.leg) {
                return Err(invalid());
            }
            derived.extend(head.recognitions.iter().copied());
            join_evidence(state, &heads, head, join, &leg.leg)?;
            joined.push((join, &leg.leg, head));
        }
        // The request names exactly the derived closure, each recognition once.
        let mut named = BTreeSet::new();
        for again in &request.reemissions {
            if !named.insert(again.original) {
                return Err(invalid());
            }
        }
        if named != derived {
            return Err(invalid());
        }
        // Each re-emission is an identity-only correction of its original.
        let mut originals = Vec::new();
        let mut reissues = Vec::new();
        for again in &request.reemissions {
            let Some(AuthoritativeRecordV1::Recognition(original)) =
                state.records.get(&RecordId(again.original.0))
            else {
                return Err(incomplete());
            };
            // Activity on the account after the old cutoff keeps its IDs.
            if original
                .postings
                .iter()
                .any(|p| p.account_id == c.account_id)
            {
                return Err(EngineError::version());
            }
            let reissue = CorrectionV1 {
                active_recognition: again.original,
                replacement: vec![again.replacement.clone()],
                remapping: vec![],
                reason: "Issue a transfer recognition again for a replaced segment".into(),
            };
            crate::correction::CorrectionEngine::correct(state, &reissue)?;
            originals.push(original);
            reissues.push(reissue);
        }
        // Each fact the old opening included moves exactly once: to a
        // backfill's sources or to the new opening's inclusion map.
        let mut moved = BTreeMap::<_, usize>::new();
        for source in basis.included_activity.iter().chain(
            request
                .backfill
                .iter()
                .flat_map(|d| d.legs.iter().flat_map(|l| &l.sources)),
        ) {
            let count = moved.entry(fact_in(&lookup, source)?).or_default();
            *count = count.saturating_add(1);
        }
        for source in &old_basis.included_activity {
            let key = fact_in(&lookup, source)?;
            if moved.get(&key) != Some(&1) {
                return Err(invalid());
            }
        }
        // Every identity the plan adds beyond the drafts is fresh.
        let added: Vec<RecordId> = request
            .transfers
            .iter()
            .map(|j| j.revision)
            .chain(request.reemissions.iter().flat_map(|r| {
                [r.id, RecordId(r.replacement.id.0)]
                    .into_iter()
                    .chain(r.replacement.postings.iter().map(|p| p.id))
            }))
            .collect();
        for id in &added {
            if id.0.is_nil()
                || state.records.contains_key(id)
                || index.recognitions.contains_key(&RecognitionId(id.0))
                || index
                    .recognitions
                    .values()
                    .flatten()
                    .flat_map(|e| &e.postings)
                    .any(|p| p.original.id == *id)
            {
                return Err(invalid());
            }
        }
        let drafts = adopt_drafts(
            &lookup,
            std::iter::once(&request.opening).chain(&request.backfill),
            old.id,
            &added,
        )?;
        // A draft posts to clearing exactly when its leg joins a transfer
        // family, against a clearing account that family already uses.
        let mut revisions = Vec::new();
        for (_, recognition) in &drafts {
            let mut cleared = false;
            for p in &recognition.postings {
                cleared |= clearing(&lookup, p.account_id)?;
            }
            let join = joined
                .iter()
                .find(|(_, leg, _)| recognition.leg_ids.contains(&leg.id));
            match (join, recognition.postings.as_slice()) {
                (None, _) if !cleared => {}
                (Some((join, leg, head)), [anchor, counter])
                    if anchor.account_id == c.account_id
                        && clearing(&lookup, counter.account_id)?
                        && originals.iter().any(|r| {
                            head.recognitions.contains(&r.id)
                                && r.postings
                                    .iter()
                                    .any(|p| p.account_id == counter.account_id)
                        }) =>
                {
                    revisions.push(joined_revision(
                        head,
                        join,
                        leg,
                        recognition.id,
                        &request.reemissions,
                    )?);
                }
                _ => return Err(invalid()),
            }
        }
        let mut touched: BTreeSet<_> = old_opening.postings.iter().map(|p| p.account_id).collect();
        touched.extend(
            drafts
                .iter()
                .map(|(_, r)| r)
                .chain(request.reemissions.iter().map(|r| &r.replacement))
                .flat_map(|r| &r.postings)
                .map(|p| p.account_id),
        );
        let approved: BTreeSet<_> = request.approved_accounts.iter().copied().collect();
        if approved.len() != request.approved_accounts.len() || approved != touched {
            return Err(invalid());
        }
        let on_account = |r: &RecognitionV1| {
            r.postings
                .iter()
                .filter(|p| p.account_id == c.account_id)
                .try_fold(Amount::new(0, 0, c.unit_id)?, |total, p| {
                    total.checked_add(&p.amount)
                })
        };
        let mut residual = on_account(old_opening)?.checked_neg()?;
        for (_, r) in &drafts {
            residual = residual.checked_add(&on_account(r)?)?;
        }
        let residual = residual.normalized()?;
        if residual.coefficient != 0 {
            return Ok(SegmentPreviewV1::UnresolvedResidual(residual));
        }
        let (records, replacement): (Vec<_>, Vec<_>) = drafts.into_iter().unzip();
        let Some((opening, backfill)) = replacement.split_first() else {
            return Err(invalid());
        };
        let successor = AccountingSegmentV1 {
            id: basis.segment_id,
            component: c.id,
            prior_segment: Some(prior.id),
            opening: opening.id,
            included_activity: backfill.iter().map(|r| r.id).collect(),
            replacement_closure: std::iter::once(prior.opening)
                .chain(request.reemissions.iter().map(|r| r.original))
                .collect(),
            approved_accounts: request.approved_accounts.clone(),
            residual_disposition: SourceValue::Absent,
        };
        let retire = CorrectionV1 {
            active_recognition: prior.opening,
            replacement,
            remapping: vec![],
            reason: "Replace the segment opening at an earlier proven boundary".into(),
        };
        let corrections: Vec<_> = std::iter::once(retire).chain(reissues).collect();
        // Claims in the changed scope are invalidated in this commit.
        // Organization on the old opening's legs needs the owner's decision.
        let retired_revisions = joined.iter().map(|(join, _, _)| join.prior_revision);
        let change = Change::new(
            state,
            &corrections,
            old.legs.iter().copied(),
            [prior.id, old.id, old.event_id]
                .into_iter()
                .chain(retired_revisions.clone()),
        )?;
        let Dependents { claims, questions } = dependents::assess(state, &change)?;
        if !questions.is_empty() {
            return Ok(SegmentPreviewV1::NeedsOrganizationDecision(questions));
        }
        let invalidations = [prior.id, RecordId(prior.opening.0)]
            .into_iter()
            .chain(old.legs.iter().map(|l| RecordId(l.0)))
            .chain(retired_revisions)
            .chain(request.reemissions.iter().map(|r| RecordId(r.original.0)))
            .chain(claims)
            .collect();
        let mut records: Vec<_> = records.into_iter().flatten().collect();
        records.extend(
            request
                .reemissions
                .iter()
                .map(|r| AuthoritativeRecordV1::Recognition(r.replacement.clone())),
        );
        records.extend(
            revisions
                .into_iter()
                .map(AuthoritativeRecordV1::AssociationRevision),
        );
        records.push(AuthoritativeRecordV1::AccountingSegment(successor));
        records.extend(request.reemissions.iter().map(|r| {
            AuthoritativeRecordV1::SegmentReemission(SegmentReemissionV1 {
                id: r.id,
                schema_version: 1,
                segment: basis.segment_id,
                original: r.original,
                replacement: r.replacement.id,
            })
        }));
        // C04 commits these records with the plan's frozen manifest.
        let commit_records = records.len().saturating_add(1);
        if commit_records > MAX_COMMIT_ITEMS {
            return Ok(SegmentPreviewV1::TooManyRecords {
                records: commit_records,
                limit: MAX_COMMIT_ITEMS,
            });
        }
        let accounting = match <[_; 1]>::try_from(corrections) {
            Ok([retire]) => AccountingIntent::Correction(retire),
            Err(corrections) => AccountingIntent::Corrections(corrections),
        };
        Ok(SegmentPreviewV1::Ready(Box::new(DecisionPlan {
            records,
            invalidations,
            accounting,
        })))
    }
}
/// Replay re-admits a successor segment only by deriving the same plan again
/// from the prior prefix. The commit holds exactly that plan, then its manifest.
pub(crate) fn readmit(
    prior: &DomainState,
    next: &DomainState,
    commit: &DomainCommitV1,
    successor: &AccountingSegmentV1,
) -> Result<(), EngineError> {
    let invalid = EngineError::invalid;
    let mut owners = BTreeMap::<RecognitionId, Vec<&ActivityAdoptionV1>>::new();
    for record in next.records.values() {
        if let AuthoritativeRecordV1::ActivityAdoption(a) = record
            && let Some(id) = a.recognition
        {
            owners.entry(id).or_default().push(a);
        }
    }
    let draft = |id: RecognitionId| match owners.get(&id).map(Vec::as_slice) {
        Some([owner]) => restore(next, owner),
        _ => Err(invalid()),
    };
    // Each joined family's new revision adds the backfilled leg and its
    // relationship evidence last.
    let mut transfers = Vec::new();
    let mut reemissions = Vec::new();
    for record in &commit.records {
        match record {
            AuthoritativeRecordV1::AssociationRevision(r) => {
                let leg = *r.legs.last().ok_or_else(invalid)?;
                let edge = r.captured_allocations.last().filter(|e| e.leg == leg);
                transfers.push(TransferJoinV1 {
                    revision: r.id,
                    prior_revision: r.prior_revision.ok_or_else(invalid)?,
                    leg,
                    source: edge.ok_or_else(invalid)?.source.clone(),
                });
            }
            AuthoritativeRecordV1::SegmentReemission(link) => {
                let Some(AuthoritativeRecordV1::Recognition(again)) =
                    next.records.get(&RecordId(link.replacement.0))
                else {
                    return Err(invalid());
                };
                reemissions.push(ReemissionV1 {
                    id: link.id,
                    original: link.original,
                    replacement: again.clone(),
                });
            }
            _ => {}
        }
    }
    let request = SegmentReplacementV1 {
        prior_segment: successor.prior_segment.ok_or_else(invalid)?,
        opening: draft(successor.opening)?,
        backfill: successor
            .included_activity
            .iter()
            .map(|id| draft(*id))
            .collect::<Result<_, _>>()?,
        approved_accounts: successor.approved_accounts.clone(),
        transfers,
        reemissions,
    };
    let SegmentPreviewV1::Ready(plan) = SegmentEngine::replace(prior, &request)? else {
        return Err(invalid());
    };
    let [
        records @ ..,
        AuthoritativeRecordV1::FrozenAccountingManifest(_),
    ] = commit.records.as_slice()
    else {
        return Err(invalid());
    };
    if records != plan.records.as_slice()
        || commit.command_result.effects.invalidations != plan.invalidations
    {
        return Err(invalid());
    }
    Ok(())
}
