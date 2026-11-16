//! Explicit owner expiry of pending evidence. Never a clock, deletion or refund.
use crate::recognition::RecognitionEngine;
use currents_types::*;

pub struct PendingExpiryEngine;

pub(crate) fn validate(state: &DomainState, expiry: &PendingExpiryV1) -> Result<(), EngineError> {
    if expiry.schema_version != 1 {
        return Err(EngineError::version());
    }
    if expiry.id.0.is_nil()
        || expiry.id == expiry.pending_adoption
        || expiry.interpretation.authority != AuthorityKind::Owner
        || expiry.interpretation.kind != DecisionKind::Interpretation
        || expiry.interpretation.reason.trim().is_empty()
        || expiry.interpretation.reason.len() > 4096
        || expiry.interpretation.evidence.len() > 16384
    {
        return Err(EngineError::invalid());
    }
    let Some(AuthoritativeRecordV1::ActivityAdoption(pending)) =
        state.records.get(&expiry.pending_adoption)
    else {
        return Err(EngineError::invalid());
    };
    if pending.id != expiry.pending_adoption
        || pending.status != ActivityStatusV1::Pending
        || pending.recognition.is_some()
    {
        return Err(EngineError::invalid());
    }
    let Some(policy) = state
        .dataset_policies
        .get(&(expiry.governing_policy.id, expiry.governing_policy.version))
    else {
        return Err(EngineError::invalid());
    };
    if policy.id != expiry.governing_policy.id
        || policy.version == 0
        || policy.version != expiry.governing_policy.version
    {
        return Err(EngineError::invalid());
    }
    for id in &expiry.interpretation.evidence {
        if !matches!(
            state.records.get(id),
            Some(
                AuthoritativeRecordV1::ObservationVersion(_)
                    | AuthoritativeRecordV1::ImportReceipt(_)
                    | AuthoritativeRecordV1::AddressedParseProduct(_)
            )
        ) {
            return Err(EngineError::invalid());
        }
    }
    Ok(())
}
impl PendingExpiryEngine {
    /// Owner chooses expiry explicitly. An incomplete feed or tombstone cannot
    /// call this without a new authorized interpretation of this exact target.
    pub fn expire(
        state: &DomainState,
        expiry: &PendingExpiryV1,
    ) -> Result<DecisionPlan, EngineError> {
        validate(state, expiry)?;
        if state.records.contains_key(&expiry.id)
            || !RecognitionEngine::pending(state)?
                .iter()
                .any(|p| p.id == expiry.pending_adoption)
        {
            return Err(EngineError::invalid());
        }
        Ok(DecisionPlan {
            records: vec![AuthoritativeRecordV1::PendingExpiry(expiry.clone())],
            invalidations: vec![expiry.pending_adoption],
            accounting: AccountingIntent::NoEffect(NoEffectV1 {
                reason: "Explicit pending evidence expiry; no financial reversal".into(),
            }),
        })
    }
}
