//! Proposals for corrections to an accepted recognition.
use currents_types::{
    AccountingIntent, AuthoritativeRecordV1, DecisionPlan, DomainState, EngineError,
    RecognitionCorrectionRemapV1, RecognitionCorrectionRevisionV1, RecordId,
};
use std::collections::BTreeSet;

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
        if revision_id.0.is_nil()
            || revision_id == RecordId(replacement.id.0)
            || state.records.contains_key(&revision_id)
        {
            return Err(EngineError::invalid());
        }
        let mut owners = state.records.values().filter_map(|record| match record {
            AuthoritativeRecordV1::ActivityAdoption(a) if a.recognition == Some(original_id) => {
                Some(a)
            }
            _ => None,
        });
        let owner = owners.next().ok_or_else(EngineError::version)?;
        if owners.next().is_some() || owner.opening.is_some() {
            return Err(EngineError::version());
        }
        if state.records.values().any(|record| match record {
            AuthoritativeRecordV1::Association(_)
            | AuthoritativeRecordV1::Allocation(_)
            | AuthoritativeRecordV1::ReconciliationClaim(_)
            | AuthoritativeRecordV1::Organization(_)
            | AuthoritativeRecordV1::OccurrenceMatch(_)
            | AuthoritativeRecordV1::ForecastRun(_) => true,
            AuthoritativeRecordV1::AssociationRevision(a) => {
                a.recognitions.contains(&original_id)
                    || a.retired_recognitions.contains(&original_id)
                    || a.superseded_adoptions.contains(&owner.id)
                    || a.legs.iter().any(|leg| owner.legs.contains(leg))
            }
            AuthoritativeRecordV1::Review(review) => {
                review.targets.contains(&RecordId(original_id.0))
                    || review.targets.contains(&owner.id)
                    || owner
                        .legs
                        .iter()
                        .any(|leg| review.targets.contains(&RecordId(leg.0)))
            }
            AuthoritativeRecordV1::AccountingSegment(s) => {
                s.opening == original_id
                    || s.included_activity.contains(&original_id)
                    || s.replacement_closure.contains(&original_id)
            }
            AuthoritativeRecordV1::RecognitionCorrectionRevision(r) => {
                r.prior_recognition == original_id || r.replacement_recognition == original_id
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
