//! Exact-prefix trusted-host verification. No correction route is dispatched.
use currents_domain::correction::CorrectionEngine;
use currents_storage::Writer;
use currents_types::*;

/// Verify a standalone correction against an independently verified prior and
/// current authority tip. This is read-only; the writer must still recheck
/// exact preconditions inside the authority transaction before applying it.
pub fn verify_correction_preflight(
    prior: &DomainState,
    trusted_tip: &AccountingPrefixV1,
    plan: &DecisionPlan,
    frozen: &AccountingBatchV1,
) -> Result<(), EngineError> {
    if prior.accounting.as_ref().map(|index| &index.prefix) != Some(trusted_tip)
        || &frozen.preconditions != trusted_tip
    {
        return Err(EngineError::invalid());
    }
    let [
        AuthoritativeRecordV1::RecognitionCorrectionRevision(revision),
        AuthoritativeRecordV1::Recognition(_),
    ] = plan.records.as_slice()
    else {
        return Err(EngineError::invalid());
    };
    let AccountingIntent::Correction(request) = &plan.accounting else {
        return Err(EngineError::invalid());
    };
    if CorrectionEngine::correct_durable(prior, revision.id, request)? != *plan {
        return Err(EngineError::invalid());
    }
    Writer::verify_correction_freeze(prior, plan, frozen)
}

/// Synthetic qualification of the trusted-host composition, never a dispatcher.
#[cfg(feature = "synthetic-test-fixtures")]
#[doc(hidden)]
pub async fn synthetic_commit_verified_correction(
    writer: &Writer,
    prior: &DomainState,
    prepared: currents_storage::PreparedCommit,
) -> Result<currents_storage::CommitReceipt, currents_storage::CommitFailure> {
    use currents_storage::CommitFailure::DefinitelyRolledBack;
    let frozen_prefix = &prepared.frozen_batch.preconditions;
    let trusted_tip = if let Some(existing) = writer
        .command_result(prepared.result.command_id, prepared.result.digest)
        .await
        .map_err(DefinitelyRolledBack)?
    {
        // A settled retry refers to the original prior, not the new tip.
        // C04 still requires exact persisted records and manifest on retry.
        let Some(AuthoritativeRecordV1::FrozenAccountingManifest(manifest)) =
            existing.records.last()
        else {
            return Err(DefinitelyRolledBack(EngineError::invalid()));
        };
        if existing.previous_digest != frozen_prefix.domain_digest
            || existing.seq.0.checked_sub(1) != Some(frozen_prefix.through.0)
            || &manifest.preconditions != frozen_prefix
        {
            return Err(DefinitelyRolledBack(EngineError::invalid()));
        }
        manifest.preconditions.clone()
    } else {
        writer
            .audited_prefix()
            .await
            .map_err(DefinitelyRolledBack)?
    };
    verify_correction_preflight(
        prior,
        &trusted_tip,
        &prepared.decision_plan,
        &prepared.frozen_batch,
    )
    .map_err(DefinitelyRolledBack)?;
    writer.commit(prepared).await
}
