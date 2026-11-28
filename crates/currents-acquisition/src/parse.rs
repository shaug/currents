//! Strict selected synthetic profiles. Preserves row order, explicit source states and bytes.
use crate::capture::{BoundedEvidence, MAX_CHUNK_BYTES};
use currents_types::*;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

type CsvRow = SyntheticImportRowV1;
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
fn rows(bytes: &[u8], format: ImportFormatV1) -> Result<Vec<CsvRow>, EngineError> {
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
        out.push(CsvRow {
            original_id: known(0)?,
            amount: known(1)?,
            unit: r.get(2).ok_or_else(EngineError::invalid)?.into(),
            effective: known(3)?,
            description: known(4)?,
        });
    }
    Ok(out)
}
/// Refuse a body that is not plain text or that bears a secret marker, then
/// the context's identities. No generic transport envelope is accepted.
fn screen(bytes: &[u8], ctx: &ImportContextV1) -> Result<(), EngineError> {
    let text = std::str::from_utf8(bytes).map_err(invalid)?;
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
    Ok(())
}
fn day(text: &str) -> Result<TemporalValue, EngineError> {
    let fmt =
        ::time::format_description::parse_borrowed::<2>("[year]-[month]-[day]").map_err(invalid)?;
    Ok(TemporalValue::Date {
        civil: Date::parse(text, &fmt).map_err(invalid)?,
        original_text: text.to_owned(),
    })
}
/// The file and row identities a row's observation lists first.
fn row_ids(file: Digest, position: usize) -> Result<Vec<String>, EngineError> {
    Ok(vec![
        format!(
            "file:{}",
            serde_json::to_string(&file)
                .map_err(invalid)?
                .trim_matches('"')
        ),
        format!("row:{position}"),
    ])
}
/// One parsed row: its observation, its product and its retained bytes.
type Parsed = (ObservationVersionV1, AddressedParseProductV1, Vec<u8>);
/// The batch that retains `file`, its capture metadata and each row.
fn assemble(
    file: Vec<u8>,
    ctx: ImportContextV1,
    format: ImportFormatV1,
    parser_version: String,
    unit: UnitDefinition,
    parsed: Vec<Parsed>,
) -> Result<CaptureBatchV1, EngineError> {
    let file_digest = digest(&file);
    let file_reference = ContentReferenceV1 {
        digest: file_digest,
        byte_count: CommitSeq(file.len() as u64),
        media_type: if format == ImportFormatV1::InertText {
            "text/plain"
        } else {
            "application/x-currents-synthetic"
        }
        .into(),
    };
    let metadata = ImportCaptureMetadataV1 {
        schema_version: 1,
        receipt_id: ctx.receipt_id,
        source_identity: ctx.source.clone(),
        original_capture_time: ctx.captured_at.clone(),
        file_digest,
        sanitizer_version: "synthetic-allowlist/1".into(),
        profile_id: format.parser_id().into(),
        parser_version: parser_version.clone(),
        unit: unit.clone(),
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
    let mut contents = vec![
        EvidenceContentV1 {
            digest: file_digest,
            bytes: file,
        },
        EvidenceContentV1 {
            digest: metadata_reference.digest,
            bytes: metadata_bytes,
        },
    ];
    let mut observations = Vec::new();
    let mut products = Vec::new();
    for (observation, product, bytes) in parsed {
        contents.push(EvidenceContentV1 {
            digest: observation.content_digest,
            bytes,
        });
        observations.push(observation);
        products.push(product);
    }
    let receipt = ImportReceiptV1 {
        id: ctx.receipt_id,
        source: ctx.source.clone(),
        adapter_version: "synthetic-file/1".into(),
        parser_version: parser_version.clone(),
        requested_coverage: ctx.coverage.clone(),
        reported_coverage: ctx.coverage.clone(),
        received_time: ctx.received_at,
        result: ImportResult::Complete,
        cursor: SourceValue::Absent,
        truncated: false,
        complete: true,
        contents: vec![file_reference, metadata_reference],
    };
    Ok(CaptureBatchV1 {
        format,
        unit,
        connection_id: ctx.source.connection_id,
        source_identity: ctx.source,
        parser_version,
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
/// What one row reports: its retained bytes, its original identities and
/// fields, its date, and its one facet's kind and amount.
struct Row {
    bytes: Vec<u8>,
    original_ids: Vec<String>,
    fields: Vec<ReportedFieldV1>,
    temporal: TemporalValue,
    kind: ObservationKind,
    amount: SourceValue<Amount>,
}
/// `row`'s observation and its product with one facet, under `ids`.
fn observed(
    ctx: &ImportContextV1,
    ids: &ImportRowIdsV1,
    parser: (ImportFormatV1, &str),
    row: Row,
) -> Result<Parsed, EngineError> {
    if row.bytes.len() > MAX_CHUNK_BYTES {
        return Err(EngineError::invalid());
    }
    let observation = ObservationVersionV1 {
        id: ids.observation,
        source: ctx.source.clone(),
        original_ids: row.original_ids,
        sanitized_representation: String::from_utf8(row.bytes.clone()).map_err(invalid)?,
        reported_fields: row.fields,
        temporal: row.temporal.clone(),
        semantic_kind: row.kind,
        prior_version: None,
        tombstone_of: None,
        content_digest: digest(&row.bytes),
    };
    let product = AddressedParseProductV1 {
        id: ids.product,
        schema_version: 1,
        product: ParseProductV1 {
            source_version_id: ids.observation,
            parser_id: parser.0.parser_id().into(),
            parser_version: parser.1.into(),
            facets: vec![MovementFacetV1 {
                lineage_key: ids.facet,
                kind: row.kind,
                amount: row.amount,
                relationships: vec![],
                temporal: row.temporal,
            }],
        },
        balance_kinds: vec![],
    };
    Ok((observation, product, row.bytes))
}
fn field(name: &str, value: SourceValue<String>) -> ReportedFieldV1 {
    ReportedFieldV1 {
        name: name.into(),
        value,
    }
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
        screen(&input.bytes, &input.context)?;
        let ctx = input.context;
        let parsed = if profile.format == ImportFormatV1::InertText {
            let lower = String::from_utf8_lossy(&input.bytes).to_ascii_lowercase();
            if lower.contains("http:") || lower.contains("https:") || lower.contains('<') {
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
        let mut out = Vec::new();
        for (position, (row, ids)) in parsed.into_iter().zip(&ctx.row_ids).enumerate() {
            if row.unit != profile.unit.native_code {
                return Err(EngineError::invalid());
            }
            let bytes = serde_json::to_vec(&row).map_err(invalid)?;
            let temporal = match &row.effective {
                SourceValue::Known(s) => day(s)?,
                _ => TemporalValue::Unknown {
                    original_text: String::new(),
                },
            };
            let amount = state_map(row.amount.clone(), |s| amount(s, &profile.unit))?;
            let mut original_ids = row_ids(file_digest, position)?;
            if let SourceValue::Known(id) = &row.original_id {
                original_ids.push(id.clone());
            }
            let fields = vec![
                field("original_id", row.original_id),
                field("amount", row.amount),
                field("unit", SourceValue::Known(row.unit)),
                field("effective", row.effective),
                field("description", row.description),
            ];
            let row = Row {
                bytes,
                original_ids,
                fields,
                temporal,
                kind: ObservationKind::Movement,
                amount,
            };
            out.push(observed(
                &ctx,
                ids,
                (profile.format, &profile.parser_version),
                row,
            )?);
        }
        assemble(
            input.bytes,
            ctx,
            profile.format,
            profile.parser_version,
            profile.unit,
            out,
        )
    }
}
