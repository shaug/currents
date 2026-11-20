//! Bounded dependency handling shared by correction and segment replay admission.
use currents_types::{
    AuthoritativeRecordV1, DateBasis, DomainState, EngineError, LegId, OrganizationKind,
    RecognitionV1, RecordId, TemporalValue,
};
use std::collections::BTreeSet;

/// Known scope-only Activity predicates receive a deterministic stale marker.
/// Display notes may retain an exact stable leg but cannot follow retired legs.
pub(crate) fn invalidations(
    state: &DomainState,
    changed: &[&RecognitionV1],
    obsolete_legs: &[LegId],
    retired_segments: &[RecordId],
) -> Result<BTreeSet<RecordId>, EngineError> {
    let mut dependent_claims = BTreeSet::new();
    for record in state.records.values() {
        match record {
            AuthoritativeRecordV1::Organization(note) => {
                if note.kind != OrganizationKind::Note
                    || !note.allocations.is_empty()
                    || note.targets.is_empty()
                    || note.targets.iter().any(|target| {
                        obsolete_legs.iter().any(|leg| RecordId(leg.0) == *target)
                            || !matches!(
                                state.records.get(target),
                                Some(AuthoritativeRecordV1::FinancialLeg(_))
                            )
                    })
                {
                    return Err(EngineError::version());
                }
                // Exact identity compensation preserves stable economic leg IDs.
            }
            AuthoritativeRecordV1::ReconciliationClaim(claim) => {
                let (Some(start), Some(end)) = (claim.cutoffs.start, claim.cutoffs.end) else {
                    return Err(EngineError::version());
                };
                if claim.comparison_type != "Activity"
                    || !claim.matched_allocations.is_empty()
                    || !claim.evidence_dependencies.is_empty()
                    || !claim.decision_dependencies.is_empty()
                    || !claim.exclusions.is_empty()
                    || claim.scope.is_empty()
                    || start > end
                {
                    return Err(EngineError::version());
                }
                for posting in changed.iter().flat_map(|recognition| &recognition.postings) {
                    if !claim.scope.contains(&posting.account_id) {
                        continue;
                    }
                    let date = match claim.cutoffs.basis {
                        DateBasis::Effective => &posting.effective,
                        DateBasis::Posting => &posting.posting,
                    };
                    let TemporalValue::Date { civil, .. } = date else {
                        return Err(EngineError::version());
                    };
                    if start <= *civil && *civil <= end {
                        dependent_claims.insert(claim.id);
                    }
                }
            }
            AuthoritativeRecordV1::ForecastRun(run) => {
                let anchor = state
                    .verified_forecast_anchors
                    .get(&run.id)
                    .ok_or_else(EngineError::version)?;
                if retired_segments.contains(&anchor.segment) {
                    dependent_claims.insert(run.id);
                } else {
                    for posting in changed
                        .iter()
                        .flat_map(|recognition| &recognition.postings)
                        .filter(|posting| posting.account_id == anchor.account)
                    {
                        let TemporalValue::Date { civil, .. } = &posting.effective else {
                            return Err(EngineError::version());
                        };
                        if *civil <= anchor.as_of.0 {
                            dependent_claims.insert(run.id);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(dependent_claims)
}
