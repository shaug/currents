//! Trusted-host synthetic qualification only; no writable runtime route.
use currents_domain::segments::{SegmentEngine, SegmentPreviewRequestV1, SegmentPreviewV1};
use currents_storage::{CommitFailure, CommitReceipt, PreparedCommit, Writer};
use currents_types::*;

/// Map a complete C03 proposal into the owned planner against the
/// authenticated prefix of this Writer. No authority is generated, no plan is
/// committed, and typed blocked outcomes remain non-writable.
#[doc(hidden)]
pub async fn synthetic_preview_owned_segment(
    writer: &Writer,
    body: &currents_protocol::CommandBodyV1,
) -> Result<SegmentPreviewV1, EngineError> {
    authenticated_owned_segment(writer, body)
        .await
        .map(|(_, outcome)| outcome)
}

async fn authenticated_owned_segment(
    writer: &Writer,
    body: &currents_protocol::CommandBodyV1,
) -> Result<(DomainState, SegmentPreviewV1), EngineError> {
    use currents_domain::segments::SegmentReemissionV1;
    use currents_protocol::OperationV1;

    let OperationV1::ReplaceAccountingSegmentComplete(request) = &body.operation else {
        return Err(EngineError::version());
    };
    if body.dataset_id.0.is_nil() || body.command_id.0.is_nil() {
        return Err(EngineError::invalid());
    }
    let stale = || crate::admission::error(ErrorClass::StaleBasis);
    let basis = writer.audited_prefix().await?;
    if body.expected_head != basis.through || request.expected_basis != basis {
        return Err(stale());
    }
    let state = crate::verified_prefix::rehydrate(writer, &basis).await?;
    if !matches!(
        state.records.get(&RecordId(body.dataset_id.0)),
        Some(AuthoritativeRecordV1::Dataset(dataset)) if dataset.id == body.dataset_id
    ) {
        return Err(EngineError::invalid());
    }
    let scope = &request.segment;
    let outcome = SegmentEngine::replace_owned(
        &state,
        &SegmentPreviewRequestV1 {
            request: SegmentRequestV1 {
                component: scope.component,
                prior_segment: Some(scope.prior_segment),
                opening: scope.opening,
                included_activity: scope.included_activity.clone(),
                replacement_closure: scope.replacement_closure.clone(),
                approved_accounts: scope.approved_accounts.clone(),
                residual_disposition: SourceValue::Absent,
            },
            opening: request.opening.clone(),
            backfill: request.backfill.clone(),
            reemissions: request
                .reemissions
                .iter()
                .map(|r| SegmentReemissionV1 {
                    original: r.original,
                    replacement: r.replacement,
                    posting_ids: r.posting_ids.clone(),
                })
                .collect(),
        },
        request.authority_id,
    )?;
    if writer.audited_prefix().await? != basis {
        return Err(stale());
    }
    Ok((state, outcome))
}

pub(crate) async fn verify_approved_owned_segment(
    writer: &Writer,
    body: &currents_protocol::CommandBodyV1,
    approved_digest: Digest,
    supplied: &crate::CommandPlan,
) -> Result<(), EngineError> {
    use currents_protocol::{OperationV1, command_digest};
    let invalid = EngineError::invalid;
    let OperationV1::ReplaceAccountingSegmentComplete(request) = &body.operation else {
        return Err(EngineError::version());
    };
    if command_digest(body)? != approved_digest {
        return Err(invalid());
    }
    let (prior, SegmentPreviewV1::Ready(expected)) =
        authenticated_owned_segment(writer, body).await?
    else {
        return Err(invalid());
    };
    let Some(AuthoritativeRecordV1::SegmentReplacementAuthority(owner)) = expected.records.last()
    else {
        return Err(invalid());
    };
    if expected.as_ref() != &supplied.decision_plan
        || supplied.import.is_some()
        || supplied.effects.records != segment_effect_records(&expected)?
        || supplied.effects.invalidations != expected.invalidations
        || supplied.effects.accounts != owner.affected_accounts
        || !supplied.effects.amounts.is_empty()
        || supplied.approval_scope.accounts != owner.affected_accounts
        || supplied.approval_scope.operation != DecisionKind::Correction
        || supplied.policy != request.opening.governing_policy
    {
        return Err(invalid());
    }
    Writer::verify_segment_freeze(&prior, &expected, &supplied.frozen_batch)
}

