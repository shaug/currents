//! Pure evidence publication. No storage handles, clocks, UUID generation or monetary authority.
use currents_types::*;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub struct ValidatedEvidenceBatchV1(CaptureBatchV1);
fn invalid<T>(_: T) -> EngineError {
    EngineError::invalid()
}
fn safe(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    ![
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
}
impl ValidatedEvidenceBatchV1 {
    pub fn new(batch: CaptureBatchV1) -> Result<Self, EngineError> {
        let receipt = &batch.receipt;
        batch.unit.validate()?;
        if batch.sanitized_contract != "synthetic-allowlist/1"
            || batch.source_identity.source != "synthetic"
            || batch.source_identity.device_id.is_some()
            || batch.source_identity.connection_id.0.is_nil()
            || batch.connection_id != batch.source_identity.connection_id
            || receipt.source != batch.source_identity
            || receipt.parser_version != batch.parser_version
            || !matches!(batch.parser_version.as_str(), "1" | "2")
            || receipt.adapter_version != "synthetic-file/1"
            || receipt.requested_coverage != batch.coverage
            || receipt.reported_coverage != batch.coverage
            || batch.coverage.len() > 16
            || receipt.cursor != batch.cursor
            || batch.cursor != SourceValue::Absent
            || receipt.truncated
            || !receipt.complete
            || receipt.result != ImportResult::Complete
            || batch.observations.len() > 9_998
            || batch.products.len() != batch.observations.len()
            || receipt.contents.len() != 2
            || batch.contents.len() != batch.observations.len().saturating_add(2)
        {
            return Err(EngineError::invalid());
        }
        let mut bytes = BTreeMap::new();
        for content in &batch.contents {
            if content.bytes.len() > 104_857_600
                || Digest(Sha256::digest(&content.bytes).into()) != content.digest
                || !safe(&content.bytes)
            {
                return Err(EngineError::invalid());
            }
            bytes.insert(content.digest, content.bytes.as_slice());
        }
        let file = receipt.contents.first().ok_or_else(EngineError::invalid)?;
        if batch.contents.first().map(|content| content.digest) != Some(file.digest) {
            return Err(EngineError::invalid());
        }

        if bytes.get(&file.digest).map(|b| b.len() as u64) != Some(file.byte_count.0) {
            return Err(EngineError::invalid());
        }
        let metadata_reference = receipt.contents.get(1).ok_or_else(EngineError::invalid)?;
        let metadata_content = batch.contents.get(1).ok_or_else(EngineError::invalid)?;
        if metadata_reference.media_type != CAPTURE_METADATA_MEDIA_TYPE
            || metadata_reference.digest != metadata_content.digest
            || metadata_reference.byte_count.0 != metadata_content.bytes.len() as u64
            || metadata_content.bytes.len() > 1_048_576
            || file.media_type
                != if batch.format == ImportFormatV1::InertText {
                    "text/plain"
                } else {
                    "application/x-currents-synthetic"
                }
        {
            return Err(EngineError::invalid());
        }
        let metadata: ImportCaptureMetadataV1 =
            serde_json::from_slice(&metadata_content.bytes).map_err(invalid)?;
        if metadata.schema_version != 1 {
            return Err(EngineError::version());
        }
        if metadata
            != (ImportCaptureMetadataV1 {
                schema_version: 1,
                receipt_id: receipt.id,
                source_identity: batch.source_identity.clone(),
                original_capture_time: batch.original_capture_time.clone(),
                file_digest: file.digest,
                sanitizer_version: batch.sanitized_contract.clone(),
                profile_id: batch.format.parser_id().into(),
                parser_version: batch.parser_version.clone(),
                unit: batch.unit.clone(),
            })
        {
            return Err(EngineError::invalid());
        }
        let mut ids = BTreeSet::from([receipt.id]);
        if receipt.id.0.is_nil() {
            return Err(EngineError::invalid());
        }
        for (position, (observation, product)) in
            batch.observations.iter().zip(&batch.products).enumerate()
        {
            if product.schema_version != 1 {
                return Err(EngineError::version());
            }
            if product.product.source_version_id != observation.id
                || product.product.parser_id != batch.format.parser_id()
                || product.product.parser_version != batch.parser_version
                || product.product.facets.len() != 1
                || observation.source != batch.source_identity
                || observation.prior_version.is_some()
                || observation.tombstone_of.is_some()
                || observation.semantic_kind != ObservationKind::Movement
                || observation.sanitized_representation.len() > 1_048_576
                || bytes.get(&observation.content_digest).copied()
                    != Some(observation.sanitized_representation.as_bytes())
                || serde_json::to_vec(observation).map_err(invalid)?.len() > 1_048_576
                || serde_json::to_vec(product).map_err(invalid)?.len() > 1_048_576
            {
                return Err(EngineError::invalid());
            }
            let facet = product
                .product
                .facets
                .first()
                .ok_or_else(EngineError::invalid)?;
            for id in [observation.id, product.id, facet.lineage_key] {
                if id.0.is_nil() || !ids.insert(id) {
                    return Err(EngineError::invalid());
                }
            }
            if facet.kind != ObservationKind::Movement
                || !facet.relationships.is_empty()
                || facet.temporal != observation.temporal
            {
                return Err(EngineError::invalid());
            }
            if batch
                .contents
                .get(position.saturating_add(2))
                .map(|content| content.digest)
                != Some(observation.content_digest)
            {
                return Err(EngineError::invalid());
            }
            validate_row(observation, facet, &batch.unit, file.digest, position)?;
            if let SourceValue::Known(amount) = &facet.amount {
                amount.validate()?;
            }
        }
        Ok(Self(batch))
    }
    /// Trusted host stages exactly these bytes in the existing encrypted store before atomic append.
    pub fn contents(&self) -> &[EvidenceContentV1] {
        &self.0.contents
    }
}
/// Similarity is review-only and carries no merge, supersession or recognition authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportSimilarityCandidateV1 {
    pub incoming: RecordId,
    pub existing: RecordId,
}
pub struct EvidenceEngine;
impl EvidenceEngine {
    pub fn similarity_candidates(
        state: &DomainState,
        batch: &ValidatedEvidenceBatchV1,
    ) -> Vec<ImportSimilarityCandidateV1> {
        let mut candidates = Vec::new();
        for incoming in &batch.0.observations {
            for record in state.records.values() {
                if let AuthoritativeRecordV1::ObservationVersion(existing) = record
                    && existing.id != incoming.id
                    && ["amount", "unit", "effective", "description"]
                        .iter()
                        .all(|name| {
                            match (
                                existing
                                    .reported_fields
                                    .iter()
                                    .find(|field| field.name == *name),
                                incoming
                                    .reported_fields
                                    .iter()
                                    .find(|field| field.name == *name),
                            ) {
                                (Some(existing), Some(incoming)) => {
                                    existing.value == incoming.value
                                }
                                _ => false,
                            }
                        })
                    && existing.original_ids != incoming.original_ids
                {
                    candidates.push(ImportSimilarityCandidateV1 {
                        incoming: incoming.id,
                        existing: existing.id,
                    });
                }
            }
        }
        candidates
    }
    pub fn publish(
        state: &DomainState,
        batch: ValidatedEvidenceBatchV1,
    ) -> Result<DecisionPlan, EngineError> {
        let batch = batch.0;
        let mut records = Vec::new();
        if state.records.contains_key(&batch.receipt.id) {
            return Err(EngineError::invalid());
        }
        records.push(AuthoritativeRecordV1::ImportReceipt(batch.receipt));
        for observation in batch.observations {
            let incoming = AuthoritativeRecordV1::ObservationVersion(observation.clone());
            match state.records.get(&observation.id) {
                Some(prior) if prior != &incoming => return Err(EngineError::invalid()),
                Some(_) => {}
                None => records.push(incoming),
            }
        }
        for product in batch.products {
            if state.records.contains_key(&product.id) {
                return Err(EngineError::invalid());
            }
            // A reparse keeps source facet lineage; a new product cannot rename or resize it.
            for prior in state.records.values() {
                let prior = match prior {
                    AuthoritativeRecordV1::AddressedParseProduct(p) => Some(&p.product),
                    AuthoritativeRecordV1::ParseProduct(p) => Some(p),
                    _ => None,
                };
                if let Some(prior) = prior
                    && prior.source_version_id == product.product.source_version_id
                    && prior
                        .facets
                        .iter()
                        .map(|f| f.lineage_key)
                        .collect::<Vec<_>>()
                        != product
                            .product
                            .facets
                            .iter()
                            .map(|f| f.lineage_key)
                            .collect::<Vec<_>>()
                {
                    return Err(EngineError::invalid());
                }
            }
            records.push(AuthoritativeRecordV1::AddressedParseProduct(product));
        }
        Ok(DecisionPlan {
            records,
            invalidations: vec![],
            accounting: AccountingIntent::NoEffect(NoEffectV1 {
                reason: "Evidence import only; no recognition or provider precedence.".into(),
            }),
        })
    }
}

