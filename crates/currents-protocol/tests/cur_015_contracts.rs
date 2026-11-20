//! Structural command boundary only; economic admissibility is tested in domain.
use currents_protocol::{CommandBodyV1, canonical_bytes, decode};
use serde_json::{Value, json};
#[test]
fn complete_association_is_additive_and_rejects_incomplete_bindings() {
    let mut value: Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/contracts/v1/command.json"
    ))
    .unwrap();
    let id = "00000000-0000-0000-0000-000000000001";
    value["operation"] = json!({"kind":"AssociateLegsComplete","value":{
        "revision":{"id":id,"schema_version":1,"root":id,"prior_revision":null,"kind":"Transfer","legs":[],"allocations":[],"captured_allocations":[],"recognitions":[],"residuals":[],"superseded_adoptions":[],"survivor":null,"retired_recognitions":[],"interpretation":{"kind":"Interpretation","authority":"Owner","evidence":[],"reason":"synthetic structural decoding only"},"governing_policy":{"id":id,"version":1}},
        "events":[],"legs":[],"recognitions":[]
    }});
    let body: CommandBodyV1 = decode(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(decode::<CommandBodyV1>(&canonical_bytes(&body).unwrap()).unwrap())
            .unwrap(),
        value
    );
    let mut missing = value.clone();
    missing["operation"]["value"]["revision"]
        .as_object_mut()
        .unwrap()
        .remove("retired_recognitions");
    assert!(decode::<CommandBodyV1>(&serde_json::to_vec(&missing).unwrap()).is_err());
    let mut unknown = value;
    unknown["operation"]["value"]["implicit_survivor"] = json!(true);
    assert!(decode::<CommandBodyV1>(&serde_json::to_vec(&unknown).unwrap()).is_err());
    let legacy = include_bytes!("../../../fixtures/contracts/v1/command.json");
    let old: CommandBodyV1 = decode(legacy).unwrap();
    assert_eq!(
        serde_json::to_value(old).unwrap(),
        serde_json::from_slice::<Value>(legacy).unwrap()
    );
}
