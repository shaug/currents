//! Bounded synthetic pre-write proof rooted in the same writer's retained history.
use currents_storage::Writer;
use currents_types::*;

/// Load only an authenticated existing prefix; caller-provided projections
/// are never inputs. No recording, staging, or destination is performed.
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
        crate::replay::verify_retained_relationships(&source, &commit).await?;
        let mut projection = state.clone();
        currents_domain::ReplayEngine::apply(&mut projection, &commit)?;
        projection.accounting = Some(source.verify_retained_accounting(&state, &commit).await?);
        currents_domain::recognition::RecognitionEngine::audit(&projection)?;
        currents_domain::associations::IdentityEngine::audit(&projection)?;
        state = projection;
        next = stream.next().await?;
    }
    if state.through != wanted.through
        || state.domain_digest != wanted.domain_digest
        || state.accounting.as_ref().map(|index| &index.prefix) != Some(wanted)
    {
        return Err(invalid());
    }
    Ok(state)
}
