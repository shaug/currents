//! Strict selected synthetic profiles. Preserves row order, explicit source states and bytes.
use crate::capture::{BoundedEvidence, MAX_CHUNK_BYTES};
use currents_types::*;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

type Row = SyntheticImportRowV1;
pub struct FileParser;
fn digest(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}
fn invalid<T>(_: T) -> EngineError {
    EngineError::invalid()
}
fn state_map<T, U>(
    value: SourceValue<T>,
    f: impl FnOnce(T) -> Result<U, EngineError>,
) -> Result<SourceValue<U>, EngineError> {
    Ok(match value {
        SourceValue::Known(v) => SourceValue::Known(f(v)?),
        SourceValue::Missing => SourceValue::Missing,
        SourceValue::Unknown => SourceValue::Unknown,
        SourceValue::Null => SourceValue::Null,
        SourceValue::Absent => SourceValue::Absent,
        SourceValue::Deleted => SourceValue::Deleted,
    })
}
fn amount(text: String, unit: &UnitDefinition) -> Result<Amount, EngineError> {
    if text.is_empty() || text.len() > 58 {
        return Err(EngineError::precision());
    }
    let unsigned = text.strip_prefix('-').unwrap_or(&text);
    let mut parts = unsigned.split('.');
    let whole = parts.next().ok_or_else(EngineError::precision)?;
    let fraction = parts.next();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || (whole.len() > 1 && whole.starts_with('0'))
        || fraction.is_some_and(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(EngineError::precision());
    }
    let scale = u8::try_from(fraction.map_or(0, str::len)).map_err(invalid)?;
    if scale > unit.exponent {
        return Err(EngineError::precision());
    }
    let coefficient = text.replace('.', "").parse::<i128>().map_err(invalid)?;
    Amount::new(coefficient, scale, unit.id)
}
/// The CSV library deliberately accepts malformed quotes; this fixed-profile lexical gate does not.
fn csv_quotes(text: &str) -> Result<(), EngineError> {
    let mut quoted = false;
    let mut field_start = true;
    let mut after_quote = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                } else {
                    quoted = false;
                    after_quote = true;
                }
            }
        } else if after_quote {
            if matches!(c, ',' | '\n' | '\r') {
                after_quote = false;
                field_start = true;
            } else {
                return Err(EngineError::invalid());
            }
        } else if c == '"' {
            if !field_start {
                return Err(EngineError::invalid());
            }
            quoted = true;
            field_start = false;
        } else {
            field_start = matches!(c, ',' | '\n' | '\r');
        }
    }
    if quoted {
        Err(EngineError::invalid())
    } else {
        Ok(())
    }
}
fn rows(bytes: &[u8], format: ImportFormatV1) -> Result<Vec<Row>, EngineError> {
    if format == ImportFormatV1::SyntheticManual {
        if bytes.len() > MAX_CHUNK_BYTES {
            return Err(EngineError::invalid());
        }
        return serde_json::from_slice(bytes).map_err(invalid);
    }
    let text = std::str::from_utf8(bytes).map_err(invalid)?;
    csv_quotes(text)?;
    let mut reader = csv::ReaderBuilder::new().flexible(false).from_reader(bytes);
    if reader
        .headers()
        .map_err(invalid)?
        .iter()
        .collect::<Vec<_>>()
        != ["original_id", "amount", "unit", "effective", "description"]
    {
        return Err(EngineError::invalid());
    }
    let mut out = Vec::new();
    for record in reader.records() {
        let r = record.map_err(invalid)?;
        if r.as_slice().len() > MAX_CHUNK_BYTES || out.len() >= 10_000 {
            return Err(EngineError::invalid());
        }
        let known = |n| {
            r.get(n)
                .map(|s| SourceValue::Known(s.to_owned()))
                .ok_or_else(EngineError::invalid)
        };
        out.push(Row {
            original_id: known(0)?,
            amount: known(1)?,
            unit: r.get(2).ok_or_else(EngineError::invalid)?.into(),
            effective: known(3)?,
            description: known(4)?,
        });
    }
    Ok(out)
}
impl FileParser {
    pub fn parse(
        input: BoundedEvidence,
        profile: ParserProfileV1,
    ) -> Result<CaptureBatchV1, EngineError> {
        if profile.schema_version != 1 || !matches!(profile.parser_version.as_str(), "1" | "2") {
            return Err(EngineError::version());
        }
        profile.unit.validate()?;
        // No generic transport envelope is accepted. Secret-bearing bodies fail before capture.
        let text = std::str::from_utf8(&input.bytes).map_err(invalid)?;
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
        .any(|needle| lower.contains(needle))
            || text
                .bytes()
                .any(|b| b == 0 || b == 127 || (b < 32 && !matches!(b, b'\t' | b'\r' | b'\n')))
            || text.starts_with("MZ")
            || text.starts_with("PK")
            || text.starts_with("#!")
        {
            return Err(EngineError::invalid());
        }
        let ctx = input.context;
        if ctx.source.source != "synthetic"
            || ctx.source.device_id.is_some()
            || ctx.source.connection_id.0.is_nil()
            || ctx.receipt_id.0.is_nil()
            || ctx.coverage.len() > 16
        {
            return Err(EngineError::invalid());
        }
        let mut identities = BTreeSet::from([ctx.receipt_id]);
        for ids in &ctx.row_ids {
            for id in [ids.observation, ids.product, ids.facet] {
                if id.0.is_nil() || !identities.insert(id) {
                    return Err(EngineError::invalid());
                }
            }
        }
        let parsed = if profile.format == ImportFormatV1::InertText {
            if lower.contains("http:") || lower.contains("https:") || text.contains('<') {
                return Err(EngineError::invalid());
            }
            Vec::new()
        } else {
            rows(&input.bytes, profile.format)?
        };
        // Reserve a receipt, frozen no-effect manifest and decision under C04's unchanged 20,000-item limit.
        if parsed.len() != ctx.row_ids.len() || parsed.len() > 9_998 {
            return Err(EngineError::invalid());
        }
        let file_digest = digest(&input.bytes);
        let mut contents = vec![EvidenceContentV1 {
            digest: file_digest,
            bytes: input.bytes,
        }];
        let metadata = ImportCaptureMetadataV1 {
            schema_version: 1,
            receipt_id: ctx.receipt_id,
            source_identity: ctx.source.clone(),
            original_capture_time: ctx.captured_at.clone(),
            file_digest,
            sanitizer_version: "synthetic-allowlist/1".into(),
            profile_id: profile.format.parser_id().into(),
            parser_version: profile.parser_version.clone(),
            unit: profile.unit.clone(),
        };
        let metadata_bytes = serde_json::to_vec(&metadata).map_err(invalid)?;
        if metadata_bytes.len() > MAX_CHUNK_BYTES {
            return Err(EngineError::invalid());
        }
        let metadata_reference = ContentReferenceV1 {
            digest: digest(&metadata_bytes),
            byte_count: CommitSeq(metadata_bytes.len() as u64),
            media_type: CAPTURE_METADATA_MEDIA_TYPE.into(),
        };
        contents.push(EvidenceContentV1 {
            digest: metadata_reference.digest,
            bytes: metadata_bytes,
        });
        let mut observations = Vec::new();
        let mut products = Vec::new();
        for (position, (row, ids)) in parsed.into_iter().zip(ctx.row_ids).enumerate() {
            if row.unit != profile.unit.native_code {
                return Err(EngineError::invalid());
            }
            let row_bytes = serde_json::to_vec(&row).map_err(invalid)?;
            if row_bytes.len() > MAX_CHUNK_BYTES {
                return Err(EngineError::invalid());
            }
            let row_digest = digest(&row_bytes);
            let temporal = match &row.effective {
                SourceValue::Known(s) => {
                    let fmt =
                        ::time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
                            .map_err(invalid)?;
                    TemporalValue::Date {
                        civil: Date::parse(s, &fmt).map_err(invalid)?,
                        original_text: s.clone(),
                    }
                }
                _ => TemporalValue::Unknown {
                    original_text: String::new(),
                },
            };
            let value = state_map(row.amount.clone(), |s| amount(s, &profile.unit))?;
            let fields = vec![
                ReportedFieldV1 {
                    name: "original_id".into(),
                    value: row.original_id.clone(),
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
            let mut original_ids = vec![
                format!(
                    "file:{}",
                    serde_json::to_string(&file_digest)
                        .map_err(invalid)?
                        .trim_matches('"')
                ),
                format!("row:{position}"),
            ];
            if let SourceValue::Known(id) = row.original_id {
                original_ids.push(id);
            }
            observations.push(ObservationVersionV1 {
                id: ids.observation,
                source: ctx.source.clone(),
                original_ids,
                sanitized_representation: String::from_utf8(row_bytes.clone()).map_err(invalid)?,
                reported_fields: fields,
                temporal: temporal.clone(),
                semantic_kind: ObservationKind::Movement,
                prior_version: None,
                tombstone_of: None,
                content_digest: row_digest,
            });
            products.push(AddressedParseProductV1 {
                id: ids.product,
                schema_version: 1,
                product: ParseProductV1 {
                    source_version_id: ids.observation,
                    parser_id: profile.format.parser_id().into(),
                    parser_version: profile.parser_version.clone(),
                    facets: vec![MovementFacetV1 {
                        lineage_key: ids.facet,
                        kind: ObservationKind::Movement,
                        amount: value,
                        relationships: vec![],
                        temporal,
                    }],
                },
                balance_kinds: vec![],
            });
            contents.push(EvidenceContentV1 {
                digest: row_digest,
                bytes: row_bytes,
            });
        }
        let receipt = ImportReceiptV1 {
            id: ctx.receipt_id,
            source: ctx.source.clone(),
            adapter_version: "synthetic-file/1".into(),
            parser_version: profile.parser_version.clone(),
            requested_coverage: ctx.coverage.clone(),
            reported_coverage: ctx.coverage.clone(),
            received_time: ctx.received_at,
            result: ImportResult::Complete,
            cursor: SourceValue::Absent,
            truncated: false,
            complete: true,
            contents: vec![
                ContentReferenceV1 {
                    digest: file_digest,
                    byte_count: CommitSeq(
                        contents
                            .first()
                            .ok_or_else(EngineError::invalid)?
                            .bytes
                            .len() as u64,
                    ),
                    media_type: if profile.format == ImportFormatV1::InertText {
                        "text/plain"
                    } else {
                        "application/x-currents-synthetic"
                    }
                    .into(),
                },
                metadata_reference,
            ],
        };
        Ok(CaptureBatchV1 {
            format: profile.format,
            unit: profile.unit,
            connection_id: ctx.source.connection_id,
            source_identity: ctx.source,
            parser_version: profile.parser_version,
            coverage: ctx.coverage,
            cursor: SourceValue::Absent,
            observations,
            sanitized_contract: "synthetic-allowlist/1".into(),
            original_capture_time: ctx.captured_at,
            receipt,
            products,
            contents,
        })
    }
}
