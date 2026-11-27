//! Exact-prefix trusted-host verification of segment replacements. No segment
//! route is dispatched.
use currents_domain::segments::{SegmentEngine, SegmentPreviewV1, SegmentReplacementV1};
use currents_storage::Writer;
use currents_types::*;

/// Verify a segment replacement against an independently verified prior and
/// current authority tip. The plan must be the ready preview derived again
/// from `request` at that prior, and the batch its fresh freeze. This is
/// read-only; the writer still rechecks the exact tip inside its transaction.
pub fn verify_segment_preflight(
    prior: &DomainState,
    trusted_tip: &AccountingPrefixV1,
    request: &SegmentReplacementV1,
    plan: &DecisionPlan,
    frozen: &AccountingBatchV1,
) -> Result<(), EngineError> {
    if prior.accounting.as_ref().map(|index| &index.prefix) != Some(trusted_tip)
        || &frozen.preconditions != trusted_tip
    {
        return Err(EngineError::invalid());
    }
    let SegmentPreviewV1::Ready(derived) = SegmentEngine::replace(prior, request)? else {
        return Err(EngineError::invalid());
    };
    if *derived != *plan {
        return Err(EngineError::invalid());
    }
    Writer::verify_segment_freeze(prior, plan, frozen)
}

/// Synthetic qualification of the trusted-host composition, never a dispatcher.
#[cfg(feature = "synthetic-test-fixtures")]
#[doc(hidden)]
pub async fn synthetic_commit_verified_segment(
    writer: &Writer,
    request: &SegmentReplacementV1,
    prepared: currents_storage::PreparedCommit,
) -> Result<currents_storage::CommitReceipt, currents_storage::CommitFailure> {
    use currents_storage::CommitFailure::DefinitelyRolledBack;
    let (trusted_tip, prior) = crate::verified_prefix::trusted_prior(writer, &prepared)
        .await
        .map_err(DefinitelyRolledBack)?;
    verify_segment_preflight(
        &prior,
        &trusted_tip,
        request,
        &prepared.decision_plan,
        &prepared.frozen_batch,
    )
    .map_err(DefinitelyRolledBack)?;
    writer.commit(prepared).await
}
