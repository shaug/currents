//! Bounded synthetic source descriptions only. No local-account matching,
//! revision precedence inference, financial adoption or network access.
use currents_types::*;
pub struct RelationshipSourceParser;
impl RelationshipSourceParser {
    /// This profile is deliberately explicit and small. Success means only
    /// bounded syntactic evidence, not a retained receipt or adopted revision.
    pub fn parse(bytes: &[u8]) -> Result<Vec<RelationshipSourceRowV1>, EngineError> {
        if bytes.len() > 1_048_576 {
            return Err(EngineError::invalid());
        }
        let text = std::str::from_utf8(bytes).map_err(|_| EngineError::invalid())?;
        let lower = text.to_ascii_lowercase();
        if [
            "authorization",
            "bearer ",
            "access_token",
            "refresh_token",
            "password",
            "private key",
            "cookie:",
            "client_secret",
            "token=",
        ]
        .iter()
        .any(|s| lower.contains(s))
            || bytes
                .iter()
                .any(|b| *b == 0 || *b == 127 || (*b < 32 && !matches!(b, b'\t' | b'\r' | b'\n')))
        {
            return Err(EngineError::invalid());
        }
        let rows: Vec<RelationshipSourceRowV1> =
            serde_json::from_slice(bytes).map_err(|_| EngineError::invalid())?;
        RelationshipSourceRowV1::validate_all(&rows)?;
        Ok(rows)
    }
}

impl RelationshipSourceParser {
    /// Retain the exact original file and capture time. No facts are silently
    /// converted to financial legs or assigned precedence by receipt order.
    pub fn capture(
        input: crate::capture::BoundedEvidence,
        id: RecordId,
    ) -> Result<RelationshipCaptureBatchV1, EngineError> {
        use sha2::{Digest as _, Sha256};
        let rows = Self::parse(&input.bytes)?;
        let ctx = input.context;
        if id.0.is_nil()
            || id == ctx.receipt_id
            || ctx.receipt_id.0.is_nil()
            || ctx.source.source != "synthetic"
            || ctx.source.device_id.is_some()
            || ctx.source.connection_id.0.is_nil()
            || !ctx.row_ids.is_empty()
            || ctx.coverage.len() > 16
        {
            return Err(EngineError::invalid());
        }
        let raw = ContentReferenceV1 {
            digest: Digest(Sha256::digest(&input.bytes).into()),
            byte_count: CommitSeq(input.bytes.len() as u64),
            media_type: RELATIONSHIP_SOURCE_MEDIA.into(),
        };
        let receipt = ImportReceiptV1 {
            id: ctx.receipt_id,
            source: ctx.source.clone(),
            adapter_version: "synthetic-relationships/1".into(),
            parser_version: "1".into(),
            requested_coverage: ctx.coverage.clone(),
            reported_coverage: ctx.coverage,
            received_time: ctx.received_at,
            result: ImportResult::Complete,
            cursor: SourceValue::Absent,
            truncated: false,
            complete: true,
            contents: vec![raw.clone()],
        };
        let capture = RelationshipCaptureV1 {
            id,
            schema_version: 1,
            receipt_id: receipt.id,
            source: ctx.source,
            original_capture_time: ctx.captured_at,
            raw: raw.clone(),
            rows,
        };
        Ok(RelationshipCaptureBatchV1 {
            receipt,
            capture,
            contents: vec![EvidenceContentV1 {
                digest: raw.digest,
                bytes: input.bytes,
            }],
        })
    }
}
