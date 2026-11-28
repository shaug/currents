//! Foundational record shapes. Domain validation and replay belong to their owners.
use crate::{schema::Schema, *};
use std::collections::{BTreeMap, BTreeSet};
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
// A claim's lifecycle (technical design section 6).
choices!(ReconciliationStatus {
    Proposed,
    PartiallySupported,
    ConfirmedForScope,
    Disputed,
    Superseded
});
/// One source line: a facet of one observation version.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct EvidenceLineV1 {
    pub observation: RecordId,
    pub facet: RecordId,
}
impl Schema for EvidenceLineV1 {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"object","additionalProperties":false,"required":["observation","facet"],"properties":{"observation":RecordId::schema(),"facet":RecordId::schema()}})
    }
}
/// Why a claim is only partially supported (technical design section 6).
/// A line reason names the exact source line it concerns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ClaimReasonV1 {
    UnknownOpening,
    IncompleteInterval,
    IncompatibleBalanceKinds,
    MissingDateMapping,
    UnexplainedResidual,
    UnknownClosing,
    /// The source balance is the evidence the account's opening adopted, so
    /// it cannot corroborate that opening.
    CircularAnchor,
    /// The source balance reports no balance kind.
    UnknownBalanceKind,
    /// The source balance is not in the account's unit.
    IncompatibleUnits,
    /// The source balance's as-of date is not the claim's boundary.
    IncompatibleBoundary,
    /// A source line that no recognized leg allocates.
    MissingAllocation(EvidenceLineV1),
    /// A recognized leg in scope allocates a line the source does not report.
    UnexpectedAllocation(EvidenceLineV1),
    /// A line that the source reports, or legs allocate, more than once.
    RepeatedIdentity(EvidenceLineV1),
    /// A line whose account, date or amount differs from its allocation.
    DifferentTerms(EvidenceLineV1),
    /// A recognized leg its source allocations do not fully support: in an
    /// Activity claim, a leg in scope that allocates no source line, such as
    /// a leg an association admitted; in a Relationship claim, a leg whose
    /// allocations differ from its amount.
    UnsupportedLeg(LegId),
    /// The journal records no security quantities, so a holdings claim has
    /// nothing to compare the source holdings with.
    JournalQuantityUnavailable,
}
impl Schema for ClaimReasonV1 {
    fn schema() -> serde_json::Value {
        let variant = |name: &str, value: serde_json::Value| serde_json::json!({"type":"object","additionalProperties":false,"required":[name],"properties":{name:value}});
        let line = |name: &str| variant(name, EvidenceLineV1::schema());
        serde_json::json!({"oneOf":[
            {"type":"string","enum":["UnknownOpening","IncompleteInterval","IncompatibleBalanceKinds","MissingDateMapping","UnexplainedResidual","UnknownClosing","CircularAnchor","UnknownBalanceKind","IncompatibleUnits","IncompatibleBoundary","JournalQuantityUnavailable"]},
            line("MissingAllocation"),
            line("UnexpectedAllocation"),
            line("RepeatedIdentity"),
            line("DifferentTerms"),
            variant("UnsupportedLeg", LegId::schema())
        ]})
    }
}
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
// One owner-adopted, effective-dated revision of a dataset policy family.
// Each changed field is exactly as the request states it; a null field is
// one this revision does not set.
dto!(DatasetPolicyRevisionV1 { id:RecordId, policy_id:RecordId, version:u32, prior_version:Option<u32>, effective_scope:DateScope, ownership_perimeter:Option<PolicyChangeV1<Vec<OwnershipV1>>>, source_priority:Option<PolicyChangeV1<Vec<RecordId>>>, freshness_thresholds:Option<PolicyChangeV1<Vec<FreshnessThresholdV1>>>, timezone:Option<PolicyChangeV1<TimezonePolicyV1>>, reporting_currency:Option<PolicyChangeV1<UnitId>> });
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
// What a source-reported balance measures (technical design section 6).
// Only balances of the same kind compare.
choices!(BalanceKindV1 {
    Posted,
    Available,
    Statement,
    Unknown
});
dto!(BalanceFacetKindV1 {
    facet: RecordId,
    kind: BalanceKindV1
});
/// Addressable immutable product. Legacy unaddressed products remain unchanged.
/// Schema version 2 adds `balance_kinds`: one kind for each balance facet, in
/// facet ID order. A version-1 product has none, and its bytes are unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddressedParseProductV1 {
    pub id: RecordId,
    pub schema_version: u32,
    pub product: ParseProductV1,
    pub balance_kinds: Vec<BalanceFacetKindV1>,
}
dto!(AddressedParseProductWireV1 {
    id: RecordId,
    schema_version: u32,
    product: ParseProductV1
});
dto!(AddressedParseProductWireV2 {
    id: RecordId,
    schema_version: u32,
    product: ParseProductV1,
    balance_kinds: Vec<BalanceFacetKindV1>
});
impl AddressedParseProductV1 {
    pub fn validate(&self) -> Result<(), EngineError> {
        if !matches!(self.schema_version, 1 | 2) {
            return Err(EngineError::version());
        }
        if self.id.0.is_nil()
            || self.id == self.product.source_version_id
            || self
                .product
                .facets
                .iter()
                .any(|facet| facet.lineage_key == self.id)
        {
            return Err(EngineError::invalid());
        }
        let mut balances: Vec<RecordId> = self
            .product
            .facets
            .iter()
            .filter(|facet| facet.kind == ObservationKind::Balance)
            .map(|facet| facet.lineage_key)
            .collect();
        balances.sort();
        let named: Vec<RecordId> = self.balance_kinds.iter().map(|k| k.facet).collect();
        let kinds_valid = if self.schema_version == 1 {
            named.is_empty()
        } else {
            named == balances && named.windows(2).all(|pair| pair[0] < pair[1])
        };
        if !kinds_valid {
            return Err(EngineError::invalid());
        }
        Ok(())
    }
    /// The kind of balance `facet` reports. A version-1 product reports no
    /// kind, so its balances read as `Unknown`.
    pub fn balance_kind(&self, facet: RecordId) -> BalanceKindV1 {
        self.balance_kinds
            .iter()
            .find(|k| k.facet == facet)
            .map_or(BalanceKindV1::Unknown, |k| k.kind)
    }
}
impl serde::Serialize for AddressedParseProductV1 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        if self.schema_version == 1 {
            return AddressedParseProductWireV1 {
                id: self.id,
                schema_version: self.schema_version,
                product: self.product.clone(),
            }
            .serialize(serializer);
        }
        AddressedParseProductWireV2 {
            id: self.id,
            schema_version: self.schema_version,
            product: self.product.clone(),
            balance_kinds: self.balance_kinds.clone(),
        }
        .serialize(serializer)
    }
}
impl<'de> serde::Deserialize<'de> for AddressedParseProductV1 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        // The version decides the shape: only version 1 omits `balance_kinds`.
        let value = serde_json::Value::deserialize(deserializer)?;
        let product = if value.get("schema_version") == Some(&serde_json::json!(1)) {
            let wire = AddressedParseProductWireV1::deserialize(value).map_err(D::Error::custom)?;
            Self {
                id: wire.id,
                schema_version: wire.schema_version,
                product: wire.product,
                balance_kinds: vec![],
            }
        } else {
            let wire = AddressedParseProductWireV2::deserialize(value).map_err(D::Error::custom)?;
            Self {
                id: wire.id,
                schema_version: wire.schema_version,
                product: wire.product,
                balance_kinds: wire.balance_kinds,
            }
        };
        product.validate().map_err(D::Error::custom)?;
        Ok(product)
    }
}
impl Schema for AddressedParseProductV1 {
    fn schema() -> serde_json::Value {
        let mut first = AddressedParseProductWireV1::schema();
        first["properties"]["schema_version"] = serde_json::json!({"const":1});
        let mut second = AddressedParseProductWireV2::schema();
        second["properties"]["schema_version"] = serde_json::json!({"const":2});
        serde_json::json!({"oneOf":[first, second]})
    }
}
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
dto!(RecognitionCorrectionRevisionV1 {
    id: RecordId,
    schema_version: u32,
    prior_recognition: RecognitionId,
    replacement_recognition: RecognitionId,
    reason: String,
    remap_disposition: RecognitionCorrectionRemapV1
});
/// What a correction does to its recognition's legs and interpretation. A
/// version-1 revision keeps the legs and interpretation and changes
/// identities only; a version-2 revision keeps the legs and records a new
/// interpretation. A version-3 revision retires the adoption that owns the
/// legs and names the fresh adoption that replaces them for the same source
/// facets.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum RecognitionCorrectionRemapV1 {
    UnchangedStableLegs,
    ReinterpretedStableLegs(StableLegsReinterpretationV1),
    ReplacedLegs {
        retired_adoption: RecordId,
        replacement_adoption: RecordId,
    },
}
impl RecognitionCorrectionRemapV1 {
    /// The revision schema version this disposition requires.
    pub fn schema_version(&self) -> u32 {
        match self {
            Self::UnchangedStableLegs => 1,
            Self::ReinterpretedStableLegs(_) => 2,
            Self::ReplacedLegs { .. } => 3,
        }
    }
}
impl Schema for RecognitionCorrectionRemapV1 {
    fn schema() -> serde_json::Value {
        serde_json::json!({"oneOf": [
            {"const": "UnchangedStableLegs"},
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["ReinterpretedStableLegs"],
                "properties": {
                    "ReinterpretedStableLegs": StableLegsReinterpretationV1::schema()
                }
            },
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["ReplacedLegs"],
                "properties": {
                    "ReplacedLegs": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["retired_adoption", "replacement_adoption"],
                        "properties": {
                            "retired_adoption": RecordId::schema(),
                            "replacement_adoption": RecordId::schema()
                        }
                    }
                }
            }
        ]})
    }
}
// The new interpretation of a recognition whose legs stay: the exact
// governing policy, the unresolved-equity account it names, if any, and every
// account whose balance the correction moves on some date.
dto!(StableLegsReinterpretationV1 {
    governing_policy: PolicyReferenceV1,
    unresolved_account: Option<AccountId>,
    changed_accounts: Vec<AccountId>
});
// One transfer recognition that a segment replacement reversed and issued
// again with fresh recognition and posting IDs and unchanged economics.
dto!(SegmentReemissionV1 {
    id: RecordId,
    schema_version: u32,
    segment: RecordId,
    original: RecognitionId,
    replacement: RecognitionId
});
dto!(AccountingSegmentV1 { id:RecordId, component:RecordId, prior_segment:Option<RecordId>, opening:RecognitionId, included_activity:Vec<RecognitionId>, replacement_closure:Vec<RecognitionId>, approved_accounts:Vec<AccountId>, residual_disposition:SourceValue<String> });
// `accounting_dependencies` names the recognitions, legs, association
// revisions and segments the claim compared.
dto!(ReconciliationClaimV1 { id:RecordId, kind:ReconciliationKindV1, comparison_version:u32, basis:ReadBasis, scope:Vec<AccountId>, unit:UnitId, cutoffs:DateScope, evidence_dependencies:Vec<RecordId>, decision_dependencies:Vec<RecordId>, accounting_dependencies:Vec<RecordId>, dependency_digest:Digest, matched_allocations:Vec<EvidenceAllocationV1>, differences:Vec<Amount>, exclusions:Vec<RecordId>, status:ReconciliationStatus, reasons:Vec<ClaimReasonV1>, reason:String });
// An owner's confirmation or dispute of one claim at the dependency digest
// the claim recorded. The claim record itself is never rewritten.
choices!(ClaimDispositionV1 {
    ConfirmedForScope,
    Disputed
});
dto!(ReconciliationDispositionV1 {
    id: RecordId,
    claim: RecordId,
    dependency_digest: Digest,
    disposition: ClaimDispositionV1,
    reason: String
});
dto!(OrganizationRecordV1 { id:RecordId, version:u32, kind:OrganizationKind, targets:Vec<RecordId>, parent:Option<RecordId>, text:String, allocations:Vec<EvidenceAllocationV1>, supersedes:Option<RecordId> });
dto!(RuleV1 { id:RecordId, version:u32, operation_kinds:Vec<DecisionKind>, accounts:Vec<AccountId>, conditions:Vec<ReportedFieldV1>, grant_id:GrantId, revoked:bool });
dto!(ReviewRecordV1 { id:RecordId, targets:Vec<RecordId>, disposition:ReviewDisposition, reason:String });
dto!(ScenarioV1 { id:RecordId, version:u32, basis:ReadBasis, commitments:Vec<CommitmentId>, assumptions:Vec<String> });
dto!(ForecastPointV1 { account_id:AccountId, at:TemporalValue, amount:SourceValue<Amount>, uncertainty:Vec<String> });
dto!(DecisionEnvelopeV1 { id:RecordId, commit:CommitSeq, item_position:u32, decision_type:DecisionKind, schema_version:u32, actor:RecordId, authority:AuthorityKind, grant_id:Option<GrantId>, command_id:CommandId, command_digest:Digest, references:Vec<RecordId>, recorded_time:RecordedInstant, adopted_result:Vec<RecordId>, policy:PolicyReferenceV1, corrections:Vec<RecordId>, supersedes:Vec<RecordId> });
tagged!(AuthoritativeRecordV1 {
 DateConversionPolicy(DateConversionPolicyV1), FrozenAccountingManifest(FrozenAccountingManifestV1), Dataset(DatasetDefinitionV1), DatasetPolicy(DatasetPolicyV1), UnitCatalog(UnitCatalogV1), AccountComponent(AccountComponentV1), AccountAlias(AccountAliasV1), OwnershipPerimeter(OwnershipPerimeterV1), ImportReceipt(ImportReceiptV1), ObservationVersion(ObservationVersionV1), ParseProduct(ParseProductV1), AddressedParseProduct(AddressedParseProductV1), FinancialEvent(FinancialEventV1), FinancialLeg(FinancialLegV1), Association(AssociationV1), Allocation(EvidenceAllocationV1), Decision(DecisionEnvelopeV1), Recognition(RecognitionV1), RecognitionCorrectionRevision(RecognitionCorrectionRevisionV1), AccountingSegment(AccountingSegmentV1), SegmentReemission(SegmentReemissionV1), ReconciliationClaim(ReconciliationClaimV1), ReconciliationDisposition(ReconciliationDispositionV1), Organization(OrganizationRecordV1), Rule(RuleV1), Review(ReviewRecordV1), Commitment(CommitmentV1), OccurrenceMatch(OccurrenceMatchV1), Scenario(ScenarioV1), ForecastRun(ForecastRunV1), AccountCatalogRevision(AccountCatalogRevisionV1), ActivityAdoption(ActivityAdoptionV1), PendingExpiry(PendingExpiryV1), AssociationRevision(AssociationRevisionV1), RelationshipCapture(RelationshipCaptureV1), RelationshipParseRevision(RelationshipParseRevisionV1), Security(SecurityV1), Position(PositionV1), Trade(TradeV1), DatasetPolicyRevision(DatasetPolicyRevisionV1), HoldingsSnapshot(HoldingsSnapshotV1)
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
// `remapping` holds the owner's successor version of each organization record
// on a leg the correction retires.
dto!(CorrectionV1 { active_recognition:RecognitionId, replacement:Vec<RecognitionV1>, remapping:Vec<OrganizationRecordV1>, reason:String });
dto!(NoEffectV1 { reason: String });
tagged!(AccountingIntent { MetadataDeclaration(MetadataDeclarationV1), Recognition(RecognitionV1), Recognitions(Vec<RecognitionV1>), Correction(CorrectionV1), Corrections(Vec<CorrectionV1>), NoEffect(NoEffectV1) });
dto!(DecisionPlan { records:Vec<AuthoritativeRecordV1>, invalidations:Vec<RecordId>, accounting:AccountingIntent });
/// A nonauthoritative in-memory replay index. It deliberately has no wire/history serialization.
#[derive(Clone, Debug, Default)]
pub struct DomainState {
    /// Exact accepted ordered prefix, not a hash of the lossy latest-ID index.
    pub domain_digest: Digest,
    /// Reconstructed exact-version governing policies at this processed prefix.
    pub dataset_policies: BTreeMap<(RecordId, u32), DatasetPolicyV1>,
    pub accounting: Option<AccountingIndexV1>,
    pub records: BTreeMap<RecordId, AuthoritativeRecordV1>,
    pub through: CommitSeq,
    /// Every record a committed result invalidated through this prefix.
    pub invalidated: BTreeSet<RecordId>,
    /// The commit that first recorded each import receipt through this
    /// prefix. Receipts compare in this order, never by their received time.
    pub receipt_commits: BTreeMap<RecordId, CommitSeq>,
}
impl DomainState {
    /// The lifecycle status a claim's record holds at this prefix.
    pub fn claim_status(&self, id: RecordId) -> Option<ReconciliationStatus> {
        match self.records.get(&id) {
            Some(AuthoritativeRecordV1::ReconciliationClaim(claim)) => Some(claim.status),
            _ => None,
        }
    }
    /// A claim is invalidated from the commit whose result lists it.
    pub fn claim_invalidated(&self, id: RecordId) -> bool {
        self.claim_status(id).is_some() && self.invalidated.contains(&id)
    }
}
impl Schema for Health {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"string","enum":["healthy","source_partial","source_stale","locked","projection_lagging","integrity_failed","migration_required","recovery_in_progress"]})
    }
}
