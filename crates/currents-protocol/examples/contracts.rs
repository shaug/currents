//! Reproducible schema inventory: cargo run --locked -p currents-protocol --example contracts
use currents_protocol::*;
use currents_types::{schema::Schema, *};
fn main() {
    let schemas = serde_json::json!({
     "$schema":"https://json-schema.org/draft/2020-12/schema",
     "$id":"https://github.com/shaug/currents/schemas/v1/contracts.json",
     "$defs":{
      "RequestV1":RequestV1::schema(), "Amount":Amount::schema(), "TemporalValue":TemporalValue::schema(),
      "SourceValue":SourceValue::<String>::schema(), "ReadBasis":ReadBasis::schema(),
      "AuthoritativeRecordV1":AuthoritativeRecordV1::schema(), "DomainCommitV1":DomainCommitV1::schema(),
      "CommandBodyV1":CommandBodyV1::schema(), "CommandResultV1":CommandResultV1::schema(),
      "PreviewV1":PreviewV1::schema(), "QueryV1":QueryV1::schema(), "QueryResponseV1":QueryResponseV1::schema(),
      "DecisionPlan":DecisionPlan::schema()
     }
    });
    println!("{}", serde_json::to_string_pretty(&schemas).unwrap());
}
