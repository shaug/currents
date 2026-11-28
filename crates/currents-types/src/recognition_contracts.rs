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
/// A complete catalog request. Schema version 2 adds `overlaps`: the declared
/// components that a new total component, such as a brokerage total,
/// contains. A version-1 request has no `overlaps` field, and its bytes are
/// unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogRequestV1 {
    pub schema_version: u32,
    pub revision_id: RecordId,
    pub prior_revision: Option<RecordId>,
    pub catalog: SetAccountCatalogV1,
    pub ownership: OwnershipV1,
    pub origin: AccountOriginV1,
    pub overlaps: Vec<RecordId>,
}
dto!(CatalogRequestWireV1 { schema_version:u32, revision_id:RecordId, prior_revision:Option<RecordId>, catalog:SetAccountCatalogV1, ownership:OwnershipV1, origin:AccountOriginV1 });
dto!(CatalogRequestWireV2 { schema_version:u32, revision_id:RecordId, prior_revision:Option<RecordId>, catalog:SetAccountCatalogV1, ownership:OwnershipV1, origin:AccountOriginV1, overlaps:Vec<RecordId> });
impl serde::Serialize for CatalogRequestV1 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.schema_version == 1 {
            if !self.overlaps.is_empty() {
                return Err(serde::ser::Error::custom(
                    "a version-1 catalog request has no overlaps",
                ));
            }
            return CatalogRequestWireV1 {
                schema_version: self.schema_version,
                revision_id: self.revision_id,
                prior_revision: self.prior_revision,
                catalog: self.catalog.clone(),
                ownership: self.ownership.clone(),
                origin: self.origin.clone(),
            }
            .serialize(serializer);
        }
        CatalogRequestWireV2 {
            schema_version: self.schema_version,
            revision_id: self.revision_id,
            prior_revision: self.prior_revision,
            catalog: self.catalog.clone(),
            ownership: self.ownership.clone(),
            origin: self.origin.clone(),
            overlaps: self.overlaps.clone(),
        }
        .serialize(serializer)
    }
}
impl<'de> serde::Deserialize<'de> for CatalogRequestV1 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        // The version decides the shape: only version 1 omits `overlaps`.
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("schema_version") == Some(&serde_json::json!(1)) {
            let wire = CatalogRequestWireV1::deserialize(value).map_err(D::Error::custom)?;
            return Ok(Self {
                schema_version: wire.schema_version,
                revision_id: wire.revision_id,
                prior_revision: wire.prior_revision,
                catalog: wire.catalog,
                ownership: wire.ownership,
                origin: wire.origin,
                overlaps: vec![],
            });
        }
        let wire = CatalogRequestWireV2::deserialize(value).map_err(D::Error::custom)?;
        Ok(Self {
            schema_version: wire.schema_version,
            revision_id: wire.revision_id,
            prior_revision: wire.prior_revision,
            catalog: wire.catalog,
            ownership: wire.ownership,
            origin: wire.origin,
            overlaps: wire.overlaps,
        })
    }
}
impl schema::Schema for CatalogRequestV1 {
    fn schema() -> serde_json::Value {
        let mut first = CatalogRequestWireV1::schema();
        first["properties"]["schema_version"] = serde_json::json!({"const":1});
        let mut second = CatalogRequestWireV2::schema();
        second["properties"]["schema_version"] = serde_json::json!({"const":2});
        serde_json::json!({"oneOf":[first, second]})
    }
}
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
