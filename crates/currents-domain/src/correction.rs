//! Proposals for corrections to an accepted recognition.
use currents_types::{
    AccountingIndexV1, AccountingIntent, ActivityAdoptionV1, ActivityStatusV1,
    AuthoritativeRecordV1, DecisionPlan, DomainState, EngineError, RecognitionCorrectionRemapV1,
    RecognitionCorrectionRevisionV1, RecognitionId, RecognitionKind, RecognitionV1, RecordId,
};
use std::collections::{BTreeMap, BTreeSet};

pub use currents_types::CorrectionV1 as CorrectionRequestV1;

pub struct CorrectionEngine;

impl CorrectionEngine {
    /// Preview a one-for-one correction that changes recognition/posting identities only.
    ///
    /// This slice is not wired to service dispatch or replay admission. Broader replacement
    /// and remapping must remain unavailable until their ownership and dependency checks exist.
    pub fn correct(
        state: &DomainState,
        request: &CorrectionRequestV1,
    ) -> Result<DecisionPlan, EngineError> {
        let invalid = EngineError::invalid;
        let index = state.accounting.as_ref().ok_or_else(invalid)?;
        let key = RecordId(request.active_recognition.0);
        let Some(AuthoritativeRecordV1::Recognition(original)) = state.records.get(&key) else {
            return Err(invalid());
        };
        if original.id != request.active_recognition
            || index.prefix.through != state.through
            || index.prefix.domain_digest != state.domain_digest
            || index.prefix.native_head.semantics_version != 1
            || index.reversed.contains(&original.id)
            || index
                .recognitions
                .get(&original.id)
                .is_none_or(Vec::is_empty)
            || request.reason.trim().is_empty()
        {
            return Err(invalid());
        }
        let [replacement] = request.replacement.as_slice() else {
            return Err(EngineError::version());
        };
        if !request.remapping.is_empty() {
            return Err(EngineError::version());
        }
        if replacement.id.0.is_nil()
            || state.records.contains_key(&RecordId(replacement.id.0))
            || index.recognitions.contains_key(&replacement.id)
            || original.postings.len() != replacement.postings.len()
        {
            return Err(invalid());
        }
        let mut ids = BTreeSet::new();
        let mut unchanged = replacement.clone();
        unchanged.id = original.id;
        for (posting, previous) in unchanged.postings.iter_mut().zip(&original.postings) {
            if posting.id.0.is_nil()
                || !ids.insert(posting.id)
                || index.recognitions.values().flatten().any(|entry| {
                    entry
                        .postings
                        .iter()
                        .any(|seen| seen.original.id == posting.id)
                })
            {
                return Err(invalid());
            }
            posting.id = previous.id;
        }
        if &unchanged != original {
            return Err(EngineError::version());
        }
        Ok(DecisionPlan {
            records: vec![AuthoritativeRecordV1::Recognition(replacement.clone())],
            invalidations: std::iter::once(key)
                .chain(original.leg_ids.iter().map(|id| RecordId(id.0)))
                .collect(),
            accounting: AccountingIntent::Correction(request.clone()),
        })
    }
    /// Produce an append-only revision for the standalone identity correction slice.
    /// Dispatch remains disabled; dependency-aware remapping is not yet implemented.
    pub fn correct_durable(
        state: &DomainState,
        revision_id: RecordId,
        request: &CorrectionRequestV1,
    ) -> Result<DecisionPlan, EngineError> {
        let mut plan = Self::correct(state, request)?;
        let original_id = request.active_recognition;
        let replacement = &request.replacement[0];
        let index = state.accounting.as_ref().ok_or_else(EngineError::invalid)?;
        if revision_id.0.is_nil()
            || revision_id == RecordId(replacement.id.0)
            || revision_id == replacement.lineage
            || replacement.postings.iter().any(|p| p.id == revision_id)
            || index
                .recognitions
                .values()
                .flatten()
                .flat_map(|e| &e.postings)
                .any(|p| p.original.id == revision_id)
            || state.records.contains_key(&revision_id)
        {
            return Err(EngineError::invalid());
        }
        let chains = correction_chains(state)?;
        let root = chains.root_of(original_id);
        let owner = original_adoption(state, root)?;
        let in_family = |candidate: &RecognitionId| chains.root_of(*candidate) == root;
        let target_in_family = |candidate: &RecordId| in_family(&RecognitionId(candidate.0));
        if state.records.values().any(|record| match record {
            AuthoritativeRecordV1::Association(_)
            | AuthoritativeRecordV1::Allocation(_)
            | AuthoritativeRecordV1::ReconciliationClaim(_)
            | AuthoritativeRecordV1::Organization(_)
            | AuthoritativeRecordV1::OccurrenceMatch(_)
            | AuthoritativeRecordV1::ForecastRun(_) => true,
            AuthoritativeRecordV1::AssociationRevision(a) => {
                a.recognitions.iter().any(in_family)
                    || a.retired_recognitions.iter().any(in_family)
                    || a.superseded_adoptions.contains(&owner.id)
                    || a.legs.iter().any(|leg| owner.legs.contains(leg))
            }
            AuthoritativeRecordV1::Review(review) => {
                review.targets.iter().any(target_in_family)
                    || review.targets.contains(&owner.id)
                    || owner
                        .legs
                        .iter()
                        .any(|leg| review.targets.contains(&RecordId(leg.0)))
            }
            AuthoritativeRecordV1::AccountingSegment(s) => {
                in_family(&s.opening)
                    || s.included_activity.iter().any(in_family)
                    || s.replacement_closure.iter().any(in_family)
            }
            AuthoritativeRecordV1::SegmentReemission(r) => {
                in_family(&r.original) || in_family(&r.replacement)
            }
            _ => false,
        }) {
            return Err(EngineError::version());
        }
        plan.records.insert(
            0,
            AuthoritativeRecordV1::RecognitionCorrectionRevision(RecognitionCorrectionRevisionV1 {
                id: revision_id,
                schema_version: 1,
                prior_recognition: original_id,
                replacement_recognition: replacement.id,
                reason: request.reason.clone(),
                remap_disposition: RecognitionCorrectionRemapV1::UnchangedStableLegs,
            }),
        );
        Ok(plan)
    }
}

