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

impl RelationshipSourceParser {
    /// Apply an explicitly supplied synthetic amount-correction profile. This
    /// does not claim a new provider observation or adopt any financial effect.
    pub fn correct(
        original: &RelationshipCaptureV1,
        prior: Option<&RelationshipParseRevisionV1>,
        profile_bytes: Vec<u8>,
        context: ImportContextV1,
        id: RecordId,
    ) -> Result<RelationshipParseBatchV1, EngineError> {
        use sha2::{Digest as _, Sha256};
        if profile_bytes.len() > 1_048_576
            || context.source != original.source
            || !context.row_ids.is_empty()
            || context.coverage.len() > 16
            || id.0.is_nil()
            || context.receipt_id.0.is_nil()
            || id == context.receipt_id
        {
            return Err(EngineError::invalid());
        }
        let correction: RelationshipCorrectionProfileV1 =
            serde_json::from_slice(&profile_bytes).map_err(|_| EngineError::invalid())?;
        let base = original
            .rows
            .iter()
            .find(|r| {
                r.source_record == correction.source_record
                    && r.source_revision == correction.source_revision
            })
            .ok_or_else(EngineError::invalid)?;
        let previous = match prior {
            Some(p)
                if p.capture == original.id
                    && p.row.source_record == base.source_record
                    && p.row.source_revision == base.source_revision =>
            {
                &p.row
            }
            Some(_) => return Err(EngineError::invalid()),
            None => base,
        };
        let row = correction.apply(previous)?;
        let profile = ContentReferenceV1 {
            digest: Digest(Sha256::digest(&profile_bytes).into()),
            byte_count: CommitSeq(profile_bytes.len() as u64),
            media_type: RELATIONSHIP_CORRECTION_MEDIA.into(),
        };
        let receipt = ImportReceiptV1 {
            id: context.receipt_id,
            source: context.source,
            adapter_version: "synthetic-relationships/1".into(),
            parser_version: RELATIONSHIP_CORRECTION_PARSER.into(),
            requested_coverage: context.coverage.clone(),
            reported_coverage: context.coverage,
            received_time: context.received_at,
            result: ImportResult::Complete,
            cursor: SourceValue::Absent,
            truncated: false,
            complete: true,
            contents: vec![profile.clone()],
        };
        let revision = RelationshipParseRevisionV1 {
            id,
            schema_version: 1,
            receipt_id: receipt.id,
            capture: original.id,
            prior_parse: prior.map(|p| p.id),
            profile: profile.clone(),
            correction,
            row,
        };
        Ok(RelationshipParseBatchV1 {
            receipt,
            revision,
            contents: vec![EvidenceContentV1 {
                digest: profile.digest,
                bytes: profile_bytes,
            }],
        })
    }
}
