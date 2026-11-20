//! Public synthetic source descriptions only. Financial adoption remains domain-owned.
use currents_acquisition::identity::RelationshipSourceParser;
use currents_types::*;

#[test]
fn relationship_source_keeps_payroll_facets_and_net_only_unknowns() {
    let rows = RelationshipSourceParser::parse(include_bytes!(
        "../../../fixtures/journeys/s3/relationship-source.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].facets.len(), 5);
    assert_eq!(rows[0].facets[0].role, RelationshipFacetRoleV1::GrossPay);
    assert_eq!(rows[0].facets[0].amount, "5000");
    let net = RelationshipSourceParser::parse(include_bytes!(
        "../../../fixtures/journeys/s3/net-only-source.json"
    ))
    .unwrap();
    assert_eq!(net[0].facets.len(), 1);
    assert_eq!(net[0].facets[0].role, RelationshipFacetRoleV1::NetPay);
}

#[test]
fn relationship_source_rejects_ambiguous_identity_and_secret_bodies() {
    let original: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/journeys/s3/relationship-source.json"
    ))
    .unwrap();
    let mut duplicate = original.clone();
    duplicate[0]["facets"][1]["lineage"] = duplicate[0]["facets"][0]["lineage"].clone();
    assert!(RelationshipSourceParser::parse(&serde_json::to_vec(&duplicate).unwrap()).is_err());
    let mut unknown = original.clone();
    unknown[0]["facets"][0]["role"] = "GuessGrossFromNet".into();
    assert!(RelationshipSourceParser::parse(&serde_json::to_vec(&unknown).unwrap()).is_err());
    let mut secret = original;
    secret[0]["access_token"] = "SYNTHETIC_SECRET_CANARY".into();
    let error = RelationshipSourceParser::parse(&serde_json::to_vec(&secret).unwrap())
        .err()
        .unwrap();
    assert!(!format!("{error:?}").contains("SYNTHETIC_SECRET_CANARY"));
}

#[test]
fn relationship_profile_is_explicit_bounded_and_never_orders_by_arrival() {
    let original: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/journeys/s2/relationship-source.json"
    ))
    .unwrap();
    let rows = RelationshipSourceParser::parse(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[3].facets[0].role, RelationshipFacetRoleV1::Fee);
    for case in 0..7 {
        let mut invalid = original.clone();
        match case {
            0 => invalid[0]["facets"][0]["amount"] = "1e3".into(),
            1 => invalid[0]["facets"][0]["amount"] = "NaN".into(),
            2 => invalid[0]["facets"][0]["amount"] = "01".into(),
            3 => invalid[0]["effective"] = "2026-02-30".into(),
            4 => invalid[0]["prior_revision"] = "original".into(),
            5 => invalid[0]["facets"][0]["unit"] = "usd".into(),
            _ => {
                invalid[0].as_object_mut().unwrap().remove("prior_revision");
            }
        }
        assert!(
            RelationshipSourceParser::parse(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "case {case}"
        );
    }
    let mut competing = original[0].clone();
    competing["source_revision"] = "unorderable-alternative".into();
    competing["facets"][0]["amount"] = "-1200".into();
    let alternatives = serde_json::json!([original[0], competing]);
    let parsed =
        RelationshipSourceParser::parse(&serde_json::to_vec(&alternatives).unwrap()).unwrap();
    assert_eq!(parsed.len(), 2);
    assert!(parsed.iter().all(|r| r.prior_revision.is_none()));
    assert_eq!(parsed[0].facets[0].amount, "-1000");
    assert_eq!(parsed[1].facets[0].amount, "-1200");
    assert!(RelationshipSourceParser::parse(&vec![b' '; 1_048_577]).is_err());
}

#[tokio::test]
async fn relationship_capture_retains_exact_bytes_and_separate_times_without_money() {
    use currents_acquisition::capture::BoundedEvidence;
    use currents_domain::relationship_evidence::{
        RelationshipEvidenceEngine, ValidatedRelationshipCaptureV1,
    };
    let id = |n| RecordId(uuid::Uuid::from_u128(n));
    let bytes = include_bytes!("../../../fixtures/journeys/s3/relationship-source.json").to_vec();
    let context = ImportContextV1 {
        receipt_id: id(100),
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: id(101),
            device_id: None,
        },
        captured_at: serde_json::from_str("\"2026-11-01T00:00:00Z\"").unwrap(),
        received_at: serde_json::from_str("\"2026-11-02T00:00:00Z\"").unwrap(),
        coverage: vec![],
        row_ids: vec![],
    };
    let batch = RelationshipSourceParser::capture(
        BoundedEvidence::new("relationship.json", bytes.clone(), context.clone()).unwrap(),
        id(102),
    )
    .unwrap();
    assert_eq!(batch.capture.original_capture_time, context.captured_at);
    assert_eq!(batch.receipt.received_time, context.received_at);
    assert_eq!(batch.capture.rows[0].facets.len(), 5);
    let mut forged = batch.clone();
    forged.capture.rows[0].facets[0].amount = "9999".into();
    assert!(ValidatedRelationshipCaptureV1::new(forged).is_err());
    let mut forged = batch.clone();
    forged.receipt.source.connection_id = id(999);
    assert!(ValidatedRelationshipCaptureV1::new(forged).is_err());
    let validated = ValidatedRelationshipCaptureV1::new(batch).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-capture.db");
    let store = currents_storage::EncryptedStore::open(
        &path,
        currents_cipher::DatasetKey::new([24; 32]),
        currents_cipher::CipherProfileV1,
    )
    .await
    .unwrap();
    let staged = store.stage(&validated.contents()[0].bytes).await.unwrap();
    let plan = RelationshipEvidenceEngine::publish(&DomainState::default(), validated).unwrap();
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
    assert_eq!(plan.records.len(), 2);
    let command = CommandResultV1 {
        command_id: CommandId(id(103).0),
        digest: Digest([4; 32]),
        state: CommandState::Committed,
        commit: None,
        effects: EffectsV1 {
            records: vec![],
            accounts: vec![],
            amounts: vec![],
            invalidations: vec![],
        },
        health: vec![],
    };
    let commit = store
        .writer()
        .append(currents_storage::HistoryAppend {
            prior_head: CommitSeq(0),
            records: plan.records,
            decisions: vec![],
            staged: vec![staged],
            native: None,
            result: command,
        })
        .await
        .unwrap();
    store.close().await.unwrap();
    let store = currents_storage::EncryptedStore::open(
        &path,
        currents_cipher::DatasetKey::new([24; 32]),
        currents_cipher::CipherProfileV1,
    )
    .await
    .unwrap();
    assert_eq!(
        store.history().read_prefix(commit.seq).await.unwrap(),
        vec![commit.clone()]
    );
    let AuthoritativeRecordV1::RelationshipCapture(capture) = &commit.records[1] else {
        panic!("capture required")
    };
    let mut stream = store
        .history()
        .stream_content(commit.seq, capture.raw.digest)
        .await
        .unwrap();
    let mut retained = Vec::new();
    while let Some(chunk) = stream.next().await.unwrap() {
        retained.extend(chunk);
    }
    assert_eq!(retained, bytes);
    drop(stream);
    store.close().await.unwrap();
}
