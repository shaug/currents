//! Verified projections rebuilt from the same writer's retained history,
//! without a replay destination.
use currents_storage::{HistoryReader, Writer};
use currents_types::*;

/// Bounded synthetic pre-write proof for corrections.
/// Load only an authenticated existing prefix; caller-provided projections
/// are never inputs. No recording, staging, or destination is performed.
#[cfg(feature = "synthetic-test-fixtures")]
pub(crate) async fn rehydrate(
    writer: &Writer,
    wanted: &AccountingPrefixV1,
) -> Result<DomainState, EngineError> {
    let invalid = EngineError::invalid;
    let current = writer.audited_prefix().await?;
    if wanted.through == CommitSeq(0)
        || wanted.through.0 > current.through.0
        || (wanted.through == current.through && wanted != &current)
    {
        return Err(invalid());
    }
    let source = writer.retained_history();
    let mut stream = source.stream_prefix(wanted.through).await?;
    let first = stream.next().await?.ok_or_else(invalid)?;
    // Synthetic writer qualification intentionally supports a history that
    // starts with its retained C05 genesis declaration, not guessed metadata.
    let Some(genesis) = first.records.iter().find_map(|record| match record {
        AuthoritativeRecordV1::FrozenAccountingManifest(m) => Some(&m.preconditions),
        _ => None,
    }) else {
        return Err(EngineError::version());
    };
    if genesis.through != CommitSeq(0)
        || genesis.domain_digest != Digest::default()
        || genesis.native_head.semantics_version != 1
    {
        return Err(invalid());
    }
    let mut state = DomainState {
        accounting: Some(AccountingIndexV1::empty(genesis.native_head.clone())),
        ..DomainState::default()
    };
    let mut next = Some(first);
    while let Some(commit) = next {
        state = apply_retained(&source, &state, &commit).await?;
        next = stream.next().await?;
    }
    exact(state, wanted)
}

/// The prefix a synthetic trusted-host commit is checked against. A retry of a
/// command that already committed refers to its original prior, not the new
/// tip; C04 still requires the exact persisted records and manifest.
#[cfg(feature = "synthetic-test-fixtures")]
pub(crate) async fn settled_or_audited_tip(
    writer: &Writer,
    prepared: &currents_storage::PreparedCommit,
) -> Result<AccountingPrefixV1, EngineError> {
    let invalid = EngineError::invalid;
    let Some(existing) = writer
        .command_result(prepared.result.command_id, prepared.result.digest)
        .await?
    else {
        return writer.audited_prefix().await;
    };
    let frozen_prefix = &prepared.frozen_batch.preconditions;
    let Some(AuthoritativeRecordV1::FrozenAccountingManifest(manifest)) = existing.records.last()
    else {
        return Err(invalid());
    };
    let mut submitted_result = prepared.result.clone();
    submitted_result.commit = Some(existing.seq);
    if existing.previous_digest != frozen_prefix.domain_digest
        || existing.seq.0.checked_sub(1) != Some(frozen_prefix.through.0)
        || &manifest.preconditions != frozen_prefix
        || submitted_result != existing.command_result
    {
        return Err(invalid());
    }
    Ok(manifest.preconditions.clone())
}

/// Move a verified projection forward over the commits appended after it, with
/// the same per-commit checks as `rehydrate`. The service uses this after its
/// own commit so the next command does not need a full replay.
pub(crate) async fn advance(
    writer: &Writer,
    mut state: DomainState,
    wanted: &AccountingPrefixV1,
) -> Result<DomainState, EngineError> {
    if wanted.through.0 <= state.through.0 {
        return Err(EngineError::invalid());
    }
    let source = writer.retained_history();
    // The projection's digest is its last commit's digest, which links the next.
    let mut stream = source
        .stream_after(state.through, state.domain_digest, wanted.through)
        .await?;
    while let Some(commit) = stream.next().await? {
        state = apply_retained(&source, &state, &commit).await?;
    }
    exact(state, wanted)
}

/// One commit of verified replay without a destination: the retained content,
/// domain records and accounting index are checked exactly as recovery does.
async fn apply_retained(
    source: &HistoryReader,
    state: &DomainState,
    commit: &DomainCommitV1,
) -> Result<DomainState, EngineError> {
    crate::replay::verify_retained_relationships(source, commit).await?;
    let mut projection = state.clone();
    currents_domain::ReplayEngine::apply(&mut projection, commit)?;
    projection.accounting = Some(source.verify_retained_accounting(state, commit).await?);
    currents_domain::recognition::RecognitionEngine::audit(&projection)?;
    currents_domain::associations::IdentityEngine::audit(&projection)?;
    Ok(projection)
}

fn exact(state: DomainState, wanted: &AccountingPrefixV1) -> Result<DomainState, EngineError> {
    if state.through != wanted.through
        || state.domain_digest != wanted.domain_digest
        || state.accounting.as_ref().map(|index| &index.prefix) != Some(wanted)
    {
        return Err(EngineError::invalid());
    }
    Ok(state)
}
