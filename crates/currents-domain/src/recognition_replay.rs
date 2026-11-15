//! C08 replay admission uses the exact prior prefix, never a later repaired state.
use crate::{
    catalog::CatalogEngine,
    recognition::{RecognitionEngine, restore},
};
use currents_types::*;

pub(crate) fn validate_transition(
    prior: &DomainState,
    next: &DomainState,
    commit: &DomainCommitV1,
) -> Result<(), EngineError> {
    let mut emitted = Vec::new();
    for record in &commit.records {
        let plan = match record {
            AuthoritativeRecordV1::AccountCatalogRevision(revision) => {
                let Some(AuthoritativeRecordV1::AccountComponent(root)) =
                    next.records.get(&revision.component_id)
                else {
                    return Err(EngineError::invalid());
                };
                Some(CatalogEngine::set_complete(
                    prior,
                    &CatalogRequestV1 {
                        schema_version: revision.schema_version,
                        revision_id: revision.id,
                        prior_revision: revision.prior_revision,
                        catalog: revision.catalog.clone(),
                        ownership: root.ownership.clone(),
                        origin: revision.origin.clone(),
                    },
                )?)
            }
            AuthoritativeRecordV1::ActivityAdoption(adoption) => {
                Some(RecognitionEngine::adopt(prior, &restore(next, adoption)?)?)
            }
            _ => None,
        };
        if let Some(plan) = plan {
            for expected in plan.records {
                if !commit.records.contains(&expected) || emitted.contains(&expected) {
                    return Err(EngineError::invalid());
                }
                emitted.push(expected);
            }
        }
    }
    for record in &commit.records {
        let requires_owner = match record {
            AuthoritativeRecordV1::AccountCatalogRevision(_)
            | AuthoritativeRecordV1::ActivityAdoption(_)
            | AuthoritativeRecordV1::AccountingSegment(_) => true,
            AuthoritativeRecordV1::FinancialLeg(l) => !l.allocations.is_empty(),
            AuthoritativeRecordV1::Recognition(r) => {
                !r.evidence_allocations.is_empty() || !r.leg_ids.is_empty()
            }
            _ => false,
        };
        if requires_owner && !emitted.contains(record) {
            return Err(EngineError::version());
        }
    }
    Ok(())
}
