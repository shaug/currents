//! Stable command bodies never contain live authorization or accepted identities/times.
pub use currents_types::contracts::*;
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
tagged!(OperationV1 {
 SetAccountCatalog(SetAccountCatalogV1), SetDatasetPolicy(SetDatasetPolicyV1), RecordManualPosition(ManualPositionV1), RecordLoanPayment(LoanPaymentV1), PublishImport(PublishImportV1), AdoptInterpretation(AdoptInterpretationV1), SetOrganization(OrganizationRequestV1), SetReviewDisposition(ReviewDispositionV1), Recognize(RecognitionRequestV1), CorrectRecognition(CorrectionRequestV1), ReplaceAccountingSegment(SegmentRequestV1), AssociateLegs(AssociationRequestV1), EvaluateReconciliation(ReconciliationRequestV1), ConfirmReconciliation(ConfirmReconciliationV1), SetCommitment(CommitmentV1), SetOccurrenceMatch(OccurrenceMatchV1), SaveForecast(ForecastRunV1), Export(MaintenanceV1), Backup(MaintenanceV1), Verify(MaintenanceV1), Rebuild(MaintenanceV1), Migrate(MaintenanceV1)
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
tagged!(QueryOutputV1 { Position(PositionResponseV1), History(HistoryResponseV1), Explanation(ExplanationV1), Review(Vec<ReviewRecordV1>), Forecast(ForecastRunV1) });
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