/// An immutable rooted chain. The root owns historical evidence; the native
/// index proves which entries are retired and which terminal is live.
#[derive(Default)]
pub(crate) struct CorrectionChains {
    roots: BTreeMap<RecognitionId, RecognitionId>,
}
impl CorrectionChains {
    fn root_of(&self, id: RecognitionId) -> RecognitionId {
        self.roots.get(&id).copied().unwrap_or(id)
    }

    pub(crate) fn is_replacement(&self, id: RecognitionId) -> bool {
        self.root_of(id) != id
    }
}

fn original_adoption(
    state: &DomainState,
    root: RecognitionId,
) -> Result<&ActivityAdoptionV1, EngineError> {
    let mut owners = state
        .records
        .iter()
        .filter_map(|(key, record)| match record {
            AuthoritativeRecordV1::ActivityAdoption(a) if a.recognition == Some(root) => {
                Some((key, a))
            }
            _ => None,
        });
    let (key, owner) = owners.next().ok_or_else(EngineError::version)?;
    if owners.next().is_some()
        || *key != owner.id
        || owner.opening.is_some()
        || owner.status != ActivityStatusV1::Posted
    {
        return Err(EngineError::version());
    }
    Ok(owner)
}

fn retained_recognition<'a>(
    state: &'a DomainState,
    index: &AccountingIndexV1,
    id: RecognitionId,
) -> Result<&'a RecognitionV1, EngineError> {
    let invalid = EngineError::invalid;
    let Some(AuthoritativeRecordV1::Recognition(r)) = state.records.get(&RecordId(id.0)) else {
        return Err(invalid());
    };
    let [frozen] = index.recognitions.get(&id).ok_or_else(invalid)?.as_slice() else {
        return Err(invalid());
    };
    if r.id != id
        || r.kind != RecognitionKind::Activity
        || frozen.recognition_id != id
        || frozen.reverses.is_some()
        || r.postings.len() != frozen.postings.len()
    {
        return Err(invalid());
    }
    for (typed, native) in r.postings.iter().zip(&frozen.postings) {
        let mut canonical = typed.clone();
        canonical.amount = typed.amount.normalized()?;
        if canonical != native.original {
            return Err(invalid());
        }
    }
    Ok(r)
}

pub(crate) fn correction_chains(state: &DomainState) -> Result<CorrectionChains, EngineError> {
    let invalid = EngineError::invalid;
    let mut successors = BTreeMap::new();
    let mut predecessors = BTreeMap::new();
    for (id, record) in &state.records {
        let AuthoritativeRecordV1::RecognitionCorrectionRevision(revision) = record else {
            continue;
        };
        if revision.schema_version != 1 {
            return Err(EngineError::version());
        }
        if *id != revision.id
            || id.0.is_nil()
            || revision.prior_recognition == revision.replacement_recognition
            || revision.reason.trim().is_empty()
            || successors
                .insert(revision.prior_recognition, revision.replacement_recognition)
                .is_some()
            || predecessors
                .insert(revision.replacement_recognition, revision.prior_recognition)
                .is_some()
        {
            return Err(invalid());
        }
    }
    let mut chains = CorrectionChains::default();
    if successors.is_empty() {
        return Ok(chains);
    }
    let index = state.accounting.as_ref().ok_or_else(invalid)?;
    if index.prefix.through != state.through
        || index.prefix.domain_digest != state.domain_digest
        || index.prefix.native_head.semantics_version != 1
    {
        return Err(invalid());
    }
    let roots = successors
        .keys()
        .filter(|id| !predecessors.contains_key(id));
    for &root in roots {
        let owner = original_adoption(state, root)?;
        let mut prior_id = root;
        let mut prior = retained_recognition(state, index, prior_id)?;
        if prior.leg_ids != owner.legs || chains.roots.insert(root, root).is_some() {
            return Err(invalid());
        }
        let mut posting_ids = BTreeSet::new();
        if prior
            .postings
            .iter()
            .any(|posting| !posting_ids.insert(posting.id))
        {
            return Err(invalid());
        }
        while let Some(&next_id) = successors.get(&prior_id) {
            let next = retained_recognition(state, index, next_id)?;
            if !index.reversed.contains(&prior_id)
                || chains.roots.insert(next_id, root).is_some()
                || prior.postings.len() != next.postings.len()
            {
                return Err(invalid());
            }
            if next.postings.iter().any(|p| !posting_ids.insert(p.id))
                || !crate::segments::same_economics(prior, next)
            {
                return Err(invalid());
            }
            prior_id = next_id;
            prior = next;
        }
        if index.reversed.contains(&prior_id) {
            return Err(invalid());
        }
    }
    // A disconnected cycle cannot be reached from a uniquely adopted root.
    if successors.keys().any(|id| !chains.roots.contains_key(id))
        || state.records.values().any(|record| {
            matches!(record,
            AuthoritativeRecordV1::ActivityAdoption(adoption)
                if adoption.recognition.is_some_and(|id| chains.is_replacement(id)))
        })
    {
        return Err(invalid());
    }
    Ok(chains)
}
