//! Versioned C05 instructions, with no storage or native-kernel types.
use crate::*;
use std::collections::BTreeMap;
choices!(BookkeepingFallbackV1 {
    RequirePosting,
    Effective,
    Explicit
});
dto!(FixedOffsetV1 { offset: String });
dto!(ResolvedInstantV1 {
    utc: String,
    zone: String,
    data_version: String,
    offset: String
});
tagged!(InstantConversionV1 { FixedOffset(FixedOffsetV1), Resolved(Vec<ResolvedInstantV1>) });
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "value", deny_unknown_fields)]
pub enum DateConversionV1 {
    CivilOnly,
    Instants(InstantConversionV1),
}
impl schema::Schema for DateConversionV1 {
    fn schema() -> serde_json::Value {
        serde_json::json!({"oneOf":[{"type":"object","additionalProperties":false,"required":["kind"],"properties":{"kind":{"const":"CivilOnly"}}},{"type":"object","additionalProperties":false,"required":["kind","value"],"properties":{"kind":{"const":"Instants"},"value":<InstantConversionV1 as schema::Schema>::schema()}}]})
    }
}
dto!(DateConversionPolicyV1 {
    id: RecordId,
    version: u32,
    conversion: DateConversionV1,
    bookkeeping: BookkeepingFallbackV1
});
dto!(AccountingPrefixV1 {
    through: CommitSeq,
    domain_digest: Digest,
    native_head: NativeHead
});
dto!(NativeAccountMappingV1 {
    account_id: AccountId,
    unit_id: UnitId,
    account_class: AccountClass,
    path: String
});
dto!(FrozenPostingV1 {
    original: PostingV1,
    native_coefficient: String,
    path: String,
    unit: UnitDefinition
});
dto!(FrozenEntryV1 {recognition_id:RecognitionId,key:String,booking_date:String,value_date:String,postings:Vec<FrozenPostingV1>,date_policy:DateConversionPolicyV1,generated_document:Digest,reverses:Option<String>});
dto!(NativeCalendarV1 {
    start: String,
    end: String
});
dto!(NativePolicyV1 { effective: String });
tagged!(FrozenNativeOperationV1 {Policy(NativePolicyV1),Calendar(NativeCalendarV1),Unit(UnitDefinition),Account(NativeAccountMappingV1),Entry(FrozenEntryV1)});
dto!(AccountingExpectationsV1 {
    native_operation_count: CommitSeq,
    financial_entry_count: CommitSeq
});
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountingBatchV1 {
    pub schema_version: u32,
    pub native_semantics: u32,
    pub operations: Vec<FrozenNativeOperationV1>,
    pub preconditions: AccountingPrefixV1,
    pub decision_digest: Digest,
    pub result_expectations: AccountingExpectationsV1,
}
impl schema::Schema for AccountingBatchV1 {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"object","additionalProperties":false,"required":["schema_version","native_semantics","operations","preconditions","decision_digest","result_expectations"],"properties":{"schema_version":{"const":1},"native_semantics":{"const":1},"operations":{"type":"array","maxItems":20000,"items":FrozenNativeOperationV1::schema()},"preconditions":AccountingPrefixV1::schema(),"decision_digest":Digest::schema(),"result_expectations":AccountingExpectationsV1::schema()}})
    }
}
dto!(FrozenAccountingManifestV1 {
    id: RecordId,
    schema_version: u32,
    native_semantics: u32,
    instructions: ContentReferenceV1,
    preconditions: AccountingPrefixV1,
    decision_digest: Digest,
    result_expectations: AccountingExpectationsV1
});
/// Derived cache only. Rebuild from verified ordered manifests, never persist as authority.
#[derive(Clone, Debug)]
pub struct AccountingIndexV1 {
    pub prefix: AccountingPrefixV1,
    pub calendar_declared: bool,
    pub date_policies: BTreeMap<(RecordId, u32), DateConversionPolicyV1>,
    pub units: BTreeMap<UnitId, UnitDefinition>,
    pub accounts: BTreeMap<AccountId, NativeAccountMappingV1>,
    pub recognitions: BTreeMap<RecognitionId, Vec<FrozenEntryV1>>,
    pub reversed: std::collections::BTreeSet<RecognitionId>,
}
impl AccountingIndexV1 {
    pub fn empty(native_head: NativeHead) -> Self {
        Self {
            prefix: AccountingPrefixV1 {
                through: CommitSeq(0),
                domain_digest: Digest([0; 32]),
                native_head,
            },
            calendar_declared: false,
            date_policies: BTreeMap::new(),
            units: BTreeMap::new(),
            accounts: BTreeMap::new(),
            recognitions: BTreeMap::new(),
            reversed: std::collections::BTreeSet::new(),
        }
    }
}
