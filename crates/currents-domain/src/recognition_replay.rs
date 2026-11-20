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
                // Independent plans share the prior prefix. Their combined
                // projection must still have one chain and unambiguous aliases.
                CatalogEngine::current(next, revision.component_id)?;
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
            AuthoritativeRecordV1::RecognitionCorrectionRevision(revision) => {
                let Some(AuthoritativeRecordV1::Recognition(replacement)) = next
                    .records
                    .get(&RecordId(revision.replacement_recognition.0))
                else {
                    return Err(EngineError::invalid());
                };
                // The bounded slice is one owner and replacement, in a fixed order.
                let [
                    owner,
                    adopted,
                    AuthoritativeRecordV1::FrozenAccountingManifest(_),
                ] = commit.records.as_slice()
                else {
                    return Err(EngineError::invalid());
                };
                if owner != record
                    || adopted != &AuthoritativeRecordV1::Recognition(replacement.clone())
                {
                    return Err(EngineError::invalid());
                }
                Some(crate::correction::CorrectionEngine::correct_durable(
                    prior,
                    revision.id,
                    &CorrectionV1 {
                        active_recognition: revision.prior_recognition,
                        replacement: vec![replacement.clone()],
                        remapping: vec![],
                        reason: revision.reason.clone(),
                    },
                )?)
            }
            AuthoritativeRecordV1::ActivityAdoption(adoption) => {
                Some(RecognitionEngine::adopt(prior, &restore(next, adoption)?)?)
            }
            AuthoritativeRecordV1::PendingExpiry(expiry) => {
                Some(crate::expiry::PendingExpiryEngine::expire(prior, expiry)?)
            }
            AuthoritativeRecordV1::RelationshipParseRevision(p) => {
                let Some(AuthoritativeRecordV1::ImportReceipt(r)) = next.records.get(&p.receipt_id)
                else {
                    return Err(EngineError::invalid());
                };
                Some(crate::relationship_parse::plan(prior, p, r)?)
            }
            AuthoritativeRecordV1::RelationshipCapture(capture) => {
                let Some(AuthoritativeRecordV1::ImportReceipt(receipt)) =
                    next.records.get(&capture.receipt_id)
                else {
                    return Err(EngineError::invalid());
                };
                crate::relationship_evidence::validate_metadata(capture, receipt)?;
                if prior.records.contains_key(&capture.id)
                    || prior.records.contains_key(&receipt.id)
                {
                    return Err(EngineError::invalid());
                }
                Some(DecisionPlan {
                    records: vec![
                        AuthoritativeRecordV1::ImportReceipt(receipt.clone()),
                        record.clone(),
                    ],
                    invalidations: vec![],
                    accounting: AccountingIntent::NoEffect(NoEffectV1 {
                        reason: "Retained descriptive relationship source only".into(),
                    }),
                })
            }
            AuthoritativeRecordV1::AssociationRevision(revision)
                if revision.kind == RelationshipKind::Duplicate =>
            {
                Some(crate::associations::IdentityEngine::associate(
                    prior,
                    &CompleteAssociationRequestV1 {
                        revision: revision.clone(),
                        events: vec![],
                        legs: vec![],
                        recognitions: vec![],
                    },
                )?)
            }
            AuthoritativeRecordV1::AssociationRevision(revision) => {
                let mut legs = Vec::new();
                let mut events = Vec::new();
                for id in &revision.legs {
                    let Some(AuthoritativeRecordV1::FinancialLeg(leg)) =
                        next.records.get(&RecordId(id.0))
                    else {
                        return Err(EngineError::invalid());
                    };
                    let Some(AuthoritativeRecordV1::FinancialEvent(event)) =
                        next.records.get(&leg.event_id)
                    else {
                        return Err(EngineError::invalid());
                    };
                    if !events.contains(event) {
                        events.push(event.clone());
                    }
                    legs.push(leg.clone());
                }
                let mut recognitions = Vec::new();
                for id in &revision.recognitions {
                    let Some(AuthoritativeRecordV1::Recognition(recognition)) =
                        next.records.get(&RecordId(id.0))
                    else {
                        return Err(EngineError::invalid());
                    };
                    recognitions.push(recognition.clone());
                }
                Some(crate::associations::IdentityEngine::associate(
                    prior,
                    &CompleteAssociationRequestV1 {
                        revision: revision.clone(),
                        events,
                        legs,
                        recognitions,
                    },
                )?)
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
            | AuthoritativeRecordV1::RecognitionCorrectionRevision(_)
            | AuthoritativeRecordV1::RelationshipCapture(_)
            | AuthoritativeRecordV1::RelationshipParseRevision(_)
            | AuthoritativeRecordV1::AssociationRevision(_)
            | AuthoritativeRecordV1::PendingExpiry(_)
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
