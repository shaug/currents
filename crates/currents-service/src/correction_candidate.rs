//! Dormant synthetic candidate for the additive correction preview boundary.
//! Not an OperationV1, transport type, approval token, or submit route.
use currents_domain::correction::CorrectionEngine;
use currents_storage::Writer;
use currents_types::*;
use sha2::{Digest as _, Sha256};

#[derive(Clone, serde::Serialize)]
pub struct SyntheticCorrectionCandidate {
    pub dataset: DatasetId,
    pub command: CommandId,
    pub expected_basis: AccountingPrefixV1,
    pub revision: RecordId,
    pub active_recognition: RecognitionId,
    pub replacement: RecognitionV1,
    pub reason: String,
    pub governing_policy: PolicyReferenceV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntheticCorrectionCandidatePreview {
    pub basis: AccountingPrefixV1,
    pub policy: PolicyReferenceV1,
    pub candidate_digest: Digest,
    pub plan: DecisionPlan,
}

/// Bind the proposed request to a currently audited prefix. No IDs, policy,
/// authorization, or durable decisions are minted by this preview.
pub async fn synthetic_preview_correction_candidate(
    writer: &Writer,
    candidate: &SyntheticCorrectionCandidate,
) -> Result<SyntheticCorrectionCandidatePreview, EngineError> {
    let stale = || crate::admission::error(ErrorClass::StaleBasis);
    let basis = writer.audited_prefix().await?;
    if basis != candidate.expected_basis {
        return Err(stale());
    }
    if candidate.dataset.0.is_nil() || candidate.command.0.is_nil() {
        return Err(EngineError::invalid());
    }
    let canonical = currents_protocol::canonical_bytes(&(
        "currents.synthetic.correction-candidate.v1",
        candidate,
    ))?;
    let candidate_digest = Digest(Sha256::digest(canonical).into());
    let state = crate::verified_prefix::rehydrate(writer, &basis).await?;
    let Some(AuthoritativeRecordV1::Dataset(dataset)) =
        state.records.get(&RecordId(candidate.dataset.0))
    else {
        return Err(EngineError::invalid());
    };
    if dataset.id != candidate.dataset {
        return Err(EngineError::invalid());
    }
    let plan = CorrectionEngine::correct_durable_at_policy(
        &state,
        candidate.revision,
        &CorrectionV1 {
            active_recognition: candidate.active_recognition,
            replacement: vec![candidate.replacement.clone()],
            remapping: vec![],
            reason: candidate.reason.clone(),
        },
        &candidate.governing_policy,
    )?;
    if writer.audited_prefix().await? != basis {
        return Err(stale());
    }
    Ok(SyntheticCorrectionCandidatePreview {
        basis,
        policy: candidate.governing_policy.clone(),
        candidate_digest,
        plan,
    })
}
