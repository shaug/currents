//! Trusted, bounded import content. Raw bytes never enter command/preview DTOs.
use currents_domain::evidence::{EvidenceEngine, ValidatedEvidenceBatchV1};
use currents_protocol::{CanonicalLimits, PublishImportV1, canonical_bytes_with_limits};
use currents_types::*;
use sha2::{Digest as _, Sha256};

#[derive(Clone)]
enum RetainedInput {
    Ordinary(CaptureBatchV1),
    Relationship(RelationshipCaptureBatchV1),
    RelationshipParse(RelationshipParseBatchV1),
}
#[derive(Clone, serde::Serialize)]
pub struct ImportEvidence {
    identity: Digest,
    descriptors: Vec<(Digest, CommitSeq)>,
    #[serde(skip)]
    capture: RetainedInput,
    #[serde(skip)]
    metadata_bytes: usize,
}
impl ImportEvidence {
    pub fn new(capture: CaptureBatchV1) -> Result<Self, EngineError> {
        if capture.contents.len() > 10_002
            || capture
                .contents
                .iter()
                .fold(0usize, |n, c| n.saturating_add(c.bytes.len()))
                > crate::admission::MAX_RETAINED_BYTES
        {
            return Err(EngineError::invalid());
        }
        ValidatedEvidenceBatchV1::new(capture.clone())?;
        let descriptors: Vec<_> = capture
            .contents
            .iter()
            .map(|c| (c.digest, CommitSeq(c.bytes.len() as u64)))
            .collect();
        // Metadata references bind profile, source, capture time and raw digest;
        // the entire addressed parse result/receipt identity is bound separately.
        let bytes = canonical_bytes_with_limits(
            &(
                &capture.unit,
                &capture.receipt,
                &capture.observations,
                &capture.products,
                &descriptors,
            ),
            CanonicalLimits {
                max_bytes: 64 * 1_048_576,
                max_collection_items: 65_536,
            },
        )?;
        Ok(Self {
            identity: Digest(Sha256::digest(&bytes).into()),
            descriptors,
            capture: RetainedInput::Ordinary(capture),
            metadata_bytes: bytes.len(),
        })
    }
    /// Explicit synthetic relationship profile; retains the established staged
    /// evidence path without placing source bytes in command or preview DTOs.
    pub fn new_relationship(capture: RelationshipCaptureBatchV1) -> Result<Self, EngineError> {
        currents_domain::relationship_evidence::ValidatedRelationshipCaptureV1::new(
            capture.clone(),
        )?;
        let descriptors: Vec<_> = capture
            .contents
            .iter()
            .map(|c| (c.digest, CommitSeq(c.bytes.len() as u64)))
            .collect();
        let bytes = canonical_bytes_with_limits(
            &(
                "relationship-capture/1",
                &capture.receipt,
                &capture.capture,
                &descriptors,
            ),
            CanonicalLimits {
                max_bytes: 64 * 1_048_576,
                max_collection_items: 65_536,
            },
        )?;
        if capture
            .contents
            .iter()
            .fold(bytes.len(), |n, c| n.saturating_add(c.bytes.len()))
            > crate::admission::MAX_RETAINED_BYTES
        {
            return Err(EngineError::invalid());
        }
        Ok(Self {
            identity: Digest(Sha256::digest(&bytes).into()),
            descriptors,
            capture: RetainedInput::Relationship(capture),
            metadata_bytes: bytes.len(),
        })
    }
    /// Explicit correction profile, independently retained from immutable raw evidence.
    pub fn new_relationship_parse(batch: RelationshipParseBatchV1) -> Result<Self, EngineError> {
        currents_domain::relationship_parse::ValidatedRelationshipParseV1::new(batch.clone())?;
        let descriptors: Vec<_> = batch
            .contents
            .iter()
            .map(|c| (c.digest, CommitSeq(c.bytes.len() as u64)))
            .collect();
        let bytes = canonical_bytes_with_limits(
            &(
                "relationship-parse/1",
                &batch.receipt,
                &batch.revision,
                &descriptors,
            ),
            CanonicalLimits {
                max_bytes: 64 * 1_048_576,
                max_collection_items: 65_536,
            },
        )?;
        if batch
            .contents
            .iter()
            .fold(bytes.len(), |n, c| n.saturating_add(c.bytes.len()))
            > crate::admission::MAX_RETAINED_BYTES
        {
            return Err(EngineError::invalid());
        }
        Ok(Self {
            identity: Digest(Sha256::digest(&bytes).into()),
            descriptors,
            capture: RetainedInput::RelationshipParse(batch),
            metadata_bytes: bytes.len(),
        })
    }
    pub fn request(&self) -> PublishImportV1 {
        let (receipt, checkpoint) = match &self.capture {
            RetainedInput::Ordinary(c) => (c.receipt.id, c.cursor.clone()),
            RetainedInput::Relationship(c) => (c.receipt.id, SourceValue::Absent),
            RetainedInput::RelationshipParse(c) => (c.receipt.id, SourceValue::Absent),
        };
        PublishImportV1 {
            staged_batch_digest: self.identity,
            receipt,
            checkpoint,
        }
    }
    pub fn plan(&self, state: &DomainState) -> Result<DecisionPlan, EngineError> {
        match &self.capture {
            RetainedInput::Ordinary(c) => {
                EvidenceEngine::publish(state, ValidatedEvidenceBatchV1::new(c.clone())?)
            }
            RetainedInput::RelationshipParse(c) => {
                currents_domain::relationship_parse::RelationshipParseEngine::publish(
                    state,
                    currents_domain::relationship_parse::ValidatedRelationshipParseV1::new(
                        c.clone(),
                    )?,
                )
            }
            RetainedInput::Relationship(c) => {
                currents_domain::relationship_evidence::RelationshipEvidenceEngine::publish(
                    state,
                    currents_domain::relationship_evidence::ValidatedRelationshipCaptureV1::new(
                        c.clone(),
                    )?,
                )
            }
        }
    }
    fn raw_contents(&self) -> &[EvidenceContentV1] {
        match &self.capture {
            RetainedInput::Ordinary(c) => &c.contents,
            RetainedInput::Relationship(c) => &c.contents,
            RetainedInput::RelationshipParse(c) => &c.contents,
        }
    }
    pub(crate) fn contents(&self, records: &[AuthoritativeRecordV1]) -> Vec<EvidenceContentV1> {
        // Reparse may reuse an already-persisted observation. Only stage content
        // referenced by this exact publication; the complete batch stays bound.
        let required: std::collections::BTreeSet<_> = records
            .iter()
            .flat_map(|record| match record {
                AuthoritativeRecordV1::ImportReceipt(r) => {
                    r.contents.iter().map(|c| c.digest).collect::<Vec<_>>()
                }
                AuthoritativeRecordV1::ObservationVersion(o) => vec![o.content_digest],
                _ => vec![],
            })
            .collect();
        self.raw_contents()
            .iter()
            .filter(|c| required.contains(&c.digest))
            .map(|c| (c.digest, c.clone()))
            .collect::<std::collections::BTreeMap<_, _>>()
            .into_values()
            .collect()
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        self.raw_contents()
            .iter()
            .fold(self.metadata_bytes, |n, c| n.saturating_add(c.bytes.len()))
    }
}
