//! Bounded dependency handling shared by correction and segment replay admission.
use currents_types::{
    AuthoritativeRecordV1, DateBasis, DomainState, EngineError, LegId, OrganizationKind,
    RecognitionV1, RecordId, TemporalValue,
};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganizationDispositionItem {
    pub organization_revision: RecordId,
    pub organization_version: u32,
    pub prior_leg: LegId,
    pub eligible_candidate_legs: Vec<LegId>,
}

pub(crate) struct DependentChanges {
    pub invalidations: BTreeSet<RecordId>,
    pub organization_questions: Vec<OrganizationDispositionItem>,
}

/// Known scope-only Activity predicates receive a deterministic stale marker.
/// Display notes may retain an exact stable leg. Moving a retired leg note
/// requires a separate explicit owner decision before the plan can escape.
pub(crate) fn invalidations(
    state: &DomainState,
    changed: &[&RecognitionV1],
    obsolete_legs: &[LegId],
    retired_segments: &[RecordId],
) -> Result<DependentChanges, EngineError> {
    let mut dependent_claims = BTreeSet::new();
    let mut organization_questions = Vec::new();
    for record in state.records.values() {
        match record {
            AuthoritativeRecordV1::Organization(note) => {
                if note.kind != OrganizationKind::Note
                    || !note.allocations.is_empty()
                    || note.targets.is_empty()
                    || note.targets.iter().any(|target| {
                        !matches!(
                            state.records.get(target),
                            Some(AuthoritativeRecordV1::FinancialLeg(_))
                        )
                    })
                {
                    return Err(EngineError::version());
                }
                let mut retired = obsolete_legs
                    .iter()
                    .copied()
                    .filter(|leg| note.targets.contains(&RecordId(leg.0)));
                if let Some(prior_leg) = retired.next() {
                    if note.targets.len() != 1 || retired.next().is_some()
                        || note.version != 1 || note.parent.is_some() || note.supersedes.is_some()
                        || state.records.values().any(|record| matches!(
                            record,
                            AuthoritativeRecordV1::Organization(other)
                                if other.supersedes == Some(note.id) || other.parent == Some(note.id)
                        ))
                    {
                        return Err(EngineError::version());
                    }
                    organization_questions.push(OrganizationDispositionItem {
                        organization_revision: note.id,
                        organization_version: note.version,
                        prior_leg,
                        eligible_candidate_legs: Vec::new(),
                    });
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
    Ok(DependentChanges {
        invalidations: dependent_claims,
        organization_questions,
    })
}
