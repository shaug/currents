//! Public contract conformance, synthetic inputs only.
use std::{fs, path::PathBuf};
fn fixture(name: &str) -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/contracts/v1")
            .join(name),
    )
    .expect("CUR-002 must publish this versioned contract fixture")
}
#[test]
fn cur_002_1() {
    assert!(fixture("canonical.json").contains("utf16"));
    canonical_vectors();
}
#[test]
fn cur_002_2() {
    assert!(fixture("bounds.json").contains("scale19"));
    strict_bounds();
}
#[test]
fn cur_002_3() {
    assert!(fixture("temporal.json").contains("original_text"));
    temporal_precision();
}
#[test]
fn cur_002_4() {
    assert!(fixture("command.json").contains("expected_head"));
    digest_auth();
}

use currents_protocol::{
    AuthorizationEnvelope, CommandBodyV1, MAX_MESSAGE_BYTES, canonical_bytes, command_digest,
    decode,
};
use currents_types::{
    Amount, AuthoritativeRecordV1, CommitSeq, DecisionEnvelopeV1, SourceValue, TemporalValue,
    UnitId,
};
use serde_json::{Value, json};
fn amount(coefficient: i128, scale: u8) -> Amount {
    Amount::new(coefficient, scale, UnitId(uuid::Uuid::from_u128(1))).unwrap()
}
fn canonical_vectors() {
    let vectors: Value = serde_json::from_str(&fixture("canonical.json")).unwrap();
    let input = &vectors["utf16"]["input"];
    let bytes = canonical_bytes(input).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let mut last = 0;
    for key in vectors["utf16"]["expected_keys"].as_array().unwrap() {
        let needle = serde_json::to_string(key).unwrap();
        let position = text.find(&needle).unwrap();
        assert!(position >= last);
        last = position;
    }
    let expected = "{\"\\r\":\"Carriage Return\",\"1\":\"One\",\"\u{80}\":\"Control\",\"ö\":\"Latin Small Letter O With Diaeresis\",\"€\":\"Euro Sign\",\"😀\":\"Emoji: Grinning Face\",\"דּ\":\"Hebrew Letter Dalet With Dagesh\"}";
    assert_eq!(text, expected);
    assert_eq!(
        canonical_bytes(&vectors["escaping"]["input"]).unwrap(),
        vectors["escaping"]["expected"].as_str().unwrap().as_bytes()
    );
    assert_eq!(amount(12300, 4).decimal_string().unwrap(), "1.23");
    assert_eq!(amount(0, 18).decimal_string().unwrap(), "0");
    let zero: Amount = decode(
        br#"{"coefficient":"-0","scale":18,"unit_id":"00000000-0000-0000-0000-000000000001"}"#,
    )
    .unwrap();
    assert_eq!(serde_json::to_value(zero).unwrap()["coefficient"], "0");
    assert_eq!(
        amount(1, 18).decimal_string().unwrap(),
        "0.000000000000000001"
    );
    assert_eq!(
        amount(i128::MAX, 0).checked_ratio(2, 2, 0).unwrap(),
        amount(i128::MAX, 0)
    );
    assert!(amount(1, 1).checked_rescale(0).is_err());
    assert!(amount(1, 0).checked_ratio(1, 3, 0).is_err());
    assert_eq!(
        serde_json::to_value(CommitSeq(u64::MAX)).unwrap(),
        u64::MAX.to_string()
    );
}
fn strict_bounds() {
    let unit = json!({"id":"00000000-0000-0000-0000-000000000001","exponent":18,"native_code":"USD","kind":"Currency"});
    assert!(decode::<currents_types::UnitDefinition>(&serde_json::to_vec(&unit).unwrap()).is_ok());
    let mut bad_unit = unit.clone();
    bad_unit["exponent"] = json!(19);
    assert!(
        decode::<currents_types::UnitDefinition>(&serde_json::to_vec(&bad_unit).unwrap()).is_err()
    );
    bad_unit = unit;
    bad_unit["native_code"] = json!("usd");
    assert!(
        decode::<currents_types::UnitDefinition>(&serde_json::to_vec(&bad_unit).unwrap()).is_err()
    );
    let dtos: Value = serde_json::from_str(&fixture("dtos.json")).unwrap();
    macro_rules! roundtrip {
        ($ty:ty,$name:literal) => {
            let value: $ty = decode(&serde_json::to_vec(&dtos[$name]).unwrap()).unwrap();
            assert_eq!(
                decode::<$ty>(&canonical_bytes(&value).unwrap()).unwrap(),
                value
            );
        };
    }
    roundtrip!(currents_types::ReadBasis, "ReadBasis");
    roundtrip!(currents_types::DomainCommitV1, "DomainCommitV1");
    roundtrip!(currents_types::CommandResultV1, "CommandResultV1");
    roundtrip!(currents_protocol::PreviewV1, "PreviewV1");
    roundtrip!(currents_protocol::QueryV1, "QueryV1");
    roundtrip!(currents_protocol::QueryResponseV1, "QueryResponseV1");
    roundtrip!(currents_types::DecisionPlan, "DecisionPlan");
    let requests: Vec<currents_protocol::RequestV1> =
        decode(fixture("requests.json").as_bytes()).unwrap();
    assert_eq!(requests.len(), 5);
    let mut missing = dtos["CommandResultV1"].clone();
    missing.as_object_mut().unwrap().remove("commit");
    assert!(
        decode::<currents_types::CommandResultV1>(&serde_json::to_vec(&missing).unwrap()).is_err()
    );

    let bounds: Value = serde_json::from_str(&fixture("bounds.json")).unwrap();
    for value in bounds.as_object().unwrap().values() {
        assert!(decode::<Amount>(&serde_json::to_vec(value).unwrap()).is_err());
    }
    for coefficient in [
        "1e2",
        "+1",
        "01",
        "1.0",
        "99999999999999999999999999999999999999999999999999999999999",
    ] {
        assert!(decode::<Amount>(&serde_json::to_vec(&json!({"coefficient":coefficient,"scale":0,"unit_id":"00000000-0000-0000-0000-000000000001"})).unwrap()).is_err());
    }
    assert!(amount(i128::MAX, 0).checked_rescale(1).is_err());
    assert!(amount(i128::MAX, 0).checked_add(&amount(1, 0)).is_err());
    assert!(amount(i128::MAX, 0).checked_ratio(2, 1, 0).is_err());
    let baseline: CommandBodyV1 = decode(fixture("command.json").as_bytes()).unwrap();
    let digest = command_digest(&baseline).unwrap();
    let mut unknown = serde_json::to_value(&baseline).unwrap();
    unknown["operation"]["kind"] = json!("UnknownFinancialCommand");
    assert!(decode::<CommandBodyV1>(&serde_json::to_vec(&unknown).unwrap()).is_err());
    unknown = serde_json::to_value(&baseline).unwrap();
    unknown["protocol_version"] = json!(2);
    assert!(decode::<CommandBodyV1>(&serde_json::to_vec(&unknown).unwrap()).is_err());
    assert!(
        decode::<AuthoritativeRecordV1>(br#"{"kind":"UnknownFinancialEvent","value":{}}"#).is_err()
    );
    assert!(decode::<Value>(&vec![b' '; MAX_MESSAGE_BYTES + 1]).is_err());
    let nested = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert!(decode::<Value>(nested.as_bytes()).is_err());
    assert!(decode::<Value>(br#"{"x":1,"x":2}"#).is_err());
    assert!(decode::<Value>(br#"{"x":1,"\u0078":2}"#).is_err());
    assert!(decode::<CommitSeq>(b"9007199254740993").is_err());
    assert!(decode::<CommitSeq>(br#""18446744073709551616""#).is_err());
    assert_eq!(
        command_digest(&baseline).unwrap(),
        digest,
        "decode has no mutation authority"
    );
    let records: Vec<AuthoritativeRecordV1> = decode(fixture("records.json").as_bytes()).unwrap();
    assert_eq!(records.len(), 24);
    let operations: Vec<CommandBodyV1> = decode(fixture("operations.json").as_bytes()).unwrap();
    assert_eq!(operations.len(), 22);
    for record in records {
        let bytes = canonical_bytes(&record).unwrap();
        assert_eq!(decode::<AuthoritativeRecordV1>(&bytes).unwrap(), record);
    }
    for body in operations {
        let bytes = canonical_bytes(&body).unwrap();
        assert_eq!(decode::<CommandBodyV1>(&bytes).unwrap(), body);
    }
    let schema: Value = serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1/contracts.json"),
        )
        .unwrap(),
    )
    .unwrap();
    use currents_types::schema::Schema;
    assert_eq!(
        schema["$defs"]["AuthoritativeRecordV1"],
        AuthoritativeRecordV1::schema()
    );
    assert_eq!(schema["$defs"]["CommandBodyV1"], CommandBodyV1::schema());
    assert_eq!(schema["$defs"]["Amount"], Amount::schema());
}
fn temporal_precision() {
    let temporal: TemporalValue = decode(fixture("temporal.json").as_bytes()).unwrap();
    let TemporalValue::Date {
        civil,
        original_text,
    } = &temporal
    else {
        panic!("not a civil date")
    };
    assert_eq!(civil.to_string(), "2026-11-09");
    assert_eq!(original_text, "11/09/2026");
    let text = String::from_utf8(canonical_bytes(&temporal).unwrap()).unwrap();
    assert!(!text.contains("00:00"));
    let values = [
        SourceValue::<String>::Missing,
        SourceValue::Null,
        SourceValue::Absent,
        SourceValue::Deleted,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for value in values {
        let bytes = canonical_bytes(&value).unwrap();
        assert!(seen.insert(bytes.clone()));
        assert_eq!(decode::<SourceValue<String>>(&bytes).unwrap(), value);
    }
    assert!(decode::<SourceValue<String>>(b"null").is_err());
    let unknown = TemporalValue::Unknown {
        original_text: "9am somewhere".into(),
    };
    assert_eq!(
        decode::<TemporalValue>(&canonical_bytes(&unknown).unwrap()).unwrap(),
        unknown
    );
    assert!(decode::<TemporalValue>(br#"{"kind":"Instant","utc":"2026-11-09T09:00:00","original_offset":null,"original_zone":null,"precision":"Unknown","original_text":"9am"}"#).is_err());
    let instant=br#"{"kind":"Instant","utc":"2026-11-09T09:00:00Z","original_offset":null,"original_zone":null,"precision":"Unknown","original_text":"9am"}"#;
    let t: TemporalValue = decode(instant).unwrap();
    let TemporalValue::Instant {
        original_zone,
        original_offset,
        ..
    } = t
    else {
        panic!()
    };
    assert_eq!(original_zone, None);
    assert_eq!(original_offset, None);
}
fn digest_auth() {
    let dtos: Value = serde_json::from_str(&fixture("dtos.json")).unwrap();
    let mut commit: currents_types::DomainCommitV1 =
        decode(&serde_json::to_vec(&dtos["DomainCommitV1"]).unwrap()).unwrap();
    let digest = currents_protocol::commit_digest(&commit).unwrap();
    commit.digest = currents_types::Digest([7; 32]);
    assert_eq!(currents_protocol::commit_digest(&commit).unwrap(), digest);
    commit.native_head_after.checkpoint = currents_types::Digest([8; 32]);
    assert_ne!(currents_protocol::commit_digest(&commit).unwrap(), digest);
    let golden: Value = serde_json::from_str(&fixture("command-digest.json")).unwrap();
    let body: CommandBodyV1 = decode(fixture("command.json").as_bytes()).unwrap();
    assert_eq!(
        canonical_bytes(&body).unwrap(),
        golden["canonical_utf8"].as_str().unwrap().as_bytes()
    );
    assert_eq!(
        serde_json::to_value(command_digest(&body).unwrap()).unwrap(),
        golden["sha256"]
    );

    let body: CommandBodyV1 = decode(fixture("command.json").as_bytes()).unwrap();
    let digest = command_digest(&body).unwrap();
    let auth1:AuthorizationEnvelope=decode(br#"{"session":"00000000-0000-0000-0000-000000000001","capability":"CANARY-BEARER-ONE","approval":null}"#).unwrap();
    let auth2:AuthorizationEnvelope=decode(br#"{"session":"00000000-0000-0000-0000-000000000002","capability":"CANARY-BEARER-TWO","approval":"CANARY-APPROVAL"}"#).unwrap();
    assert_ne!(auth1.session, auth2.session);
    assert_eq!(command_digest(&body).unwrap(), digest);
    let mut changed = body.clone();
    changed.expected_head = CommitSeq(body.expected_head.0 + 1);
    assert_ne!(command_digest(&changed).unwrap(), digest);
    let operations: Vec<CommandBodyV1> = decode(fixture("operations.json").as_bytes()).unwrap();
    let mut monetary = operations
        .into_iter()
        .find(|b| {
            matches!(
                b.operation,
                currents_protocol::OperationV1::RecordManualPosition(_)
            )
        })
        .unwrap();
    let original = command_digest(&monetary).unwrap();
    if let currents_protocol::OperationV1::RecordManualPosition(ref mut p) = monetary.operation {
        p.amount = amount(2, 0);
    }
    assert_ne!(command_digest(&monetary).unwrap(), original);
    let records: Value = serde_json::from_str(&fixture("records.json")).unwrap();
    let decision = records
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == "Decision")
        .unwrap()["value"]
        .clone();
    let d: DecisionEnvelopeV1 = decode(&serde_json::to_vec(&decision).unwrap()).unwrap();
    let bytes = canonical_bytes(&d).unwrap();
    assert!(!String::from_utf8(bytes).unwrap().contains("CANARY"));
    let mut injection = decision;
    injection["capability"] = json!(auth1.capability);
    assert!(decode::<DecisionEnvelopeV1>(&serde_json::to_vec(&injection).unwrap()).is_err());
}
