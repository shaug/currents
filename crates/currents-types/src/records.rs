//! Foundational record shapes. Domain validation and replay belong to their owners.
use crate::{schema::Schema, *};
use std::collections::BTreeMap;
choices!(UnitKind {
    Currency,
    Security,
    Analytical
});
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitDefinition {
    pub id: UnitId,
    pub exponent: u8,
    pub native_code: String,
    pub kind: UnitKind,
}
dto!(UnitWire {
    id: UnitId,
    exponent: u8,
    native_code: String,
    kind: UnitKind
});
impl UnitDefinition {
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.exponent > 18
            || self.native_code.is_empty()
            || self.native_code.len() > 16
            || !self
                .native_code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            Err(EngineError::precision())
        } else {
            Ok(())
        }
    }
}
impl serde::Serialize for UnitDefinition {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        UnitWire {
            id: self.id,
            exponent: self.exponent,
            native_code: self.native_code.clone(),
            kind: self.kind,
        }
        .serialize(s)
    }
}
impl<'de> serde::Deserialize<'de> for UnitDefinition {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = UnitWire::deserialize(d)?;
        let v = Self {
            id: w.id,
            exponent: w.exponent,
            native_code: w.native_code,
            kind: w.kind,
        };
        v.validate().map_err(serde::de::Error::custom)?;
        Ok(v)
    }
}
impl Schema for UnitDefinition {
    fn schema() -> serde_json::Value {
        let mut s = UnitWire::schema();
        s["properties"]["exponent"]["maximum"] = serde_json::json!(18);
        s["properties"]["native_code"] =
            serde_json::json!({"type":"string","pattern":"^[A-Z0-9]{1,16}$"});
        s
    }
}
choices!(AccountClass {
    Asset,
    Liability,
    Income,
    Expense,
    Equity
});
choices!(NormalSign { Debit, Credit });
choices!(Lifecycle {
    Open,
    Closed,
    Unknown
});
choices!(CoverageStatus {
    Complete,
    Partial,
    Unknown
});
choices!(ObservationKind {
    Movement,
    Balance,
    Holding,
    Valuation,
    Identity,
    Tombstone
});
choices!(RelationshipKind {
    Lifecycle,
    Transfer,
    Payroll,
    Duplicate,
    Overlap
});
choices!(RecognitionKind {
    Activity,
    Opening,
    Reversal,
    Replacement,
    Quantity
});
choices!(ReconciliationStatus {
    Unassessed,
    Matched,
    Partial,
    Unmatched,
    Invalidated
});
choices!(AuthorityKind {
    Owner,
    StandingRule,
    Source
});
choices!(DecisionKind {
    Interpretation,
    Recognition,
    Correction,
    Organization,
    Review,
    Reconciliation,
    Planning,
    Catalog,
    Policy
});
choices!(OrganizationKind {
    Party,
    Alias,
    Category,
    Tag,
    Note,
    ReportingAllocation
});
choices!(ReviewDisposition {
    Unreviewed,
    Accepted,
    Rejected,
    NeedsEvidence
});
choices!(ImportResult {
    Complete,
    Partial,
    Failed
});
choices!(MatchKind { Manual, Rule });
dto!(FormatVersionsV1 {
    records: u32,
    protocol: u32,
    canonical: u32
});
dto!(PolicyVersionsV1 {
    selection: u32,
    recognition: u32,
    query: u32,
    calendar: u32,
    timezone_data: String
});
dto!(OwnershipV1 {
    account_id: AccountId,
    numerator: CommitSeq,
    denominator: CommitSeq
});
dto!(CoverageV1 {
    scope: DateScope,
    status: CoverageStatus
});
dto!(SourceIdentityV1 { source:String, connection_id:RecordId, device_id:Option<RecordId> });
dto!(SourceAliasV1 {
    source: SourceIdentityV1,
    original_id: String
});
dto!(ContentReferenceV1 {
    digest: Digest,
    byte_count: CommitSeq,
    media_type: String
});
dto!(PolicyReferenceV1 {
    id: RecordId,
    version: u32
});
dto!(DatasetDefinitionV1 { id:DatasetId, format_versions:FormatVersionsV1, household_timezone:SourceValue<String>, reporting_currency:UnitId, unit_catalog:RecordId, ownership_perimeter:Vec<OwnershipV1>, commit_head:CommitSeq, policy_versions:PolicyVersionsV1 });
dto!(DatasetPolicyV1 { id:RecordId, version:u32, policies:PolicyVersionsV1, source_precedence:Vec<RecordId>, freshness_seconds:CommitSeq, opening_policy:RecordId, withholding_policy:RecordId });
dto!(UnitCatalogV1 { id:RecordId, version:u32, units:Vec<UnitDefinition> });
dto!(AccountComponentV1 { id:RecordId, account_id:AccountId, account_class:AccountClass, unit_id:UnitId, normal_sign:NormalSign, aliases:Vec<SourceAliasV1>, ownership:OwnershipV1, coverage:Vec<CoverageV1>, opening_boundary:SourceValue<TemporalValue>, overlapping_components:Vec<RecordId>, lifecycle:Lifecycle, display_name:String });
dto!(AccountAliasV1 {
    id: RecordId,
    account_id: AccountId,
    alias: SourceAliasV1
});
dto!(OwnershipPerimeterV1 { id:RecordId, version:u32, ownership:Vec<OwnershipV1> });
dto!(ImportReceiptV1 { id:RecordId, source:SourceIdentityV1, adapter_version:String, parser_version:String, requested_coverage:Vec<CoverageV1>, reported_coverage:Vec<CoverageV1>, received_time:RecordedInstant, result:ImportResult, cursor:SourceValue<String>, truncated:bool, complete:bool, contents:Vec<ContentReferenceV1> });
dto!(ReportedFieldV1 { name:String, value:SourceValue<String> });
dto!(ObservationVersionV1 { id:RecordId, source:SourceIdentityV1, original_ids:Vec<String>, sanitized_representation:String, reported_fields:Vec<ReportedFieldV1>, temporal:TemporalValue, semantic_kind:ObservationKind, prior_version:Option<RecordId>, tombstone_of:Option<RecordId>, content_digest:Digest });
dto!(MovementFacetV1 { lineage_key:RecordId, kind:ObservationKind, amount:SourceValue<Amount>, relationships:Vec<RecordId>, temporal:TemporalValue });
dto!(ParseProductV1 { source_version_id:RecordId, parser_id:String, parser_version:String, facets:Vec<MovementFacetV1> });
dto!(EvidenceAllocationV1 {
    observation: RecordId,
    facet: RecordId,
    amount: Amount
});
dto!(InterpretationV1 { kind:DecisionKind, authority:AuthorityKind, evidence:Vec<RecordId>, reason:String });
dto!(FinancialEventV1 { id:RecordId, legs:Vec<LegId>, relationships:Vec<RecordId>, interpretation:SourceValue<InterpretationV1> });
dto!(FinancialLegV1 { id:LegId, event_id:RecordId, account_id:AccountId, movement:Amount, allocations:Vec<EvidenceAllocationV1>, effective:TemporalValue, posting:TemporalValue, relationships:Vec<RecordId>, unresolved_residual:SourceValue<Amount>, interpretation:SourceValue<InterpretationV1> });
dto!(AssociationV1 { id:RecordId, kind:RelationshipKind, legs:Vec<LegId>, allocations:Vec<EvidenceAllocationV1>, survivor:Option<LegId>, corrections:Vec<RecognitionId>, reason:String });
dto!(PostingV1 {
    id: RecordId,
    account_id: AccountId,
    amount: Amount,
    effective: TemporalValue,
    posting: TemporalValue
});
dto!(RecognitionV1 { id:RecognitionId, lineage:RecordId, postings:Vec<PostingV1>, kind:RecognitionKind, evidence_allocations:Vec<EvidenceAllocationV1>, leg_ids:Vec<LegId>, scope:DateScope, bookkeeping_date:TemporalValue, date_policy:PolicyReferenceV1 });
dto!(AccountingSegmentV1 { id:RecordId, component:RecordId, prior_segment:Option<RecordId>, opening:RecognitionId, included_activity:Vec<RecognitionId>, replacement_closure:Vec<RecognitionId>, approved_accounts:Vec<AccountId>, residual_disposition:SourceValue<String> });
dto!(ReconciliationClaimV1 { id:RecordId, comparison_type:String, scope:Vec<AccountId>, cutoffs:DateScope, evidence_dependencies:Vec<RecordId>, decision_dependencies:Vec<RecordId>, dependency_digest:Digest, matched_allocations:Vec<EvidenceAllocationV1>, differences:Vec<Amount>, exclusions:Vec<RecordId>, status:ReconciliationStatus, reason:String });
dto!(OrganizationRecordV1 { id:RecordId, version:u32, kind:OrganizationKind, targets:Vec<RecordId>, parent:Option<RecordId>, text:String, allocations:Vec<EvidenceAllocationV1>, supersedes:Option<RecordId> });
dto!(RuleV1 { id:RecordId, version:u32, operation_kinds:Vec<DecisionKind>, accounts:Vec<AccountId>, conditions:Vec<ReportedFieldV1>, grant_id:GrantId, revoked:bool });
dto!(ReviewRecordV1 { id:RecordId, targets:Vec<RecordId>, disposition:ReviewDisposition, reason:String });
dto!(ScenarioV1 { id:RecordId, version:u32, basis:ReadBasis, commitments:Vec<CommitmentId>, assumptions:Vec<String> });
dto!(ForecastPointV1 { account_id:AccountId, at:TemporalValue, amount:SourceValue<Amount>, uncertainty:Vec<String> });
dto!(DecisionEnvelopeV1 { id:RecordId, commit:CommitSeq, item_position:u32, decision_type:DecisionKind, schema_version:u32, actor:RecordId, authority:AuthorityKind, grant_id:Option<GrantId>, command_id:CommandId, command_digest:Digest, references:Vec<RecordId>, recorded_time:RecordedInstant, adopted_result:Vec<RecordId>, policy:PolicyReferenceV1, corrections:Vec<RecordId>, supersedes:Vec<RecordId> });
tagged!(AuthoritativeRecordV1 {
 DateConversionPolicy(DateConversionPolicyV1), FrozenAccountingManifest(FrozenAccountingManifestV1), Dataset(DatasetDefinitionV1), DatasetPolicy(DatasetPolicyV1), UnitCatalog(UnitCatalogV1), AccountComponent(AccountComponentV1), AccountAlias(AccountAliasV1), OwnershipPerimeter(OwnershipPerimeterV1), ImportReceipt(ImportReceiptV1), ObservationVersion(ObservationVersionV1), ParseProduct(ParseProductV1), FinancialEvent(FinancialEventV1), FinancialLeg(FinancialLegV1), Association(AssociationV1), Allocation(EvidenceAllocationV1), Decision(DecisionEnvelopeV1), Recognition(RecognitionV1), AccountingSegment(AccountingSegmentV1), ReconciliationClaim(ReconciliationClaimV1), Organization(OrganizationRecordV1), Rule(RuleV1), Review(ReviewRecordV1), Commitment(CommitmentV1), OccurrenceMatch(OccurrenceMatchV1), Scenario(ScenarioV1), ForecastRun(ForecastRunV1)
});
choices!(CommandState {
    Committed,
    Pending,
    ResultUnknown,
    DefinitelyNotCommitted
});
dto!(EffectsV1 { records:Vec<RecordId>, accounts:Vec<AccountId>, amounts:Vec<Amount>, invalidations:Vec<RecordId> });
dto!(CommandResultV1 { command_id:CommandId, digest:Digest, state:CommandState, commit:Option<CommitSeq>, effects:EffectsV1, health:Vec<Health> });
// C05 owns the native format. Preserve its stable bytes, never equate its sequence to the domain's.
dto!(NativeHead {
    semantics_version: u32,
    checkpoint: Digest
});
dto!(DomainCommitV1 { seq:CommitSeq, previous_digest:Digest, records:Vec<AuthoritativeRecordV1>, decisions:Vec<DecisionEnvelopeV1>, native_head_after:NativeHead, command_result:CommandResultV1, digest:Digest });
dto!(MetadataDeclarationV1 { accounts:Vec<AccountComponentV1>, units:Vec<UnitDefinition> });
dto!(CorrectionV1 { active_recognition:RecognitionId, replacement:Vec<RecognitionV1>, remapping:Vec<AssociationV1>, reason:String });
dto!(NoEffectV1 { reason: String });
tagged!(AccountingIntent { MetadataDeclaration(MetadataDeclarationV1), Recognition(RecognitionV1), Correction(CorrectionV1), NoEffect(NoEffectV1) });
dto!(DecisionPlan { records:Vec<AuthoritativeRecordV1>, invalidations:Vec<RecordId>, accounting:AccountingIntent });
/// A nonauthoritative in-memory replay index. It deliberately has no wire/history serialization.
#[derive(Clone, Debug, Default)]
pub struct DomainState {
    pub accounting: Option<AccountingIndexV1>,
    pub records: BTreeMap<RecordId, AuthoritativeRecordV1>,
    pub through: CommitSeq,
}
impl Schema for Health {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"string","enum":["healthy","source_partial","source_stale","locked","projection_lagging","integrity_failed","migration_required","recovery_in_progress"]})
    }
}
