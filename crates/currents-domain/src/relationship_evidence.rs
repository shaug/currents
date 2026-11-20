//! Retained synthetic source descriptions, with no financial or revision authority.
use currents_types::*;
use sha2::{Digest as _, Sha256};
pub struct ValidatedRelationshipCaptureV1(RelationshipCaptureBatchV1);
impl ValidatedRelationshipCaptureV1 {
    pub fn new(batch: RelationshipCaptureBatchV1) -> Result<Self, EngineError> {
        let c = &batch.capture;
        let r = &batch.receipt;
        validate_metadata(c, r)?;
        if batch.contents.len() != 1 {
            return Err(EngineError::invalid());
        }
        let content = &batch.contents[0];
        if content.bytes.len() > 1_048_576
            || content.digest != c.raw.digest
            || content.bytes.len() as u64 != c.raw.byte_count.0
            || Digest(Sha256::digest(&content.bytes).into()) != content.digest
            || !crate::evidence::safe(&content.bytes)
        {
            return Err(EngineError::invalid());
        }
        let decoded: Vec<RelationshipSourceRowV1> =
            serde_json::from_slice(&content.bytes).map_err(|_| EngineError::invalid())?;
        if decoded != c.rows {
            return Err(EngineError::invalid());
        }
        Ok(Self(batch))
    }
    pub fn contents(&self) -> &[EvidenceContentV1] {
        &self.0.contents
    }
}
pub struct RelationshipEvidenceEngine;
impl RelationshipEvidenceEngine {
    pub fn publish(
        state: &DomainState,
        batch: ValidatedRelationshipCaptureV1,
    ) -> Result<DecisionPlan, EngineError> {
        let batch = batch.0;
        if state.records.contains_key(&batch.receipt.id)
            || state.records.contains_key(&batch.capture.id)
        {
            return Err(EngineError::invalid());
        }
        Ok(DecisionPlan {
            records: vec![
                AuthoritativeRecordV1::ImportReceipt(batch.receipt),
                AuthoritativeRecordV1::RelationshipCapture(batch.capture),
            ],
            invalidations: vec![],
            accounting: AccountingIntent::NoEffect(NoEffectV1 {
                reason: "Retained descriptive relationship source only".into(),
            }),
        })
    }
}

pub(crate) fn validate_metadata(
    c: &RelationshipCaptureV1,
    r: &ImportReceiptV1,
) -> Result<(), EngineError> {
    if c.schema_version != 1 {
        return Err(EngineError::version());
    }
    RelationshipSourceRowV1::validate_all(&c.rows)?;
    if c.id.0.is_nil()
        || c.receipt_id.0.is_nil()
        || c.id == c.receipt_id
        || c.receipt_id != r.id
        || c.source != r.source
        || c.source.source != "synthetic"
        || c.source.device_id.is_some()
        || c.source.connection_id.0.is_nil()
        || r.adapter_version != "synthetic-relationships/1"
        || r.parser_version != "1"
        || r.requested_coverage != r.reported_coverage
        || r.requested_coverage.len() > 16
        || r.result != ImportResult::Complete
        || !r.complete
        || r.truncated
        || r.cursor != SourceValue::Absent
        || r.contents != vec![c.raw.clone()]
        || c.raw.media_type != RELATIONSHIP_SOURCE_MEDIA
        || c.raw.byte_count.0 > 1_048_576
    {
        return Err(EngineError::invalid());
    }
    Ok(())
}
