//! Stable command bodies never contain live authorization or accepted identities/times.
use currents_types::{schema::Schema, *};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolVersion;
impl Serialize for ProtocolVersion {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(1)
    }
}
impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if u32::deserialize(d)? != 1 {
            return Err(serde::de::Error::custom("unsupported protocol version"));
        }
        Ok(Self)
    }
}
impl Schema for ProtocolVersion {
    fn schema() -> serde_json::Value {
        serde_json::json!({"const":1})
    }
}
dto!(SetAccountCatalogV1 { account_id:AccountId, component_id:RecordId, account_class:AccountClass, unit_id:UnitId, normal_sign:NormalSign, aliases:Vec<SourceAliasV1>, lifecycle:Lifecycle, display_name:String });
dto!(SetDatasetPolicyV1 {
    policy: DatasetPolicyV1
});
dto!(ManualPositionV1 {
    account_id: AccountId,
    amount: Amount,
    as_of: TemporalValue,
    source: String,
    coverage: CoverageV1,
    reason: String
});
dto!(LoanPaymentV1 { account_id:AccountId, cash_account_id:AccountId, principal:Amount, interest:Amount, fees:Vec<Amount>, effective:TemporalValue, posting:TemporalValue, evidence:Vec<RecordId> });
dto!(PublishImportV1 { staged_batch_digest:Digest, receipt:RecordId, checkpoint:SourceValue<String> });
dto!(AdoptInterpretationV1 { targets:Vec<RecordId>, interpretation:InterpretationV1 });
dto!(RecognitionRequestV1 { leg_ids:Vec<LegId>, facet_allocations:Vec<EvidenceAllocationV1>, postings:Vec<PostingV1>, native_dates:Vec<TemporalValue>, interpretation:InterpretationV1, authority_basis:PolicyReferenceV1 });
dto!(ReconciliationRequestV1 {
    claim: RecordId,
    basis: ReadBasis,
    dependency_digest: Digest
});
dto!(MaintenanceV1 { basis:ReadBasis, destination_handle:Option<RecordId>, target_version:Option<u32> });
tagged!(OperationV1 {
 SetAccountCatalog(SetAccountCatalogV1), SetDatasetPolicy(SetDatasetPolicyV1), RecordManualPosition(ManualPositionV1), RecordLoanPayment(LoanPaymentV1), PublishImport(PublishImportV1), AdoptInterpretation(AdoptInterpretationV1), SetOrganization(OrganizationRecordV1), SetReviewDisposition(ReviewRecordV1), Recognize(RecognitionRequestV1), CorrectRecognition(CorrectionV1), ReplaceAccountingSegment(AccountingSegmentV1), AssociateLegs(AssociationV1), EvaluateReconciliation(ReconciliationRequestV1), ConfirmReconciliation(ReconciliationRequestV1), SetCommitment(CommitmentV1), SetOccurrenceMatch(OccurrenceMatchV1), SaveForecast(ForecastRunV1), Export(MaintenanceV1), Backup(MaintenanceV1), Verify(MaintenanceV1), Rebuild(MaintenanceV1), Migrate(MaintenanceV1)
});
dto!(CommandBodyV1 {
    protocol_version: ProtocolVersion,
    dataset_id: DatasetId,
    command_id: CommandId,
    expected_head: CommitSeq,
    operation: OperationV1
});
/// Intentionally neither Serialize nor Debug: bearer bytes cannot enter a history DTO.
/// ```compile_fail
/// fn cannot_record(auth: currents_protocol::AuthorizationEnvelope) {
///     let _ = currents_protocol::canonical_bytes(&auth);
/// }
/// ```
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationEnvelope {
    pub session: SessionId,
    pub capability: String,
    pub approval: Option<String>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", content = "value", deny_unknown_fields)]
pub enum RequestV1 {
    Preview(CommandBodyV1),
    Submit {
        body: CommandBodyV1,
        authorization: AuthorizationEnvelope,
    },
    Query(QueryV1),
    GetCommandResult {
        dataset_id: DatasetId,
        command_id: CommandId,
        digest: Digest,
    },
    Cancel {
        command_id: CommandId,
    },
}
choices!(QueryKindV1 {
    QueryPosition,
    QueryHistory,
    Explain,
    QueryReview,
    RunForecast
});
dto!(QueryV1 { kind:QueryKindV1, basis:ReadBasis, accounts:Vec<AccountId>, records:Vec<RecordId> });
dto!(ApprovalScopeV1 { accounts:Vec<AccountId>, operation:DecisionKind, fields:Vec<String> });
dto!(PreviewV1 { digest:Digest, expected_head:CommitSeq, effects:EffectsV1, warnings:Vec<ErrorClass>, approval_scope:ApprovalScopeV1, closure_digest:Digest });
tagged!(QueryOutputV1 { Position(Vec<ManualPositionV1>), History(Vec<AuthoritativeRecordV1>), Explanation(Vec<DecisionEnvelopeV1>), Review(Vec<ReviewRecordV1>), Forecast(ForecastRunV1) });
dto!(QueryResponseV1 { processed_head:CommitSeq, basis:ReadBasis, health:Vec<Health>, output:QueryOutputV1 });
/// Host-supplied context. There is intentionally no Deserialize implementation.
pub struct RequestContext {
    pub authenticated_client: RecordId,
    pub session: SessionId,
    pub grant_id: GrantId,
    pub instance_id: InstanceId,
}
/// Host adapter, not an ambient clock and not a caller timestamp.
pub trait TrustedClock {
    fn now(&self) -> currents_types::time::OffsetDateTime;
}
impl Schema for AuthorizationEnvelope {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"object","additionalProperties":false,"required":["session","capability"],"properties":{"session":SessionId::schema(),"capability":String::schema(),"approval":Option::<String>::schema()}})
    }
}
impl Schema for RequestV1 {
    fn schema() -> serde_json::Value {
        use serde_json::json;
        let variant = |kind: &str, value: serde_json::Value| json!({"type":"object","additionalProperties":false,"required":["kind","value"],"properties":{"kind":{"const":kind},"value":value}});
        json!({"oneOf":[
            variant("Preview",CommandBodyV1::schema()),
            variant("Submit",json!({"type":"object","additionalProperties":false,"required":["body","authorization"],"properties":{"body":CommandBodyV1::schema(),"authorization":AuthorizationEnvelope::schema()}})),
            variant("Query",QueryV1::schema()),
            variant("GetCommandResult",json!({"type":"object","additionalProperties":false,"required":["dataset_id","command_id","digest"],"properties":{"dataset_id":DatasetId::schema(),"command_id":CommandId::schema(),"digest":Digest::schema()}})),
            variant("Cancel",json!({"type":"object","additionalProperties":false,"required":["command_id"],"properties":{"command_id":CommandId::schema()}}))
        ]})
    }
}
