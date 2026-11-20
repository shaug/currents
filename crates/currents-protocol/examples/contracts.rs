//! Reproducible schema inventory: cargo run --locked -p currents-protocol --example contracts
use currents_protocol::*;
use currents_types::{schema::Schema, *};
use serde_json::{Value, json};
fn main() {
    let mut schemas = serde_json::json!({
     "$schema":"https://json-schema.org/draft/2020-12/schema",
     "$id":"https://github.com/shaug/currents/schemas/v1/contracts.json",
     "$defs":{
      "CatalogRequestV1":CatalogRequestV1::schema(), "ActivityRequestV1":ActivityRequestV1::schema(), "AccountCatalogRevisionV1":AccountCatalogRevisionV1::schema(), "ActivityAdoptionV1":ActivityAdoptionV1::schema(), "PendingExpiryV1":PendingExpiryV1::schema(), "CompleteAssociationRequestV1":CompleteAssociationRequestV1::schema(), "CompleteCorrectionRequestV1":CompleteCorrectionRequestV1::schema(), "CompleteSegmentRequestV1":CompleteSegmentRequestV1::schema(), "AssociationRevisionV1":AssociationRevisionV1::schema(), "RelationshipCaptureV1":RelationshipCaptureV1::schema(), "RelationshipParseRevisionV1":RelationshipParseRevisionV1::schema(), "RelationshipCorrectionProfileV1":RelationshipCorrectionProfileV1::schema(),
      "AccountingBatchV1":AccountingBatchV1::schema(), "FrozenAccountingManifestV1":FrozenAccountingManifestV1::schema(), "DateConversionPolicyV1":DateConversionPolicyV1::schema(),
      "AggregateV1":AggregateV1::schema(), "HistoryResponseV1":HistoryResponseV1::schema(), "ExplanationV1":ExplanationV1::schema(), "RequestV1":RequestV1::schema(), "Amount":Amount::schema(), "TemporalValue":TemporalValue::schema(),
      "SourceValue":SourceValue::<String>::schema(), "ReadBasis":ReadBasis::schema(),
      "AuthoritativeRecordV1":AuthoritativeRecordV1::schema(), "DomainCommitV1":DomainCommitV1::schema(),
      "CommandBodyV1":CommandBodyV1::schema(), "CommandResultV1":CommandResultV1::schema(),
      "PreviewV1":PreviewV1::schema(), "QueryV1":QueryV1::schema(), "QueryResponseV1":QueryResponseV1::schema(),
      "DecisionPlan":DecisionPlan::schema(), "EngineError":EngineError::schema()
     }
    });
    // Only the new segment payload is referenced. Preserve existing operation
    // nodes exactly; otherwise each command/submit node duplicates both drafts.
    reference_complete_segment(&mut schemas);
    let segment = &mut schemas["$defs"]["CompleteSegmentRequestV1"]["properties"];
    let draft = json!({"$ref": "#/$defs/ActivityRequestV1"});
    assert_eq!(segment["opening"], ActivityRequestV1::schema());
    assert_eq!(segment["backfill"]["items"], ActivityRequestV1::schema());
    segment["opening"] = draft.clone();
    segment["backfill"]["items"] = draft;
    println!("{}", serde_json::to_string_pretty(&schemas).unwrap());
}

fn reference_complete_segment(node: &mut Value) {
    match node {
        Value::Array(items) => items.iter_mut().for_each(reference_complete_segment),
        Value::Object(fields) => {
            if let Some(properties) = fields.get_mut("properties").and_then(Value::as_object_mut)
                && properties.get("kind").and_then(|kind| kind.get("const"))
                    == Some(&json!("ReplaceAccountingSegmentComplete"))
            {
                let payload = properties.get_mut("value").expect("tagged segment payload");
                assert_eq!(*payload, CompleteSegmentRequestV1::schema());
                *payload = json!({"$ref": "#/$defs/CompleteSegmentRequestV1"});
                return;
            }
            fields.values_mut().for_each(reference_complete_segment);
        }
        _ => {}
    }
}
