//! CUR-017 reconciliation claim records.
use currents_types::*;
use serde_json::{Value, json};

/// The claim golden vector from the contract fixtures.
fn golden() -> Value {
    let records: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/contracts/v1/records.json")).unwrap();
    records
        .into_iter()
        .find(|r| r["kind"] == "ReconciliationClaim")
        .expect("claim fixture")
}
fn canonical(value: &Value) -> Vec<u8> {
    serde_json_canonicalizer::to_vec(value).unwrap()
}
fn decode(value: &Value) -> serde_json::Result<AuthoritativeRecordV1> {
    serde_json::from_slice(&canonical(value))
}

#[test]
fn cur_017_claim_record_round_trip() {
    let id = json!("00000000-0000-0000-0000-000000000096");
    for kind in ["Balance", "Activity", "Relationship", "Holdings"] {
        for status in [
            "Proposed",
            "PartiallySupported",
            "ConfirmedForScope",
            "Disputed",
            "Superseded",
        ] {
            let mut record = golden();
            let value = &mut record["value"];
            value["kind"] = json!(kind);
            value["status"] = json!(status);
            value["accounting_dependencies"] = json!([id]);
            value["reasons"] = json!([
                "UnknownOpening",
                "IncompleteInterval",
                "IncompatibleBalanceKinds",
                "MissingDateMapping",
                "UnexplainedResidual"
            ]);
            let claim = decode(&record).unwrap();
            let encoded = serde_json::to_value(&claim).unwrap();
            assert_eq!(canonical(&encoded), canonical(&record), "{kind} {status}");
        }
    }

    // An unknown kind or status, or the old untyped comparison, fails.
    for (field, value) in [
        ("kind", "Cash"),
        ("status", "Matched"),
        ("status", "Invalidated"),
        ("reasons", "UnknownOpening"),
    ] {
        let mut record = golden();
        record["value"][field] = json!(value);
        assert!(decode(&record).is_err(), "{field} {value}");
    }
    let mut legacy = golden();
    let value = legacy["value"].as_object_mut().unwrap();
    value.remove("kind");
    value.insert("comparison_type".into(), json!("synthetic"));
    assert!(decode(&legacy).is_err());
    let mut both = golden();
    both["value"]["comparison_type"] = json!("synthetic");
    assert!(decode(&both).is_err());
}
