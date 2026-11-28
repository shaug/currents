//! Complete C08 commands. Existing legacy command and record encodings are unchanged.
use crate::*;
choices!(ControlRoleV1 {
    UnresolvedEquity,
    OpeningEquity,
    IncomeFlow,
    ExpenseFlow,
    Clearing,
    TaxPrepayment,
    QuantityControl,
    InvestmentCarrying,
    DisposalClearing
});
dto!(InternalAccountV1 {
    role: ControlRoleV1,
    created_at: TemporalValue
});
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "value", deny_unknown_fields)]
pub enum AccountOriginV1 {
    External,
    Internal(InternalAccountV1),
}
impl schema::Schema for AccountOriginV1 {
    fn schema() -> serde_json::Value {
        serde_json::json!({"oneOf":[{"type":"object","additionalProperties":false,"required":["kind"],"properties":{"kind":{"const":"External"}}},{"type":"object","additionalProperties":false,"required":["kind","value"],"properties":{"kind":{"const":"Internal"},"value":<InternalAccountV1 as schema::Schema>::schema()}}]})
    }
}
dto!(CatalogRequestV1 { schema_version:u32, revision_id:RecordId, prior_revision:Option<RecordId>, catalog:SetAccountCatalogV1, ownership:OwnershipV1, origin:AccountOriginV1 });
dto!(AccountCatalogRevisionV1 { id:RecordId, schema_version:u32, component_id:RecordId, account_id:AccountId, prior_revision:Option<RecordId>, catalog:SetAccountCatalogV1, origin:AccountOriginV1 });
dto!(SourceFacetBindingV1 {
    observation: RecordId,
    parse_product: RecordId,
    facet: RecordId,
    amount: Amount
});
dto!(AdoptedLegV1 { leg:FinancialLegV1, sources:Vec<SourceFacetBindingV1> });
dto!(AdoptedFacetV1 {
    leg_id: LegId,
    source: SourceFacetBindingV1
});
choices!(ActivityStatusV1 { Pending, Posted });
dto!(OpeningBasisV1 { segment_id:RecordId, component_id:RecordId, amount:Amount, cutoff:TemporalValue, evidence:SourceFacetBindingV1, included_activity:Vec<SourceFacetBindingV1> });
dto!(RecognitionTermsV1 { recognition_id:RecognitionId, lineage:RecordId, postings:Vec<PostingV1>, bookkeeping_date:TemporalValue, date_policy:PolicyReferenceV1, unresolved_account:Option<AccountId>, opening:Option<OpeningBasisV1> });
dto!(ActivityRequestV1 { schema_version:u32, adoption_id:RecordId, event:FinancialEventV1, legs:Vec<AdoptedLegV1>, status:ActivityStatusV1, prior_pending:Option<RecordId>, interpretation:InterpretationV1, governing_policy:PolicyReferenceV1, financial:Option<RecognitionTermsV1> });
dto!(ActivityAdoptionV1 { id:RecordId, schema_version:u32, event_id:RecordId, legs:Vec<LegId>, facets:Vec<AdoptedFacetV1>, status:ActivityStatusV1, prior_pending:Option<RecordId>, recognition:Option<RecognitionId>, interpretation:InterpretationV1, governing_policy:PolicyReferenceV1, opening:Option<OpeningBasisV1>, unresolved_account:Option<AccountId> });

// Explicit owner disposition. Historic pending evidence remains immutable.
dto!(PendingExpiryV1 {
    id: RecordId,
    schema_version: u32,
    pending_adoption: RecordId,
    interpretation: InterpretationV1,
    governing_policy: PolicyReferenceV1
});
