//! Structural correction transport only; no service route is activated here.
use currents_protocol::{CommandBodyV1, canonical_bytes, command_digest, decode};
use serde_json::{Value, json};

fn legacy() -> Value {
    serde_json::from_slice::<Vec<Value>>(include_bytes!(
        "../../../fixtures/contracts/v1/operations.json"
    ))
    .unwrap()
    .into_iter()
    .find(|body| body["operation"]["kind"] == "CorrectRecognition")
    .unwrap()
}

fn complete() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../../fixtures/contracts/v1/correction-complete.json"
    ))
    .unwrap()
}

fn decode_body(value: &Value) -> CommandBodyV1 {
    decode(&serde_json::to_vec(value).unwrap()).unwrap()
}

#[test]
fn complete_correction_commits_caller_ids_policy_and_exact_basis_without_changing_legacy() {
    let value = complete();
    let body = decode_body(&value);
    let bytes = canonical_bytes(&body).unwrap();
    assert_eq!(
        serde_json::to_value(decode::<CommandBodyV1>(&bytes).unwrap()).unwrap(),
        value
    );
    let digest = command_digest(&body).unwrap();
    assert_eq!(
        serde_json::to_value(digest).unwrap(),
        json!("c923f6687aefb23c11405056eb0f62eb72a1e16f339968031118ff6066f5d258")
    );
    for (path, changed) in [
        (
            "/operation/value/revision_id",
            json!("00000000-0000-0000-0000-000000000030"),
        ),
        ("/operation/value/governing_policy/version", json!(2)),
        (
            "/operation/value/expected_basis/domain_digest",
            json!("33".repeat(32)),
        ),
        (
            "/operation/value/expected_basis/native_head/checkpoint",
            json!("44".repeat(32)),
        ),
        ("/expected_head", json!("2")),
    ] {
        let mut modified = value.clone();
        *modified.pointer_mut(path).unwrap() = changed;
        assert_ne!(
            command_digest(&decode_body(&modified)).unwrap(),
            digest,
            "{path}"
        );
    }
    let old = decode_body(&legacy());
    let old_bytes = canonical_bytes(&old).unwrap();
    assert_eq!(old_bytes, br#"{"command_id":"00000000-0000-0000-0000-000000000001","dataset_id":"00000000-0000-0000-0000-000000000001","expected_head":"1","operation":{"kind":"CorrectRecognition","value":{"active_recognition":"00000000-0000-0000-0000-000000000001","reason":"synthetic","remapping":[],"replacement":[]}},"protocol_version":1}"#);
}

#[test]
fn complete_correction_has_no_implicit_revision_basis_or_policy() {
    let value = complete();
    for field in [
        "schema_version",
        "revision_id",
        "governing_policy",
        "expected_basis",
        "active_recognition",
        "replacement",
        "reason",
    ] {
        let mut missing = value.clone();
        missing["operation"]["value"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            decode::<CommandBodyV1>(&serde_json::to_vec(&missing).unwrap()).is_err(),
            "{field}"
        );
    }
    let mut unknown = value.clone();
    unknown["operation"]["value"]["remapping"] = json!([]);
    assert!(decode::<CommandBodyV1>(&serde_json::to_vec(&unknown).unwrap()).is_err());
    let mut future = value;
    future["operation"]["value"]["schema_version"] = json!(2);
    assert!(decode::<CommandBodyV1>(&serde_json::to_vec(&future).unwrap()).is_err());
}

#[test]
fn complete_segment_commits_authority_basis_drafts_and_reemissions() {
    let value: Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/contracts/v1/segment-complete.json"
    ))
    .unwrap();
    let body = decode_body(&value);
    let bytes = canonical_bytes(&body).unwrap();
    assert_eq!(
        serde_json::to_value(decode::<CommandBodyV1>(&bytes).unwrap()).unwrap(),
        value
    );
    let digest = command_digest(&body).unwrap();
    assert_eq!(
        serde_json::to_value(digest).unwrap(),
        json!("a352cbf394fe9ef1d7a5cbe195bfe793b9d3e62aeb7abd3f322f7b539e13c94e")
    );
    let legacy = serde_json::from_slice::<Vec<Value>>(include_bytes!(
        "../../../fixtures/contracts/v1/operations.json"
    ))
    .unwrap()
    .into_iter()
    .find(|body| body["operation"]["kind"] == "ReplaceAccountingSegment")
    .unwrap();
    let legacy = decode_body(&legacy);
    assert_eq!(canonical_bytes(&legacy).unwrap(), br#"{"command_id":"00000000-0000-0000-0000-000000000001","dataset_id":"00000000-0000-0000-0000-000000000001","expected_head":"1","operation":{"kind":"ReplaceAccountingSegment","value":{"approved_accounts":[],"component":"00000000-0000-0000-0000-000000000001","included_activity":[],"opening":"00000000-0000-0000-0000-000000000001","prior_segment":null,"replacement_closure":[],"residual_disposition":{"state":"Missing"}}},"protocol_version":1}"#);
    for (path, changed) in [
        (
            "/operation/value/authority_id",
            json!("00000000-0000-0000-0000-000000000080"),
        ),
        (
            "/operation/value/expected_basis/domain_digest",
            json!("33".repeat(32)),
        ),
        (
            "/operation/value/reemissions/0/posting_ids/0",
            json!("00000000-0000-0000-0000-000000000081"),
        ),
        ("/operation/value/backfill", json!([])),
        ("/operation/value/segment/included_activity", json!([])),
        ("/expected_head", json!("2")),
    ] {
        let mut modified = value.clone();
        *modified.pointer_mut(path).unwrap() = changed;
        assert_ne!(
            command_digest(&decode_body(&modified)).unwrap(),
            digest,
            "{path}"
        );
    }
    for field in [
        "schema_version",
        "expected_basis",
        "authority_id",
        "segment",
        "opening",
        "backfill",
        "reemissions",
    ] {
        let mut missing = value.clone();
        missing["operation"]["value"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            decode::<CommandBodyV1>(&serde_json::to_vec(&missing).unwrap()).is_err(),
            "{field}"
        );
    }
    let mut incompatible = value.clone();
    incompatible["operation"]["value"]["schema_version"] = json!(2);
    assert!(decode::<CommandBodyV1>(&serde_json::to_vec(&incompatible).unwrap()).is_err());
    let mut unknown = value;
    unknown["operation"]["value"]["reemissions"][0]["unknown"] = json!(true);
    assert!(decode::<CommandBodyV1>(&serde_json::to_vec(&unknown).unwrap()).is_err());
}