fn segment_effect_records(plan: &DecisionPlan) -> Result<Vec<RecordId>, EngineError> {
    plan.records
        .iter()
        .map(|record| match record {
            AuthoritativeRecordV1::FinancialEvent(e) => Ok(e.id),
            AuthoritativeRecordV1::FinancialLeg(l) => Ok(RecordId(l.id.0)),
            AuthoritativeRecordV1::ActivityAdoption(a) => Ok(a.id),
            AuthoritativeRecordV1::Recognition(r) => Ok(RecordId(r.id.0)),
            AuthoritativeRecordV1::AccountingSegment(s) => Ok(s.id),
            AuthoritativeRecordV1::SegmentReplacementAuthority(owner) => Ok(owner.id),
            _ => Err(EngineError::invalid()),
        })
        .collect()
}

#[doc(hidden)]
pub async fn synthetic_commit_verified_segment(
    writer: &Writer,
    proposal: &SegmentPreviewRequestV1,
    prepared: PreparedCommit,
) -> Result<CommitReceipt, CommitFailure> {
    use CommitFailure::DefinitelyRolledBack;
    let invalid = || DefinitelyRolledBack(EngineError::invalid());
    let trusted_tip = writer
        .audited_prefix()
        .await
        .map_err(DefinitelyRolledBack)?;
    if prepared.frozen_batch.preconditions != trusted_tip
        || prepared.prior_head != trusted_tip.through
        || proposal.request.included_activity.is_empty()
        || !prepared.evidence.is_empty()
        || !prepared.decisions.is_empty()
        || prepared.result.state != CommandState::Committed
        || prepared.result.commit.is_some()
    {
        return Err(invalid());
    }
    let prior = crate::verified_prefix::rehydrate(writer, &trusted_tip)
        .await
        .map_err(DefinitelyRolledBack)?;
    let mut owners = prepared
        .decision_plan
        .records
        .iter()
        .filter_map(|record| match record {
            AuthoritativeRecordV1::SegmentReplacementAuthority(owner) => Some(owner),
            _ => None,
        });
    let owner = owners.next();
    if owners.next().is_some() {
        return Err(invalid());
    }
    let admitted = if let Some(owner) = owner {
        SegmentEngine::replace_owned(&prior, proposal, owner.id)
    } else if proposal.reemissions.is_empty() && proposal.request.replacement_closure.len() == 1 {
        SegmentEngine::replace(&prior, proposal)
    } else {
        return Err(invalid());
    };
    let SegmentPreviewV1::Ready(readmitted) = admitted.map_err(DefinitelyRolledBack)? else {
        return Err(invalid());
    };
    if *readmitted != prepared.decision_plan {
        return Err(invalid());
    }
    let emitted = segment_effect_records(&readmitted).map_err(DefinitelyRolledBack)?;
    let effects = &prepared.result.effects;
    if effects.records != emitted
        || effects.invalidations != readmitted.invalidations
        || effects.accounts.as_slice()
            != owner.map_or(&[][..], |owner| owner.affected_accounts.as_slice())
        || !effects.amounts.is_empty()
        || !prepared.result.health.is_empty()
    {
        return Err(invalid());
    }
    Writer::verify_segment_freeze(&prior, &prepared.decision_plan, &prepared.frozen_batch)
        .map_err(DefinitelyRolledBack)?;
    // This check above binds semantics. C04 separately verifies the actual
    // authority tip inside the write transaction, including the native head.
    writer.commit(prepared).await
}
