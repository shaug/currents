//! Evidence-supported openings and pure replacement previews.
use crate::{
    catalog::{CatalogEngine, incomplete},
    recognition::fact,
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
fn opening_basis(
    state: &DomainState,
    request: &ActivityRequestV1,
) -> Result<Option<Date>, EngineError> {
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
    Ok(Some(cutoff))
}
pub(crate) fn initial(
    state: &DomainState,
    request: &ActivityRequestV1,
    admission: bool,
) -> Result<Option<AccountingSegmentV1>, EngineError> {
    let Some(_) = opening_basis(state, request)? else {
        return Ok(None);
    };
    let terms = request.financial.as_ref().ok_or_else(incomplete)?;
    let opening = terms.opening.as_ref().ok_or_else(incomplete)?;
    let Some(AuthoritativeRecordV1::AccountComponent(c)) = state.records.get(&opening.component_id)
    else {
        return Err(incomplete());
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
        let (root, successor) = segment_family(state, c.id)?.ok_or_else(incomplete)?;
        let historical = if root.id == opening.segment_id {
            root
        } else {
            successor
                .filter(|s| s.id == opening.segment_id)
                .ok_or_else(incomplete)?
        };
        if historical.opening != terms.recognition_id {
            return Err(EngineError::invalid());
        }
        if historical.prior_segment.is_some() {
            // The owner was previously re-admitted against the complete
            // prior prefix. Its immutable authored segment is not a new root.
            let index = state.accounting.as_ref().ok_or_else(incomplete)?;
            if !index.reversed.contains(&root.opening)
                || index.reversed.contains(&historical.opening)
                || !index.recognitions.contains_key(&historical.opening)
            {
                return Err(EngineError::invalid());
            }
            return Ok(Some(historical.clone()));
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
/// The delivered family is deliberately one root plus at most one standalone
/// successor. No fork, self-link, second hop or reused opening is active.
pub(crate) fn segment_family(
    state: &DomainState,
    component: RecordId,
) -> Result<Option<(&AccountingSegmentV1, Option<&AccountingSegmentV1>)>, EngineError> {
    let mut root = None;
    let mut successor = None;
    for (id, record) in &state.records {
        let AuthoritativeRecordV1::AccountingSegment(segment) = record else {
            continue;
        };
        if segment.component != component {
            continue;
        }
        if segment.id != *id || id.0.is_nil() {
            return Err(EngineError::invalid());
        }
        let slot = if segment.prior_segment.is_none() {
            &mut root
        } else {
            &mut successor
        };
        if slot.replace(segment).is_some() {
            return Err(EngineError::invalid());
        }
    }
    let Some(root) = root else {
        if successor.is_some() {
            return Err(EngineError::invalid());
        }
        return Ok(None);
    };
    if let Some(s) = successor
        && (s.prior_segment != Some(root.id)
            || s.opening == root.opening
            || s.replacement_closure.first() != Some(&root.opening)
            || s.included_activity.is_empty()
            || s.included_activity
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != s.included_activity.len()
            || s.included_activity
                .iter()
                .any(|id| *id == root.opening || *id == s.opening)
            || !matches!(s.residual_disposition, SourceValue::Absent))
    {
        return Err(EngineError::invalid());
    }
    if let Some(s) = successor {
        let mut owners = state.records.values().filter_map(|record| match record {
            AuthoritativeRecordV1::SegmentReplacementAuthority(owner) if owner.segment == s.id => {
                Some(owner)
            }
            _ => None,
        });
        if let Some(owner) = owners.next() {
            if owner.schema_version != 1
                || owner.prior_segment != root.id
                || owner.affected_accounts.is_empty()
                || owner.counterparts.is_empty()
                || owners.next().is_some()
                || owner
                    .counterparts
                    .iter()
                    .map(|pair| pair.original)
                    .collect::<BTreeSet<_>>()
                    != s.replacement_closure.iter().copied().skip(1).collect()
            {
                return Err(EngineError::invalid());
            }
        } else if s.replacement_closure != [root.opening] {
            return Err(EngineError::invalid());
        }
    }
    Ok(Some((root, successor)))
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
        let Some((root, successor)) = segment_family(state, c.id)? else {
            return Err(incomplete());
        };
        let active = successor.unwrap_or(root);
        if active.id != opening.segment_id {
            continue;
        }
        if a.recognition != Some(active.opening) {
            return Err(EngineError::invalid());
        }
        let cutoff = civil(&opening.cutoff)?;
        for (account, effective) in movements.clone() {
            if account == c.account_id && civil(effective)? <= cutoff {
                return Err(incomplete());
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct SegmentReemissionV1 {
    pub original: RecognitionId,
    pub replacement: RecognitionId,
    pub posting_ids: Vec<RecordId>,
}

#[derive(Clone, Debug)]
pub struct SegmentPreviewRequestV1 {
    pub request: SegmentRequestV1,
    pub opening: ActivityRequestV1,
    pub backfill: Vec<ActivityRequestV1>,
    pub reemissions: Vec<SegmentReemissionV1>,
}

pub use crate::correction_dependents::OrganizationDispositionItem;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeedsOrganizationDisposition {
    pub prior_basis: AccountingPrefixV1,
    pub prior_segment: RecordId,
    pub affected: Vec<OrganizationDispositionItem>,
}

#[derive(Clone, Debug)]
pub enum SegmentPreviewV1 {
    Ready(Box<DecisionPlan>),
    NeedsOrganizationDisposition(NeedsOrganizationDisposition),
    UnresolvedResidual(Amount),
    ClosureTooLarge { operations: usize, maximum: usize },
}

enum SegmentPreparation {
    Validated {
        plan: Box<DecisionPlan>,
        question: Option<NeedsOrganizationDisposition>,
    },
    Stopped(SegmentPreviewV1),
}

impl SegmentPreparation {
    fn finish(self) -> SegmentPreviewV1 {
        match self {
            Self::Stopped(outcome) => outcome,
            Self::Validated {
                question: Some(question),
                ..
            } => SegmentPreviewV1::NeedsOrganizationDisposition(question),
            Self::Validated {
                plan,
                question: None,
            } => SegmentPreviewV1::Ready(plan),
        }
    }
}

pub struct SegmentEngine;

// C05 supports native Gregorian years 0001..=9999. This preview is
// intentionally civil-only: no Instant conversion or Unknown fallback.
fn native_civil(value: &TemporalValue) -> Result<Date, EngineError> {
    let date = civil(value)?;
    if !(1..=9999).contains(&date.year()) {
        return Err(EngineError::invalid());
    }
    Ok(date)
}

fn exact_date_policy<'a>(
    state: &'a DomainState,
    reference: &PolicyReferenceV1,
) -> Result<&'a DateConversionPolicyV1, EngineError> {
    let indexed = state
        .accounting
        .as_ref()
        .and_then(|index| index.date_policies.get(&(reference.id, reference.version)))
        .ok_or_else(incomplete)?;
    let mut retained = state.records.values().filter_map(|record| match record {
        AuthoritativeRecordV1::DateConversionPolicy(p)
            if p.id == reference.id && p.version == reference.version =>
        {
            Some(p)
        }
        _ => None,
    });
    if indexed.id != reference.id
        || indexed.version != reference.version
        || indexed.version == 0
        || retained.next().is_some_and(|p| p != indexed)
        || retained.next().is_some()
    {
        return Err(EngineError::invalid());
    }
    Ok(indexed)
}

/// This is shared only with the private proposal path; ordinary
/// adoption continues to enforce its initial, never overlapping boundary.
pub(crate) fn validate_draft_window(
    state: &DomainState,
    request: &ActivityRequestV1,
    bound_component: RecordId,
    prior_cutoff: Date,
    new_cutoff: Date,
) -> Result<(), EngineError> {
    let invalid = EngineError::invalid;
    let Some(AuthoritativeRecordV1::AccountComponent(component)) =
        state.records.get(&bound_component)
    else {
        return Err(incomplete());
    };
    let terms = request.financial.as_ref().ok_or_else(incomplete)?;
    // For supported explicit civil dates, mirror C05's booking rule without
    // making the domain crate depend on the native accounting adapter.
    exact_date_policy(state, &terms.date_policy)?;
    let booked = native_civil(&terms.bookkeeping_date)?;
    if !(2..=1024).contains(&terms.postings.len()) {
        return Err(invalid());
    }
    let mut native_totals = BTreeMap::<UnitId, Amount>::new();
    for posting in &terms.postings {
        if native_civil(&posting.posting)? != booked {
            return Err(invalid());
        }
        // The normal adoption path checks the declared currency exponent.
        // C05 also requires representability in native signed scale 18,
        // including every ordered partial sum for each unit.
        let native = posting.amount.checked_rescale(18)?;
        let total =
            native_totals
                .entry(native.unit_id)
                .or_insert(Amount::new(0, 18, native.unit_id)?);
        *total = total.checked_add(&native)?;
    }
    if native_totals.values().any(|amount| amount.coefficient != 0) {
        return Err(invalid());
    }
    let [leg] = request.legs.as_slice() else {
        return Err(EngineError::version());
    };
    let [source] = leg.sources.as_slice() else {
        return Err(EngineError::version());
    };
    if new_cutoff >= prior_cutoff
        || request.status != ActivityStatusV1::Posted
        || request.prior_pending.is_some()
        || terms.unresolved_account.is_some()
        || leg.leg.account_id != component.account_id
    {
        return Err(invalid());
    }
    if let Some(opening) = &terms.opening {
        if opening.component_id != bound_component
            || opening_basis(state, request)? != Some(new_cutoff)
            || !opening.included_activity.is_empty()
        {
            return Err(invalid());
        }
    } else {
        let day = source_time(state, source, ObservationKind::Movement)?;
        if day <= new_cutoff
            || day > prior_cutoff
            || native_civil(&leg.leg.effective)? != day
            || terms
                .postings
                .iter()
                .any(|p| native_civil(&p.effective) != Ok(day))
        {
            return Err(invalid());
        }
    }
    Ok(())
}

fn original_recognition<'a>(
    state: &'a DomainState,
    index: &AccountingIndexV1,
    id: RecognitionId,
) -> Result<&'a RecognitionV1, EngineError> {
    let invalid = EngineError::invalid;
    let Some(AuthoritativeRecordV1::Recognition(recognition)) = state.records.get(&RecordId(id.0))
    else {
        return Err(incomplete());
    };
    let [entry] = index
        .recognitions
        .get(&id)
        .ok_or_else(incomplete)?
        .as_slice()
    else {
        return Err(EngineError::version());
    };
    if recognition.id != id
        || entry.recognition_id != id
        || index.reversed.contains(&id)
        || entry.reverses.is_some()
        || entry.postings.len() != recognition.postings.len()
        || recognition.postings.is_empty()
        || exact_date_policy(state, &recognition.date_policy)? != &entry.date_policy
        || native_civil(&recognition.bookkeeping_date)?.to_string() != entry.booking_date
    {
        return Err(invalid());
    }
    for (typed, frozen) in recognition.postings.iter().zip(&entry.postings) {
        let mut normalized = typed.clone();
        normalized.amount = typed.amount.normalized()?;
        if normalized != frozen.original
            || native_civil(&typed.effective)?.to_string() != entry.value_date
            || native_civil(&typed.posting)?.to_string() != entry.booking_date
        {
            return Err(invalid());
        }
    }
    Ok(recognition)
}

impl SegmentEngine {
    /// Bounded one-off preview of an earlier opening with ordered intervening
    /// evidence-backed movements. No segment owner is admitted or dispatched.
    pub fn replace(
        state: &DomainState,
        preview: &SegmentPreviewRequestV1,
    ) -> Result<SegmentPreviewV1, EngineError> {
        Self::prepare(state, preview).map(SegmentPreparation::finish)
    }

    fn prepare(
        state: &DomainState,
        preview: &SegmentPreviewRequestV1,
    ) -> Result<SegmentPreparation, EngineError> {
        let request = &preview.request;
        let invalid = EngineError::invalid;
        let index = state.accounting.as_ref().ok_or_else(incomplete)?;
        if index.prefix.through != state.through
            || index.prefix.domain_digest != state.domain_digest
            || index.prefix.native_head.semantics_version != 1
            || !matches!(request.residual_disposition, SourceValue::Absent)
            || preview.reemissions.len() > 1024
            || request.replacement_closure.len() > 1025
            || request.approved_accounts.len() > 1024
        {
            return Err(invalid());
        }
        if preview.backfill.is_empty() || preview.backfill.len() != request.included_activity.len()
        {
            return Err(invalid());
        }
        let prior_id = request.prior_segment.ok_or_else(incomplete)?;
        let Some(AuthoritativeRecordV1::AccountingSegment(prior)) = state.records.get(&prior_id)
        else {
            return Err(incomplete());
        };
        if prior.id != prior_id
            || prior.component != request.component
            || prior.prior_segment.is_some()
            || state.records.values().any(|record| {
                matches!(record,
                AuthoritativeRecordV1::AccountingSegment(other)
                    if other.component == prior.component && other.id != prior.id)
            })
        {
            return Err(invalid());
        }
        let Some(AuthoritativeRecordV1::AccountComponent(component)) =
            state.records.get(&prior.component)
        else {
            return Err(incomplete());
        };

        let mut prior_openings = state.records.values().filter_map(|r| match r {
            AuthoritativeRecordV1::ActivityAdoption(a) if a.recognition == Some(prior.opening) => {
                a.opening.as_ref()
            }
            _ => None,
        });
        let old_opening = prior_openings.next().ok_or_else(incomplete)?;
        if prior_openings.next().is_some()
            || old_opening.segment_id != prior.id
            || old_opening.component_id != prior.component
        {
            return Err(invalid());
        }
        // Exact aggregate agreement is not proof that previously included
        // movement evidence is represented in the replacement interval.
        if !old_opening.included_activity.is_empty() || !prior.included_activity.is_empty() {
            return Err(EngineError::version());
        }
        let old_cutoff = native_civil(&old_opening.cutoff)?;
        let old_anchor = original_recognition(state, index, prior.opening)?;
        if old_anchor.kind != RecognitionKind::Opening {
            return Err(invalid());
        }
        let proposed_opening = preview
            .opening
            .financial
            .as_ref()
            .and_then(|terms| terms.opening.as_ref())
            .ok_or_else(incomplete)?;
        let new_cutoff = native_civil(&proposed_opening.cutoff)?;

        // Discover active accounting consequences rather than accepting a
        // caller-supplied subset of original entries. Expand across all
        // retained transfer relations, including their unchanged counterparts.
        let mut closure = BTreeSet::from([prior.opening]);
        for (&id, entries) in &index.recognitions {
            if !index.reversed.contains(&id)
                && entries
                    .iter()
                    .flat_map(|entry| &entry.postings)
                    .any(|p| p.original.account_id == component.account_id)
            {
                let r = original_recognition(state, index, id)?;
                if id != prior.opening
                    && (r.kind != RecognitionKind::Activity
                        || r.postings.iter().any(|p| {
                            native_civil(&p.effective).is_err()
                                || native_civil(&p.effective).is_ok_and(|day| day <= old_cutoff)
                        }))
                {
                    return Err(EngineError::version());
                }
                closure.insert(id);
                if closure.len() > 1025 {
                    return Err(invalid());
                }
            }
        }
        // A missing indexed recognition is not evidence that it was retired.
        for record in state.records.values() {
            if let AuthoritativeRecordV1::Recognition(r) = record
                && r.postings
                    .iter()
                    .any(|p| p.account_id == component.account_id)
                && !index.recognitions.contains_key(&r.id)
            {
                return Err(incomplete());
            }
        }
        loop {
            let before = closure.len();
            for record in state.records.values() {
                match record {
                    AuthoritativeRecordV1::AssociationRevision(association)
                        if association
                            .recognitions
                            .iter()
                            .any(|id| closure.contains(id))
                            || association
                                .retired_recognitions
                                .iter()
                                .any(|id| closure.contains(id)) =>
                    {
                        if association.kind != RelationshipKind::Transfer
                            || association.prior_revision.is_some()
                            || !association.retired_recognitions.is_empty()
                        {
                            return Err(EngineError::version());
                        }
                        for id in &association.recognitions {
                            original_recognition(state, index, *id)?;
                            closure.insert(*id);
                            if closure.len() > 1025 {
                                return Err(invalid());
                            }
                        }
                    }
                    // Dependency remapping is not in this preview slice.
                    AuthoritativeRecordV1::Association(_)
                    | AuthoritativeRecordV1::Allocation(_)
                    | AuthoritativeRecordV1::OccurrenceMatch(_)
                    | AuthoritativeRecordV1::Review(_) => return Err(EngineError::version()),
                    _ => {}
                }
            }
            if closure.len() == before {
                break;
            }
        }
        let requested = request
            .replacement_closure
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if requested != closure || requested.len() != request.replacement_closure.len() {
            return Err(invalid());
        }
        let mut untouched = closure.clone();
        untouched.remove(&prior.opening);
        if preview.reemissions.len() != untouched.len()
            || preview
                .reemissions
                .iter()
                .map(|r| r.original)
                .collect::<BTreeSet<_>>()
                != untouched
        {
            return Err(invalid());
        }

        let approved: BTreeSet<_> = request.approved_accounts.iter().copied().collect();
        if approved.len() != request.approved_accounts.len()
            || !approved.contains(&component.account_id)
            || old_anchor
                .postings
                .iter()
                .any(|p| !approved.contains(&p.account_id))
        {
            return Err(invalid());
        }
        // Count only matched typed proposals and the exact retained closure,
        // before cloning/admitting draft authority or materializing native work.
        let mut proposed_ids = BTreeSet::from([request.opening]);
        let mut proposed_sources = BTreeSet::new();
        for (draft, expected) in preview.backfill.iter().zip(&request.included_activity) {
            let Some(terms) = draft.financial.as_ref() else {
                return Err(invalid());
            };
            let [leg] = draft.legs.as_slice() else {
                return Err(invalid());
            };
            let [source] = leg.sources.as_slice() else {
                return Err(invalid());
            };
            if terms.recognition_id != *expected
                || !proposed_ids.insert(*expected)
                || expected.0.is_nil()
                || terms.opening.is_some()
                || draft.status != ActivityStatusV1::Posted
                || draft.prior_pending.is_some()
                || leg.leg.account_id != component.account_id
                || state.records.contains_key(&RecordId(expected.0))
                || !proposed_sources.insert((source.observation, source.facet))
                || !matches!(
                    state.records.get(&source.observation),
                    Some(AuthoritativeRecordV1::ObservationVersion(_))
                )
                || !matches!(
                    state.records.get(&source.parse_product),
                    Some(AuthoritativeRecordV1::AddressedParseProduct(_))
                )
            {
                return Err(invalid());
            }
        }
        let Some(opening_terms) = preview.opening.financial.as_ref() else {
            return Err(invalid());
        };
        if opening_terms.recognition_id != request.opening
            || request.opening.0.is_nil()
            || state.records.contains_key(&RecordId(request.opening.0))
            || preview.opening.status != ActivityStatusV1::Posted
            || preview.opening.prior_pending.is_some()
            || opening_basis(state, &preview.opening)? != Some(new_cutoff)
        {
            return Err(invalid());
        }
        let operations = closure
            .len()
            .checked_add(preview.reemissions.len())
            .and_then(|n| n.checked_add(1))
            .and_then(|n| n.checked_add(preview.backfill.len()))
            .ok_or_else(invalid)?;
        if operations > MAX_CLOSURE_OPERATIONS {
            return Ok(SegmentPreparation::Stopped(
                SegmentPreviewV1::ClosureTooLarge {
                    operations,
                    maximum: MAX_CLOSURE_OPERATIONS,
                },
            ));
        }

        let drafts = std::iter::once(&preview.opening).chain(&preview.backfill);
        let mut seen_facts = BTreeSet::new();
        let mut records = Vec::new();
        let mut replacements = Vec::with_capacity(proposed_ids.len());
        for draft in drafts.clone() {
            for source in draft.legs.iter().flat_map(|leg| &leg.sources) {
                if !seen_facts.insert(fact(state, source)?) {
                    return Err(invalid());
                }
            }
            let plan = crate::recognition::RecognitionEngine::segment_draft(
                state,
                draft,
                prior.component,
                old_cutoff,
                new_cutoff,
            )?;
            let AccountingIntent::Recognition(recognition) = plan.accounting else {
                return Err(invalid());
            };
            if recognition
                .postings
                .iter()
                .any(|p| !approved.contains(&p.account_id))
            {
                return Err(invalid());
            }
            records.extend(plan.records);
            replacements.push(recognition);
        }
        let sum = |r: &RecognitionV1| {
            r.postings
                .iter()
                .filter(|p| p.account_id == component.account_id)
                .try_fold(Amount::new(0, 0, component.unit_id)?, |acc, posting| {
                    acc.checked_add(&posting.amount)
                })
        };
        let old_balance = sum(old_anchor)?;
        if old_balance.normalized()? != old_opening.amount.normalized()? {
            return Err(invalid());
        }
        // All new caller-supplied identities are checked against each other,
        // retained typed authority, and frozen historical posting identities.
        let mut identities: BTreeSet<_> = index
            .recognitions
            .values()
            .flatten()
            .flat_map(|entry| &entry.postings)
            .map(|p| p.original.id)
            .collect();
        let mut fresh = |id: RecordId| {
            if id.0.is_nil() || state.records.contains_key(&id) || !identities.insert(id) {
                Err(invalid())
            } else {
                Ok(())
            }
        };
        fresh(proposed_opening.segment_id)?;
        for draft in drafts {
            fresh(draft.adoption_id)?;
            fresh(draft.event.id)?;
            for leg in &draft.legs {
                fresh(RecordId(leg.leg.id.0))?;
            }
        }
        for r in &replacements {
            fresh(RecordId(r.id.0))?;
            fresh(r.lineage)?;
            for p in &r.postings {
                fresh(p.id)?;
            }
        }
        for reemit in &preview.reemissions {
            let original = original_recognition(state, index, reemit.original)?;
            if original.postings.len() != reemit.posting_ids.len() {
                return Err(invalid());
            }
            fresh(RecordId(reemit.replacement.0))?;
            for id in &reemit.posting_ids {
                fresh(*id)?;
            }
        }
        let replacement_balance = replacements
            .iter()
            .try_fold(Amount::new(0, 0, component.unit_id)?, |total, r| {
                total.checked_add(&sum(r)?)
            })?;
        let difference = replacement_balance
            .checked_add(&old_balance.checked_neg()?)?
            .normalized()?;
        if difference.coefficient != 0 {
            return Ok(SegmentPreparation::Stopped(
                SegmentPreviewV1::UnresolvedResidual(difference),
            ));
        }
        let dependents = {
            // Real changes are the removed anchor and its new anchor/backfill.
            // Exact retained counterpart re-emissions change IDs, not predicate inputs.
            let changed = std::iter::once(old_anchor)
                .chain(replacements.iter())
                .collect::<Vec<_>>();
            crate::correction_dependents::invalidations(
                state,
                &changed,
                &old_anchor.leg_ids,
                &[prior.id],
            )?
        };
        let mut corrections = vec![CorrectionV1 {
            active_recognition: prior.opening,
            replacement: replacements,
            remapping: vec![],
            reason: "Replace segment opening at an earlier reconciled boundary".into(),
        }];
        for reemit in &preview.reemissions {
            let original = original_recognition(state, index, reemit.original)?;
            let mut replacement = original.clone();
            replacement.id = reemit.replacement;
            for (posting, id) in replacement.postings.iter_mut().zip(&reemit.posting_ids) {
                posting.id = *id;
            }
            corrections.push(CorrectionV1 {
                active_recognition: reemit.original,
                replacement: vec![replacement.clone()],
                remapping: vec![],
                reason: "Preserve whole retained consequence across segment replacement".into(),
            });
            records.push(AuthoritativeRecordV1::Recognition(replacement));
        }
        records.push(AuthoritativeRecordV1::AccountingSegment(
            AccountingSegmentV1 {
                id: proposed_opening.segment_id,
                component: prior.component,
                prior_segment: Some(prior.id),
                opening: request.opening,
                included_activity: request.included_activity.clone(),
                replacement_closure: request.replacement_closure.clone(),
                approved_accounts: request.approved_accounts.clone(),
                residual_disposition: SourceValue::Absent,
            },
        ));
        let plan = Box::new(DecisionPlan {
            records,
            invalidations: std::iter::once(prior.id)
                .chain(closure.into_iter().map(|id| RecordId(id.0)))
                .chain(dependents.invalidations)
                .collect(),
            accounting: AccountingIntent::Corrections(corrections),
        });
        let question =
            (!dependents.organization_questions.is_empty()).then(|| NeedsOrganizationDisposition {
                prior_basis: index.prefix.clone(),
                prior_segment: prior.id,
                affected: dependents.organization_questions,
            });
        Ok(SegmentPreparation::Validated { plan, question })
    }
}

impl SegmentEngine {
    /// Re-emit a retained transfer's entire exact family with one typed
    /// witness. No external dispatch or dependency remapping is enabled.
    pub fn replace_owned(
        state: &DomainState,
        preview: &SegmentPreviewRequestV1,
        authority_id: RecordId,
    ) -> Result<SegmentPreviewV1, EngineError> {
        let invalid = EngineError::invalid;
        if authority_id.0.is_nil()
            || state.records.contains_key(&authority_id)
            || preview.reemissions.is_empty()
        {
            return Err(invalid());
        }
        let (mut plan, question) = match Self::prepare(state, preview)? {
            SegmentPreparation::Validated { plan, question } => (plan, question),
            SegmentPreparation::Stopped(outcome) => return Ok(outcome),
        };
        let Some(AuthoritativeRecordV1::AccountingSegment(segment)) = plan.records.last() else {
            return Err(invalid());
        };
        let prior_segment = segment.prior_segment.ok_or_else(invalid)?;
        if plan.records.iter().any(|record| match record {
            AuthoritativeRecordV1::FinancialEvent(e) => e.id == authority_id,
            AuthoritativeRecordV1::FinancialLeg(l) => RecordId(l.id.0) == authority_id,
            AuthoritativeRecordV1::ActivityAdoption(a) => a.id == authority_id,
            AuthoritativeRecordV1::Recognition(r) => {
                RecordId(r.id.0) == authority_id
                    || r.lineage == authority_id
                    || r.postings.iter().any(|p| p.id == authority_id)
            }
            AuthoritativeRecordV1::AccountingSegment(s) => s.id == authority_id,
            _ => true,
        }) {
            return Err(invalid());
        }
        let (root, None) = segment_family(state, segment.component)?.ok_or_else(invalid)? else {
            return Err(invalid());
        };
        if prior_segment != root.id || segment.replacement_closure.first() != Some(&root.opening) {
            return Err(invalid());
        }
        let index = state.accounting.as_ref().ok_or_else(incomplete)?;
        if index
            .recognitions
            .values()
            .flatten()
            .flat_map(|entry| &entry.postings)
            .any(|p| p.original.id == authority_id)
        {
            return Err(invalid());
        }
        let AccountingIntent::Corrections(corrections) = &plan.accounting else {
            return Err(invalid());
        };
        let mut affected = BTreeSet::new();
        for correction in corrections {
            let old = original_recognition(state, index, correction.active_recognition)?;
            affected.extend(old.postings.iter().map(|p| p.account_id));
            affected.extend(
                correction
                    .replacement
                    .iter()
                    .flat_map(|r| &r.postings)
                    .map(|p| p.account_id),
            );
        }
        let mut counterparts = Vec::with_capacity(preview.reemissions.len());
        for r in &preview.reemissions {
            let mut owners = state.records.values().filter_map(|record| match record {
                AuthoritativeRecordV1::AssociationRevision(a)
                    if a.recognitions.contains(&r.original) =>
                {
                    Some(a)
                }
                _ => None,
            });
            let owner = owners.next().ok_or_else(incomplete)?;
            if owners.next().is_some()
                || owner.kind != RelationshipKind::Transfer
                || owner.prior_revision.is_some()
                || !owner.retired_recognitions.is_empty()
            {
                return Err(EngineError::version());
            }
            counterparts.push(SegmentCounterpartBindingV1 {
                original: r.original,
                replacement: r.replacement,
            });
        }
        if affected.is_empty() || affected.len() > 4096 {
            return Err(invalid());
        }
        plan.records
            .push(AuthoritativeRecordV1::SegmentReplacementAuthority(
                SegmentReplacementAuthorityV1 {
                    id: authority_id,
                    schema_version: 1,
                    segment: segment.id,
                    prior_segment,
                    affected_accounts: affected.into_iter().collect(),
                    counterparts,
                },
            ));
        Ok(SegmentPreparation::Validated { plan, question }.finish())
    }
}