// Validate independently supplied typed batches against their retained source fields.
fn validate_row(
    observation: &ObservationVersionV1,
    facet: &MovementFacetV1,
    unit: &UnitDefinition,
    file: Digest,
    position: usize,
) -> Result<(), EngineError> {
    let row: SyntheticImportRowV1 =
        serde_json::from_str(&observation.sanitized_representation).map_err(invalid)?;
    if row.unit != unit.native_code {
        return Err(EngineError::invalid());
    }
    let expected = vec![
        ReportedFieldV1 {
            name: "original_id".into(),
            value: row.original_id,
        },
        ReportedFieldV1 {
            name: "amount".into(),
            value: row.amount,
        },
        ReportedFieldV1 {
            name: "unit".into(),
            value: SourceValue::Known(row.unit),
        },
        ReportedFieldV1 {
            name: "effective".into(),
            value: row.effective,
        },
        ReportedFieldV1 {
            name: "description".into(),
            value: row.description,
        },
    ];
    if observation.reported_fields != expected {
        return Err(EngineError::invalid());
    }
    let mut original_ids = vec![
        format!(
            "file:{}",
            serde_json::to_string(&file)
                .map_err(invalid)?
                .trim_matches('"')
        ),
        format!("row:{position}"),
    ];
    if let SourceValue::Known(id) = &expected.first().ok_or_else(EngineError::invalid)?.value {
        original_ids.push(id.clone());
    }
    if observation.original_ids != original_ids {
        return Err(EngineError::invalid());
    }
    let effective = &expected.get(3).ok_or_else(EngineError::invalid)?.value;
    let valid_time = match (effective, &observation.temporal) {
        (
            SourceValue::Known(text),
            TemporalValue::Date {
                civil,
                original_text,
            },
        ) => &civil.to_string() == text && original_text == text,
        (SourceValue::Known(_), _) => false,
        (_, TemporalValue::Unknown { original_text }) => original_text.is_empty(),
        _ => false,
    };
    if !valid_time {
        return Err(EngineError::invalid());
    }
    let amount = &expected.get(1).ok_or_else(EngineError::invalid)?.value;
    if let SourceValue::Known(text) = amount {
        let unsigned = text.strip_prefix('-').unwrap_or(text);
        let mut parts = unsigned.split('.');
        let whole = parts.next().ok_or_else(EngineError::invalid)?;
        let fraction = parts.next();
        if text.len() > 58
            || whole.is_empty()
            || (whole.len() > 1 && whole.starts_with('0'))
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || parts.next().is_some()
            || fraction.is_some_and(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(EngineError::invalid());
        }
        let scale = u8::try_from(fraction.map_or(0, str::len)).map_err(invalid)?;
        if scale > unit.exponent {
            return Err(EngineError::precision());
        }
        let parsed = Amount::new(
            text.replace('.', "").parse().map_err(invalid)?,
            scale,
            unit.id,
        )?;
        if facet.amount != SourceValue::Known(parsed) {
            return Err(EngineError::invalid());
        }
    } else {
        let source = serde_json::to_value(amount).map_err(invalid)?;
        let parsed = serde_json::to_value(&facet.amount).map_err(invalid)?;
        if source != parsed {
            return Err(EngineError::invalid());
        }
    }
    Ok(())
}
