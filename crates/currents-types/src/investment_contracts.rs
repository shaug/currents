//! C11 security and position records. CUR-021 owns their validation.
use crate::*;
// One identifier a source assigns to a security, scoped by that source.
dto!(SecuritySourceIdV1 {
    source: String,
    identifier: String
});
// An owner's decision that another source's identifier names this security.
dto!(SecurityMappingV1 {
    identifier: SecuritySourceIdV1,
    reason: String
});
// One immutable revision of a security. The first revision's `id` is the
// security's `root`; each later revision names the head it replaces. The
// display symbol is never identity.
dto!(SecurityV1 { id:RecordId, schema_version:u32, root:RecordId, prior_revision:Option<RecordId>, unit_id:UnitId, source_identifiers:Vec<SecuritySourceIdV1>, display_symbol:Option<String>, local_mappings:Vec<SecurityMappingV1> });
// A new security, its own security unit and the unit catalog record that
// declares that unit.
dto!(SecurityDeclarationV1 {
    security: SecurityV1,
    unit: UnitDefinition,
    unit_catalog: RecordId
});
// Links one security to the external account that holds its quantity and
// the internal account that holds its carrying amount.
dto!(PositionV1 {
    id: RecordId,
    schema_version: u32,
    security: RecordId,
    quantity_account: AccountId,
    carrying_account: AccountId
});
// Whether a trade adds to a position or takes from it.
choices!(TradeSideV1 { Buy, Sell });
// A buy or sale of one position's security. Each side is `Known`, one leg
// bound to one movement fact, or explicitly `Missing`: `cash` moves an
// external currency account and `quantity` moves the position's quantity
// account. `fee` is in the cash side's unit and is zero when there is none.
// `terms` carries the recognition's identities, postings and dates, and has
// no unresolved account or opening.
dto!(TradeRequestV1 { schema_version:u32, id:RecordId, position:RecordId, side:TradeSideV1, event:FinancialEventV1, cash:SourceValue<AdoptedLegV1>, quantity:SourceValue<AdoptedLegV1>, fee:Amount, terms:RecognitionTermsV1, interpretation:InterpretationV1, governing_policy:PolicyReferenceV1 });
// One recognized trade: its position and side, each side's leg and source
// or its explicit absence, the fee and the recognition of the known sides.
dto!(TradeV1 { id:RecordId, schema_version:u32, position:RecordId, side:TradeSideV1, event_id:RecordId, cash:SourceValue<AdoptedFacetV1>, quantity:SourceValue<AdoptedFacetV1>, fee:Amount, recognition:RecognitionId, interpretation:InterpretationV1, governing_policy:PolicyReferenceV1 });
// One position a holdings snapshot lists: its observation, and the security
// its identifier names with the quantity in that security's unit, or none
// when the identifier names no declared security.
dto!(SnapshotHoldingV1 { observation:RecordId, security:Option<RecordId>, quantity:Option<Amount>, market_value:Option<Amount> });
// A complete snapshot's assertion that a position in its scope holds none
// of its security. Each later assertion for the position is the next
// version and names the snapshot that holds the one before.
dto!(HoldingAbsenceV1 { position:RecordId, security:RecordId, version:u32, prior:Option<RecordId> });
// A source's holdings for an account scope as of one day. It records no
// quantity change.
dto!(HoldingsSnapshotV1 { id:RecordId, schema_version:u32, receipt:RecordId, as_of:TemporalValue, scope:Vec<AccountId>, completeness:CoverageStatus, currency:UnitId, holdings:Vec<SnapshotHoldingV1>, market_value:Option<Amount>, absences:Vec<HoldingAbsenceV1> });
// A reinvested dividend: the ordinary posted adoption of the dividend row
// against an income account, and the buy of the position's security that
// the dividend pays for. They plan and commit together.
dto!(ReinvestmentRequestV1 {
    schema_version: u32,
    id: RecordId,
    dividend: ActivityRequestV1,
    purchase: TradeRequestV1
});
// Links a dividend's income adoption to the trade it reinvests in.
dto!(ReinvestmentV1 {
    id: RecordId,
    schema_version: u32,
    adoption: RecordId,
    trade: RecordId
});
