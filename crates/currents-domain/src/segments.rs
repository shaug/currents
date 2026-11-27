//! Evidence-supported openings and one-component segment replacement.
use crate::{
    catalog::{CatalogEngine, incomplete},
    recognition::{adopt_drafts, fact, restore},
};
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};
fn civil(value: &TemporalValue) -> Result<Date, EngineError> {
    match value {
        TemporalValue::Date { civil, .. } => Ok(*civil),
        _ => Err(incomplete()),
    }
}
pub(crate) fn source_time(
    state: &DomainState,
    binding: &SourceFacetBindingV1,
    kind: ObservationKind,
) -> Result<Date, EngineError> {
    fact(state, binding)?;
    let Some(AuthoritativeRecordV1::AddressedParseProduct(p)) =
        state.records.get(&binding.parse_product)
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
type Basis<'a> = (
    &'a AccountComponentV1,
    &'a RecognitionTermsV1,
    &'a OpeningBasisV1,
    Date,
);
/// The supported opening terms that initial adoption and replacement share.
fn opening_basis<'a>(
    state: &'a DomainState,
    request: &'a ActivityRequestV1,
) -> Result<Option<Basis<'a>>, EngineError> {
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
        || CatalogEngine::current(state, c.id)?.origin != AccountOriginV1::External
        || opening.amount.normalized()? != leg.leg.movement.normalized()?
        || opening.evidence.amount.normalized()? != opening.amount.normalized()?
        || leg.sources != vec![opening.evidence.clone()]
    {
        return Err(EngineError::invalid());
    }
    let cutoff = civil(&opening.cutoff)?;
    if source_time(state, &opening.evidence, ObservationKind::Balance)? != cutoff
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
            || source_time(state, source, ObservationKind::Movement)? > cutoff
            || !facts.insert(fact(state, source)?)
        {
            return Err(EngineError::invalid());
        }
    }
    Ok(Some((c, terms, opening, cutoff)))
}
pub(crate) fn initial(
    state: &DomainState,
    request: &ActivityRequestV1,
    admission: bool,
) -> Result<Option<AccountingSegmentV1>, EngineError> {
    let Some((c, terms, opening, _)) = opening_basis(state, request)? else {
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
            if anchor.replacement_closure != [prior.opening]
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
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SegmentPreviewV1 {
    Ready(Box<DecisionPlan>),
    /// The new opening plus backfill at the old cutoff, minus the old opening.
    /// No plan exists: the owner must resolve the difference.
    UnresolvedResidual(Amount),
}
pub struct SegmentEngine;
impl SegmentEngine {
    /// Preview a replacement against an exact validated prefix. A ready plan
    /// reverses the old opening, then adds the new opening and each backfill.
    pub fn replace(
        state: &DomainState,
        request: &SegmentReplacementV1,
    ) -> Result<SegmentPreviewV1, EngineError> {
        let invalid = EngineError::invalid;
        let index = state.accounting.as_ref().ok_or_else(incomplete)?;
        if index.prefix.through != state.through
            || index.prefix.domain_digest != state.domain_digest
            || index.prefix.native_head.semantics_version != 1
            || request.backfill.len() > 4096
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
        // Each movement the old opening included would need its own place in
        // the new history; this leaf does not map them.
        if !old_basis.included_activity.is_empty() {
            return Err(EngineError::version());
        }
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
        let Some((component, _, basis, new_cutoff)) = opening_basis(state, &request.opening)?
        else {
            return Err(invalid());
        };
        if component.id != c.id
            || new_cutoff > old_cutoff
            || !basis.included_activity.is_empty()
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
            // Later CUR-016 leaves own closures across external accounts.
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
                within(source_time(state, source, ObservationKind::Movement)?)?;
            }
        }
        // Later leaves re-emit other recognitions; none may sit at or before
        // the old cutoff on this account.
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
        // Later leaves invalidate dependent records. Until then, refuse any
        // that names the account, the old opening or its legs.
        let named: BTreeSet<String> = [c.account_id.0, prior.opening.0, old.id.0, old.event_id.0]
            .into_iter()
            .chain(old.legs.iter().map(|l| l.0))
            .chain(old_opening.postings.iter().map(|p| p.id.0))
            .map(|id| id.to_string())
            .collect();
        for record in state.records.values() {
            if matches!(
                record,
                AuthoritativeRecordV1::ReconciliationClaim(_)
                    | AuthoritativeRecordV1::Organization(_)
                    | AuthoritativeRecordV1::Review(_)
                    | AuthoritativeRecordV1::ForecastRun(_)
            ) && names_any(
                &serde_json::to_value(record).map_err(|_| invalid())?,
                &named,
            ) {
                return Err(EngineError::version());
            }
        }
        let drafts = adopt_drafts(
            state,
            std::iter::once(&request.opening).chain(&request.backfill),
        )?;
        let mut touched: BTreeSet<_> = old_opening.postings.iter().map(|p| p.account_id).collect();
        touched.extend(
            drafts
                .iter()
                .flat_map(|(_, r)| &r.postings)
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
            replacement_closure: vec![prior.opening],
            approved_accounts: request.approved_accounts.clone(),
            residual_disposition: SourceValue::Absent,
        };
        let mut records: Vec<_> = records.into_iter().flatten().collect();
        records.push(AuthoritativeRecordV1::AccountingSegment(successor));
        Ok(SegmentPreviewV1::Ready(Box::new(DecisionPlan {
            records,
            invalidations: [prior.id, RecordId(prior.opening.0)]
                .into_iter()
                .chain(old.legs.iter().map(|l| RecordId(l.0)))
                .collect(),
            accounting: AccountingIntent::Correction(CorrectionV1 {
                active_recognition: prior.opening,
                replacement,
                remapping: vec![],
                reason: "Replace the segment opening at an earlier proven boundary".into(),
            }),
        })))
    }
}
fn names_any(value: &serde_json::Value, ids: &BTreeSet<String>) -> bool {
    match value {
        serde_json::Value::String(text) => ids.contains(text),
        serde_json::Value::Array(items) => items.iter().any(|v| names_any(v, ids)),
        serde_json::Value::Object(fields) => fields.values().any(|v| names_any(v, ids)),
        _ => false,
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
    let draft = |id: RecognitionId| {
        let mut owners = next.records.values().filter_map(|r| match r {
            AuthoritativeRecordV1::ActivityAdoption(a) if a.recognition == Some(id) => Some(a),
            _ => None,
        });
        let (Some(owner), None) = (owners.next(), owners.next()) else {
            return Err(invalid());
        };
        restore(next, owner)
    };
    let request = SegmentReplacementV1 {
        prior_segment: successor.prior_segment.ok_or_else(invalid)?,
        opening: draft(successor.opening)?,
        backfill: successor
            .included_activity
            .iter()
            .map(|id| draft(*id))
            .collect::<Result<_, _>>()?,
        approved_accounts: successor.approved_accounts.clone(),
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
    if records != plan.records.as_slice() {
        return Err(invalid());
    }
    Ok(())
}
