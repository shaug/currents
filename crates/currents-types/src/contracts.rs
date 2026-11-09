//! Shared domain-facing command and query shapes. Execution and validation belong to subsystem owners.
use crate::{schema::Schema, *};

// Null means unchanged; a present change records both its explicit source state and adopted basis.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyChangeV1<T> {
    pub value: SourceValue<T>,
    pub basis: ReadBasis,
}
impl<T: Schema> Schema for PolicyChangeV1<T> {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"object","additionalProperties":false,"required":["value","basis"],"properties":{"value":SourceValue::<T>::schema(),"basis":ReadBasis::schema()}})
    }
}
dto!(FreshnessThresholdV1 {
    source: RecordId,
    seconds: CommitSeq
});
dto!(TimezonePolicyV1 { zone:SourceValue<String>, timezone_data_version:String, calendar_version:u32 });
dto!(SetDatasetPolicyV1 { policy_id:RecordId, prior_version:Option<u32>, effective_scope:DateScope, ownership_perimeter:Option<PolicyChangeV1<Vec<OwnershipV1>>>, source_priority:Option<PolicyChangeV1<Vec<RecordId>>>, freshness_thresholds:Option<PolicyChangeV1<Vec<FreshnessThresholdV1>>>, timezone:Option<PolicyChangeV1<TimezonePolicyV1>>, reporting_currency:Option<PolicyChangeV1<UnitId>> });
dto!(SetAccountCatalogV1 { account_id:AccountId, component_id:RecordId, account_class:AccountClass, unit_id:UnitId, normal_sign:NormalSign, aliases:Vec<SourceAliasV1>, lifecycle:Lifecycle, display_name:String });
dto!(OwnershipShareV1 {
    numerator: CommitSeq,
    denominator: CommitSeq
});
dto!(ManualPositionV1 { component:RecordId, amount:Amount, as_of:TemporalValue, method:String, evidence:Vec<RecordId>, ownership_share:OwnershipShareV1 });
dto!(LoanPaymentV1 { cash:PostingV1, principal:SourceValue<PostingV1>, interest:SourceValue<PostingV1>, fees:Vec<PostingV1>, unknown_allocation:SourceValue<Amount>, source:SourceIdentityV1 });
dto!(PublishImportV1 { staged_batch_digest:Digest, receipt:RecordId, checkpoint:SourceValue<String> });
dto!(AdoptInterpretationV1 { targets:Vec<RecordId>, interpretation:InterpretationV1 });
dto!(RecognitionRequestV1 { leg_ids:Vec<LegId>, facet_allocations:Vec<EvidenceAllocationV1>, postings:Vec<PostingV1>, native_dates:Vec<TemporalValue>, interpretation:InterpretationV1, authority_basis:PolicyReferenceV1 });
dto!(CorrectionRequestV1 { active_recognition:RecognitionId, replacement:Vec<RecognitionV1>, remapping:Vec<AssociationV1>, reason:String });
dto!(SegmentRequestV1 { component:RecordId, prior_segment:Option<RecordId>, opening:RecognitionId, included_activity:Vec<RecognitionId>, replacement_closure:Vec<RecognitionId>, approved_accounts:Vec<AccountId>, residual_disposition:SourceValue<String> });
dto!(AssociationRequestV1 { kind:RelationshipKind, legs:Vec<LegId>, facets:Vec<RecordId>, allocations:Vec<EvidenceAllocationV1>, survivor:Option<LegId>, corrections:Vec<RecognitionId>, reason:String });
choices!(OverrideActionV1 {
    Preserve,
    Replace,
    Clear
});
tagged!(OrganizationValueV1 { Reference(RecordId), Text(String) });
dto!(OrganizationRequestV1 { economic_ids:Vec<RecordId>, field:OrganizationKind, value:SourceValue<OrganizationValueV1>, allocation:Vec<EvidenceAllocationV1>, provenance:InterpretationV1, override_action:OverrideActionV1 });
dto!(ReviewDispositionV1 { targets:Vec<RecordId>, disposition:ReviewDisposition, reason:String });
choices!(ReconciliationKindV1 {
    Balance,
    Activity,
    Relationship,
    Holdings
});
dto!(ReconciliationRequestV1 { kind:ReconciliationKindV1, scope:AccountScope, cutoff:TemporalValue, evidence:Vec<RecordId>, exclusions:Vec<RecordId>, basis:ReadBasis });
dto!(ConfirmReconciliationV1 {
    claim: RecordId,
    digest: Digest
});
dto!(MaintenanceV1 { basis:ReadBasis, destination_handle:Option<RecordId>, target_version:Option<u32> });
// C09 outputs preserve known movements, anchoring and uncertainty independently.
dto!(AccountScope { accounts:Vec<AccountId>, components:Vec<RecordId> });
dto!(AccountMovementV1 { account:AccountId, component:RecordId, amount:Amount, effective:TemporalValue, posting:TemporalValue, evidence:Vec<RecordId> });
dto!(PositionAssertionV1 {
    id: RecordId,
    component: RecordId,
    amount: Amount,
    as_of: TemporalValue,
    source: SourceIdentityV1,
    coverage: CoverageV1
});
dto!(AnchoredBalanceV1 { component:RecordId, opening:RecordId, amount:SourceValue<Amount>, through:TemporalValue, dependencies:Vec<RecordId> });
choices!(FreshnessStatus {
    Fresh,
    Stale,
    Unknown
});
dto!(FreshnessV1 {
    source: RecordId,
    status: FreshnessStatus,
    as_of: TemporalValue,
    threshold_seconds: CommitSeq
});
dto!(AggregateComponentV1 { component:RecordId, amount:SourceValue<Amount>, evidence:Vec<RecordId> });
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "value", deny_unknown_fields)]
pub enum AggregateV1 {
    Exact(Amount),
    Range {
        low: Amount,
        high: Amount,
    },
    Indeterminate {
        components: Vec<AggregateComponentV1>,
        obstructions: Vec<String>,
    },
}
impl Schema for AggregateV1 {
    fn schema() -> serde_json::Value {
        use serde_json::json;
        let v = |kind: &str, value: serde_json::Value| json!({"type":"object","additionalProperties":false,"required":["kind","value"],"properties":{"kind":{"const":kind},"value":value}});
        json!({"oneOf":[v("Exact",Amount::schema()),v("Range",json!({"type":"object","additionalProperties":false,"required":["low","high"],"properties":{"low":Amount::schema(),"high":Amount::schema()}})),v("Indeterminate",json!({"type":"object","additionalProperties":false,"required":["components","obstructions"],"properties":{"components":Vec::<AggregateComponentV1>::schema(),"obstructions":Vec::<String>::schema()}}))]})
    }
}
dto!(PositionResponseV1 { basis:ReadBasis, scope:AccountScope, assertions:Vec<PositionAssertionV1>, movements:Vec<AccountMovementV1>, anchored_balance:Vec<AnchoredBalanceV1>, selected_basis:Vec<RecordId>, pending:Vec<AccountMovementV1>, freshness:Vec<FreshnessV1>, coverage:Vec<CoverageV1>, discrepancies:Vec<ReconciliationClaimV1>, missing_inputs:Vec<String>, aggregate:AggregateV1 });
dto!(HistoryFilterV1 { accounts:Vec<AccountId>, parties:Vec<RecordId>, categories:Vec<RecordId>, tags:Vec<RecordId>, dates:DateScope, page_cursor:Option<String> });
dto!(HistoryItemV1 { leg:LegId, account:AccountId, amount:SourceValue<Amount>, effective:TemporalValue, posting:TemporalValue, organization:Vec<RecordId>, evidence:Vec<RecordId> });
dto!(IncompleteBucketV1 { accounts:Vec<AccountId>, scope:DateScope, missing_inputs:Vec<String> });
dto!(HistoryResponseV1 { basis:ReadBasis, items:Vec<HistoryItemV1>, unknown_date_bucket:Vec<HistoryItemV1>, incomplete_buckets:Vec<IncompleteBucketV1>, next_cursor:Option<String> });
choices!(ExplanationNodeKindV1 {
    Evidence,
    Decision,
    Policy
});
dto!(ExplanationNodeV1 {
    id: RecordId,
    kind: ExplanationNodeKindV1,
    summary: String
});
dto!(ExplanationEdgeV1 {
    from: RecordId,
    to: RecordId,
    relationship: String
});
dto!(ExplanationV1 { basis:ReadBasis, roots:Vec<RecordId>, nodes:Vec<ExplanationNodeV1>, edges:Vec<ExplanationEdgeV1> });
