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
    if let Some(segment) = commit.records.iter().find_map(|record| match record {
        AuthoritativeRecordV1::AccountingSegment(segment) if segment.prior_segment.is_some() => {
            Some(segment)
        }
        _ => None,
    }) {
        return validate_segment_successor(prior, next, commit, segment);
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
            | AuthoritativeRecordV1::AccountingSegment(_)
            | AuthoritativeRecordV1::SegmentReplacementAuthority(_) => true,
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

/// Re-admit the complete standalone family claim against the old prefix.
/// The draft is not accepted via the normal initial-adoption path.
fn validate_segment_successor(
    prior: &DomainState,
    next: &DomainState,
    commit: &DomainCommitV1,
    successor: &AccountingSegmentV1,
) -> Result<(), EngineError> {
    use crate::segments::{
        SegmentEngine, SegmentPreviewRequestV1, SegmentPreviewV1, SegmentReemissionV1,
        segment_family,
    };
    let invalid = EngineError::invalid;
    let (root, None) = segment_family(prior, successor.component)?.ok_or_else(invalid)? else {
        return Err(invalid());
    };
    if successor.included_activity.is_empty() {
        return Err(invalid());
    }
    if successor.prior_segment != Some(root.id) {
        return Err(EngineError::version());
    }
    let (records, manifest) = commit
        .records
        .split_last()
        .ok_or_else(invalid)
        .map(|(last, rest)| (rest, last))?;
    let AuthoritativeRecordV1::FrozenAccountingManifest(manifest) = manifest else {
        return Err(invalid());
    };
    let prefix = &prior.accounting.as_ref().ok_or_else(invalid)?.prefix;
    if &manifest.preconditions != prefix {
        return Err(invalid());
    }
    let restore_exact = |id| {
        let mut candidates = records.iter().filter_map(|record| match record {
            AuthoritativeRecordV1::ActivityAdoption(a) if a.recognition == Some(id) => Some(a),
            _ => None,
        });
        let adoption = candidates.next().ok_or_else(invalid)?;
        if candidates.next().is_some() {
            return Err(invalid());
        }
        restore(next, adoption)
    };
    let mut owners = records.iter().filter_map(|record| match record {
        AuthoritativeRecordV1::SegmentReplacementAuthority(owner) => Some(owner),
        _ => None,
    });
    let owner = owners.next();
    if owners.next().is_some() || owner.is_none() && successor.replacement_closure != [root.opening]
    {
        return Err(invalid());
    }
    let mut reemissions = Vec::new();
    if let Some(owner) = owner {
        if owner.segment != successor.id || owner.prior_segment != root.id {
            return Err(invalid());
        }
        for pair in &owner.counterparts {
            let Some(AuthoritativeRecordV1::Recognition(replacement)) =
                next.records.get(&RecordId(pair.replacement.0))
            else {
                return Err(invalid());
            };
            reemissions.push(SegmentReemissionV1 {
                original: pair.original,
                replacement: pair.replacement,
                posting_ids: replacement.postings.iter().map(|p| p.id).collect(),
            });
        }
    }
    let proposal = SegmentPreviewRequestV1 {
        request: SegmentRequestV1 {
            component: successor.component,
            prior_segment: successor.prior_segment,
            opening: successor.opening,
            included_activity: successor.included_activity.clone(),
            replacement_closure: successor.replacement_closure.clone(),
            approved_accounts: successor.approved_accounts.clone(),
            residual_disposition: successor.residual_disposition.clone(),
        },
        opening: restore_exact(successor.opening)?,
        backfill: successor
            .included_activity
            .iter()
            .copied()
            .map(restore_exact)
            .collect::<Result<Vec<_>, _>>()?,
        reemissions,
    };
    let admitted = if let Some(owner) = owner {
        SegmentEngine::replace_owned(prior, &proposal, owner.id)?
    } else {
        SegmentEngine::replace(prior, &proposal)?
    };
    let SegmentPreviewV1::Ready(plan) = admitted else {
        return Err(invalid());
    };
    if plan.records != records || plan.invalidations != commit.command_result.effects.invalidations
    {
        return Err(invalid());
    }
    Ok(())
}
