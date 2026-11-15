//! Additive C08 canonical command goldens; historical command bytes stay unchanged.
use currents_protocol::{CommandBodyV1, canonical_bytes, command_digest, decode};
use currents_types::Digest;
use serde_json::Value;
#[test]
fn cur_013_complete_command_goldens() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/contracts/v1/recognition");
    let cases: Vec<Value> =
        serde_json::from_slice(&std::fs::read(directory.join("digests.json")).unwrap()).unwrap();
    for case in cases {
        let bytes = std::fs::read(directory.join(case["file"].as_str().unwrap())).unwrap();
        let body: CommandBodyV1 = decode(&bytes).unwrap();
        let canonical = canonical_bytes(&body).unwrap();
        assert_eq!(canonical, bytes.strip_suffix(b"\n").unwrap());
        let expected: Digest = serde_json::from_value(case["command_sha256"].clone()).unwrap();
        assert_eq!(command_digest(&body).unwrap(), expected);
        let mut missing: Value = serde_json::from_slice(&bytes).unwrap();
        missing["operation"]["value"]
            .as_object_mut()
            .unwrap()
            .remove("schema_version");
        assert!(decode::<CommandBodyV1>(&serde_json::to_vec(&missing).unwrap()).is_err());
        let mut unknown: Value = serde_json::from_slice(&bytes).unwrap();
        unknown["operation"]["value"]["inferred_authority"] = Value::Bool(true);
        assert!(decode::<CommandBodyV1>(&serde_json::to_vec(&unknown).unwrap()).is_err());
    }
    let legacy = include_bytes!("../../../fixtures/contracts/v1/command.json");
    let body: CommandBodyV1 = decode(legacy).unwrap();
    let value: Value = serde_json::from_slice(legacy).unwrap();
    assert_eq!(serde_json::to_value(body).unwrap(), value);
}
