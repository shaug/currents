//! Trusted-host synthetic qualification only; no writable runtime route.
use currents_domain::segments::{SegmentEngine, SegmentPreviewRequestV1, SegmentPreviewV1};
use currents_storage::{CommitFailure, CommitReceipt, PreparedCommit, Writer};
use currents_types::*;

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
        || !proposal.reemissions.is_empty()
        || proposal.request.replacement_closure.len() != 1
        || proposal.request.included_activity.len() != 1
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
    let SegmentPreviewV1::Ready(readmitted) =
        SegmentEngine::replace(&prior, proposal).map_err(DefinitelyRolledBack)?
    else {
        return Err(invalid());
    };
    if *readmitted != prepared.decision_plan {
        return Err(invalid());
    }
    let emitted = readmitted
        .records
        .iter()
        .map(|record| match record {
            AuthoritativeRecordV1::FinancialEvent(e) => Ok(e.id),
            AuthoritativeRecordV1::FinancialLeg(l) => Ok(RecordId(l.id.0)),
            AuthoritativeRecordV1::ActivityAdoption(a) => Ok(a.id),
            AuthoritativeRecordV1::Recognition(r) => Ok(RecordId(r.id.0)),
            AuthoritativeRecordV1::AccountingSegment(s) => Ok(s.id),
            _ => Err(invalid()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let effects = &prepared.result.effects;
    if effects.records != emitted
        || effects.invalidations != readmitted.invalidations
        || !effects.accounts.is_empty()
        || !effects.amounts.is_empty()
        || !prepared.result.health.is_empty()
    {
        return Err(invalid());
    }
    Writer::verify_standalone_segment_freeze(
        &prior,
        &prepared.decision_plan,
        &prepared.frozen_batch,
    )
    .map_err(DefinitelyRolledBack)?;
    // This check above binds semantics. C04 separately verifies the actual
    // authority tip inside the write transaction, including the native head.
    writer.commit(prepared).await
}
