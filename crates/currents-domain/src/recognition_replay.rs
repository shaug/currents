//! C08 replay admission uses the exact prior prefix, never a later repaired state.
use crate::{
    catalog::CatalogEngine,
    investments::{InvestmentEngine, quantity::declaration_in},
    policy::{PolicyEngine, request_of},
    recognition::{RecognitionEngine, restore},
};
use currents_types::*;

pub(crate) fn validate_transition(
    prior: &DomainState,
    next: &DomainState,
    commit: &DomainCommitV1,
) -> Result<(), EngineError> {
    // A successor segment owns its whole commit: the plan derived again from
    // the prior prefix must match every record.
    if let Some(successor) = commit.records.iter().find_map(|record| match record {
        AuthoritativeRecordV1::AccountingSegment(s) if s.prior_segment.is_some() => Some(s),
        _ => None,
    }) {
        return crate::segments::readmit(prior, next, commit, successor);
    }
    // So does a correction that replaces a recognition's legs.
    if let Some(revision) = commit.records.iter().find_map(|record| match record {
        AuthoritativeRecordV1::RecognitionCorrectionRevision(r)
            if matches!(
                r.remap_disposition,
                RecognitionCorrectionRemapV1::ReplacedLegs { .. }
            ) =>
        {
            Some(r)
        }
        _ => None,
    }) {
        return crate::correction::readmit_replaced_legs(prior, commit, revision);
    }
    // So does a reconciliation claim, confirmation or dispute.
    if commit.records.iter().any(|record| {
        matches!(
            record,
            AuthoritativeRecordV1::ReconciliationClaim(_)
                | AuthoritativeRecordV1::ReconciliationDisposition(_)
        )
    }) {
        return crate::reconciliation::readmit(prior, commit);
    }
    // Only a segment replacement or a correction invalidates a claim, and
    // each derives its invalidations again. A later input change in a claim's
    // scope shows in its dependency digest instead.
    let corrects = commit.records.iter().any(|record| {
        matches!(
            record,
            AuthoritativeRecordV1::RecognitionCorrectionRevision(_)
        )
    });
    if !corrects
        && commit
            .command_result
            .effects
            .invalidations
            .iter()
            .any(|id| next.claim_status(*id).is_some())
    {
        return Err(EngineError::invalid());
    }
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
                // Only a version-2 creation request declares overlaps, and
                // no two totals in the combined projection list one part.
                let overlaps = if prior.records.contains_key(&revision.component_id) {
                    vec![]
                } else {
                    root.overlapping_components.clone()
                };
                for part in &overlaps {
                    let listed = next.records.values().filter(|r| {
                        matches!(r, AuthoritativeRecordV1::AccountComponent(t) if t.overlapping_components.contains(part))
                    });
                    if listed.count() != 1 {
                        return Err(EngineError::invalid());
                    }
                }
                Some(CatalogEngine::set_complete(
                    prior,
                    &CatalogRequestV1 {
                        schema_version: if overlaps.is_empty() {
                            revision.schema_version
                        } else {
                            2
                        },
                        revision_id: revision.id,
                        prior_revision: revision.prior_revision,
                        catalog: revision.catalog.clone(),
                        ownership: root.ownership.clone(),
                        origin: revision.origin.clone(),
                        overlaps,
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
                // A reinterpretation is derived again under the governing
                // policy and unresolved account its revision records.
                let plan = crate::correction::CorrectionEngine::readmit(
                    prior,
                    revision,
                    &CorrectionV1 {
                        active_recognition: revision.prior_recognition,
                        replacement: vec![replacement.clone()],
                        remapping: vec![],
                        reason: revision.reason.clone(),
                    },
                    &[],
                )?;
                // The commit records exactly the invalidations derived again.
                if commit.command_result.effects.invalidations != plan.invalidations {
                    return Err(EngineError::invalid());
                }
                Some(plan)
            }
            AuthoritativeRecordV1::ActivityAdoption(adoption) => {
                Some(RecognitionEngine::adopt(prior, &restore(next, adoption)?)?)
            }
            AuthoritativeRecordV1::PendingExpiry(expiry) => {
                Some(crate::expiry::PendingExpiryEngine::expire(prior, expiry)?)
            }
            // A new security's commit also declares its unit; a revision
            // declares nothing.
            AuthoritativeRecordV1::Security(security) if security.prior_revision.is_none() => Some(
                InvestmentEngine::declare_security(prior, &declaration_in(next, security)?)?,
            ),
            AuthoritativeRecordV1::Security(security) => {
                Some(InvestmentEngine::revise_security(prior, security)?)
            }
            AuthoritativeRecordV1::Position(position) => {
                Some(InvestmentEngine::declare_position(prior, position)?)
            }
            AuthoritativeRecordV1::Trade(trade) => Some(InvestmentEngine::recognize(
                prior,
                &crate::investments::replay::trade_request(next, trade)?,
            )?),
            // A revision is the plan the engine derives again from the prior
            // prefix. Two revisions of one family version share an ID, so the
            // projection must keep exactly this one.
            AuthoritativeRecordV1::DatasetPolicyRevision(revision) => {
                if next.records.get(&revision.id) != Some(record) {
                    return Err(EngineError::invalid());
                }
                Some(PolicyEngine::set(prior, &request_of(revision))?)
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
            | AuthoritativeRecordV1::AccountingSegment(_)
            | AuthoritativeRecordV1::SegmentReemission(_)
            | AuthoritativeRecordV1::Security(_)
            | AuthoritativeRecordV1::Position(_)
            | AuthoritativeRecordV1::Trade(_)
            | AuthoritativeRecordV1::DatasetPolicyRevision(_) => true,
            // Only a security declaration declares a security unit.
            AuthoritativeRecordV1::UnitCatalog(catalog) => catalog
                .units
                .iter()
                .any(|unit| unit.kind == UnitKind::Security),
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
