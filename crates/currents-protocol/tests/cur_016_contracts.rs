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
