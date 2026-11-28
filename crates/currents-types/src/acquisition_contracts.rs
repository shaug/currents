//! C07 in-process synthetic import values. These are not wire or durable schemas.
use crate::*;

/// Explicit profiles only; no content guessing or live provider formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportFormatV1 {
    SyntheticCsv,
    SyntheticManual,
    InertText,
    SyntheticHoldings,
}
impl ImportFormatV1 {
    pub fn parser_id(self) -> &'static str {
        match self {
            Self::SyntheticCsv => "synthetic-csv",
            Self::SyntheticManual => "synthetic-manual",
            Self::InertText => "inert-text",
            Self::SyntheticHoldings => "synthetic-holdings",
        }
    }
}
#[derive(Clone, Debug)]
pub struct ParserProfileV1 {
    pub schema_version: u32,
    pub format: ImportFormatV1,
    pub parser_version: String,
    pub unit: UnitDefinition,
}
/// Identities are allocated once by trusted command acceptance, never during parsing/replay.
#[derive(Clone, Debug)]
pub struct ImportRowIdsV1 {
    pub observation: RecordId,
    pub product: RecordId,
    pub facet: RecordId,
}
#[derive(Clone, Debug)]
pub struct ImportContextV1 {
    pub receipt_id: RecordId,
    pub source: SourceIdentityV1,
    pub captured_at: RecordedInstant,
    pub received_at: RecordedInstant,
    pub coverage: Vec<CoverageV1>,
    pub row_ids: Vec<ImportRowIdsV1>,
}
/// Retained bytes are staged only after validation; deliberately no Debug implementation.
#[derive(Clone)]
pub struct EvidenceContentV1 {
    pub digest: Digest,
    pub bytes: Vec<u8>,
}
#[derive(Clone)]
pub struct CaptureBatchV1 {
    pub format: ImportFormatV1,
    pub unit: UnitDefinition,
    pub connection_id: RecordId,
    pub source_identity: SourceIdentityV1,
    pub parser_version: String,
    pub coverage: Vec<CoverageV1>,
    pub cursor: SourceValue<String>,
    pub observations: Vec<ObservationVersionV1>,
    pub sanitized_contract: String,
    pub original_capture_time: RecordedInstant,
    pub receipt: ImportReceiptV1,
    pub products: Vec<AddressedParseProductV1>,
    pub contents: Vec<EvidenceContentV1>,
    /// The holdings snapshot a `SyntheticHoldings` batch publishes, with no
    /// absences yet; every other format has none.
    pub snapshot: Option<Box<HoldingsSnapshotV1>>,
}

/// Explicit synthetic source grammar, not a client command or authoritative record.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntheticImportRowV1 {
    pub original_id: SourceValue<String>,
    pub amount: SourceValue<String>,
    pub unit: String,
    pub effective: SourceValue<String>,
    pub description: SourceValue<String>,
}

/// One security the host has declared, by an identifier that names it.
#[derive(Clone, Debug)]
pub struct HoldingsSecurityV1 {
    pub identifier: SecuritySourceIdV1,
    pub security: RecordId,
    pub unit: UnitDefinition,
}
/// The `SyntheticHoldings` profile. `currency` prices market values;
/// `securities` gives each declared security's unit, so the parser can
/// state quantities in it. `snapshot` is the snapshot record's ID.
#[derive(Clone, Debug)]
pub struct HoldingsProfileV1 {
    pub schema_version: u32,
    pub parser_version: String,
    pub currency: UnitDefinition,
    pub securities: Vec<HoldingsSecurityV1>,
    pub snapshot: RecordId,
}
/// Explicit synthetic holdings grammar: one account scope's positions as of
/// one day. `scope` names local accounts.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntheticHoldingsV1 {
    pub as_of: String,
    pub scope: Vec<AccountId>,
    pub completeness: CoverageStatus,
    pub market_value: Option<String>,
    pub positions: Vec<SyntheticHoldingRowV1>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntheticHoldingRowV1 {
    pub source: String,
    pub identifier: String,
    pub symbol: Option<String>,
    pub quantity: String,
    pub market_value: Option<String>,
}

/// One explicitly typed metadata content reference is mandatory for every receipt.
pub const CAPTURE_METADATA_MEDIA_TYPE: &str = "application/vnd.currents.capture-metadata.v1+json";
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportCaptureMetadataV1 {
    pub schema_version: u32,
    pub receipt_id: RecordId,
    pub source_identity: SourceIdentityV1,
    pub original_capture_time: RecordedInstant,
    pub file_digest: Digest,
    pub sanitizer_version: String,
    pub profile_id: String,
    pub parser_version: String,
    pub unit: UnitDefinition,
}
