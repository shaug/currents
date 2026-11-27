//! Synthetic C07 public-boundary acceptance, never live financial input.
use currents_acquisition::{capture::BoundedEvidence, parse::FileParser};
use currents_domain::evidence::{EvidenceEngine, ValidatedEvidenceBatchV1};
use currents_types::*;

fn id(n: u128) -> RecordId {
    RecordId(uuid::Uuid::from_u128(n))
}
fn context(rows: usize, generation: u128) -> ImportContextV1 {
    ImportContextV1 {
        receipt_id: id(generation),
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: id(1),
            device_id: None,
        },
        captured_at: serde_json::from_str("\"2026-11-11T12:00:00Z\"").unwrap(),
        received_at: serde_json::from_str("\"2026-11-12T13:00:00Z\"").unwrap(),
        coverage: vec![CoverageV1 {
            scope: DateScope {
                start: None,
                end: None,
                basis: DateBasis::Effective,
            },
            status: CoverageStatus::Unknown,
        }],
        row_ids: (0..rows)
            .map(|n| ImportRowIdsV1 {
                observation: id(10000 + n as u128),
                product: id(generation + 20000 + n as u128),
                facet: id(40000 + n as u128),
            })
            .collect(),
    }
}
fn profile(format: ImportFormatV1) -> ParserProfileV1 {
    ParserProfileV1 {
        schema_version: 1,
        format,
        parser_version: "1".into(),
        unit: UnitDefinition {
            id: UnitId(id(2).0),
            exponent: 2,
            native_code: "USD".into(),
            kind: UnitKind::Currency,
        },
    }
}
fn parse(bytes: Vec<u8>, rows: usize) -> Result<CaptureBatchV1, EngineError> {
    FileParser::parse(
        BoundedEvidence::new("synthetic.csv", bytes, context(rows, 100))?,
        profile(ImportFormatV1::SyntheticCsv),
    )
}
const CSV: &str = include_str!("../../../fixtures/imports/synthetic-v1.csv");
#[test]
fn cur_012_1() {
    let bytes = vec![b'x'; 100 * 1_048_576];
    let bounded = BoundedEvidence::new("statement.txt", bytes, context(0, 100)).unwrap();
    assert_eq!(bounded.chunks().count(), 100);
    assert!(bounded.chunks().all(|c| c.len() <= 1_048_576));
    let attachment = FileParser::parse(bounded, profile(ImportFormatV1::InertText)).unwrap();
    assert_eq!(
        attachment.receipt.contents[0].byte_count,
        CommitSeq(104_857_600)
    );
    assert!(attachment.receipt.complete);
    drop(attachment);
    let mut message = b"[]".to_vec();
    message.resize(1_048_576, b' ');
    assert!(
        FileParser::parse(
            BoundedEvidence::new("manual.json", message.clone(), context(0, 110)).unwrap(),
            profile(ImportFormatV1::SyntheticManual)
        )
        .is_ok()
    );
    message.push(b' ');
    assert!(
        FileParser::parse(
            BoundedEvidence::new("manual.json", message, context(0, 111)).unwrap(),
            profile(ImportFormatV1::SyntheticManual)
        )
        .is_err()
    );

    assert!(
        BoundedEvidence::new(
            "statement.txt",
            vec![b'x'; 100 * 1_048_576 + 1],
            context(0, 100)
        )
        .is_err()
    );
    assert!(parse(CSV.as_bytes().to_vec(), 1).is_ok());
    let huge = format!(
        "original_id,amount,unit,effective,description\nr1,1.00,USD,2026-11-10,{}\n",
        "x".repeat(1_048_576)
    );
    assert!(parse(huge.into_bytes(), 1).is_err());
    let many = format!(
        "original_id,amount,unit,effective,description\n{}",
        "r,1.00,USD,2026-11-10,x\n".repeat(10001)
    );
    assert!(parse(many.into_bytes(), 10001).is_err());

    for (n, accepted) in [(9_998, true), (9_999, false), (10_000, false)] {
        let many = format!(
            "original_id,amount,unit,effective,description\n{}",
            "r,1.00,USD,2026-11-10,x\n".repeat(n)
        );
        assert_eq!(
            parse(many.into_bytes(), n).is_ok(),
            accepted,
            "exact item-boundary {n}"
        );
    }
}

#[test]
fn cur_012_2() {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(persisted_attachment());
    let executable = BoundedEvidence::new(
        "statement.txt",
        b"\x7fELF executable".to_vec(),
        context(0, 100),
    )
    .unwrap();
    assert!(
        FileParser::parse(executable, profile(ImportFormatV1::InertText)).is_err(),
        "inert attachment cannot carry executable content"
    );

    for name in [
        "../evil.csv",
        "/tmp/evil.csv",
        "a\\evil.csv",
        "..",
        "a/b.csv",
    ] {
        assert!(BoundedEvidence::new(name, CSV.as_bytes().to_vec(), context(1, 100)).is_err());
    }
    for body in [
        b"MZ executable".as_slice(),
        b"PK\x03\x04archive",
        b"\x7fELF executable",
        b"#!/bin/sh",
    ] {
        assert!(parse(body.to_vec(), 1).is_err());
    }
    for body in [
        CSV.replace("12.34", "12,34"),
        CSV.replace("12.34", "\"12,34\""),
        CSV.replace("USD,2026", "EUR,2026"),
        CSV.replace("synthetic purchase", "\"unterminated"),
        CSV.replace("12.34", "1e3"),
    ] {
        assert!(parse(body.into_bytes(), 1).is_err());
    }
    let deep = format!("{}0{}", "[".repeat(200), "]".repeat(200));
    let input = BoundedEvidence::new("manual.json", deep.into_bytes(), context(1, 100)).unwrap();
    assert!(FileParser::parse(input, profile(ImportFormatV1::SyntheticManual)).is_err());
    let attachment = FileParser::parse(
        BoundedEvidence::new(
            "statement.txt",
            b"synthetic inert statement".to_vec(),
            context(0, 100),
        )
        .unwrap(),
        profile(ImportFormatV1::InertText),
    )
    .unwrap();
    assert!(attachment.observations.is_empty());
    assert_eq!(attachment.contents[0].bytes, b"synthetic inert statement");
}
#[test]
fn cur_012_3() {
    let captured = parse(CSV.as_bytes().to_vec(), 1).unwrap();
    let actual: ImportCaptureMetadataV1 =
        serde_json::from_slice(&captured.contents[1].bytes).unwrap();
    let expected: ImportCaptureMetadataV1 = serde_json::from_str(include_str!(
        "../../../fixtures/imports/capture-metadata-v1.json"
    ))
    .unwrap();
    assert_eq!(actual, expected);

    // The unchanged public record decoder is the base-red/new-product-green surface.
    assert!(
        serde_json::from_str::<AuthoritativeRecordV1>(include_str!(
            "../../../fixtures/imports/addressed-product-v1.json"
        ))
        .is_ok()
    );

    let mut incoming_context = context(1, 102);
    incoming_context.row_ids[0] = ImportRowIdsV1 {
        observation: id(11000),
        product: id(21002),
        facet: id(41000),
    };
    let candidate = FileParser::parse(
        BoundedEvidence::new(
            "other.csv",
            CSV.replace("r1,", "r2,").into_bytes(),
            incoming_context.clone(),
        )
        .unwrap(),
        profile(ImportFormatV1::SyntheticCsv),
    )
    .unwrap();
    let original = parse(CSV.as_bytes().to_vec(), 1).unwrap();
    let mut existing = DomainState::default();
    existing.records.insert(
        original.observations[0].id,
        AuthoritativeRecordV1::ObservationVersion(original.observations[0].clone()),
    );
    // Source identifiers differ, but matching economic evidence is review-only.
    assert_ne!(
        candidate.observations[0].original_ids,
        original.observations[0].original_ids
    );
    for changed in [
        CSV.replace("12.34", "12.35"),
        CSV.replace("2026-11-10", "2026-11-09"),
        CSV.replace("synthetic purchase", "different description"),
    ] {
        let different = FileParser::parse(
            BoundedEvidence::new(
                "different.csv",
                changed.into_bytes(),
                incoming_context.clone(),
            )
            .unwrap(),
            profile(ImportFormatV1::SyntheticCsv),
        )
        .unwrap();
        assert!(
            EvidenceEngine::similarity_candidates(
                &existing,
                &ValidatedEvidenceBatchV1::new(different).unwrap()
            )
            .is_empty()
        );
    }
    let validated = ValidatedEvidenceBatchV1::new(candidate).unwrap();
    assert_eq!(
        EvidenceEngine::similarity_candidates(&existing, &validated).len(),
        1
    );
    let plan = EvidenceEngine::publish(&existing, validated).unwrap();
    assert!(plan.records.iter().any(|r|matches!(r,AuthoritativeRecordV1::ObservationVersion(o) if o.id==id(11000) && o.prior_version.is_none())));
    assert!(plan.invalidations.is_empty());
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
    assert_eq!(existing.records.len(), 1);
    let product = original.products[0].clone();
    let mut wire = serde_json::to_value(&product).unwrap();
    wire["schema_version"] = serde_json::json!(3);
    assert!(serde_json::from_value::<AddressedParseProductV1>(wire).is_err());
    let mut alias = product;
    alias.id = alias.product.source_version_id;
    assert!(serde_json::to_vec(&alias).is_err());

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(persisted_reparse());
    let first = parse(CSV.as_bytes().to_vec(), 1).unwrap();
    let mut second_context = context(1, 101);
    second_context.received_at = serde_json::from_str("\"2026-11-12T12:00:00Z\"").unwrap();
    let mut second_profile = profile(ImportFormatV1::SyntheticCsv);
    second_profile.parser_version = "2".into();
    let second = FileParser::parse(
        BoundedEvidence::new("synthetic.csv", CSV.as_bytes().to_vec(), second_context).unwrap(),
        second_profile,
    )
    .unwrap();
    assert_eq!(first.observations, second.observations);
    assert_ne!(first.products[0].id, second.products[0].id);
    assert_eq!(
        first.products[0].product.facets[0].lineage_key,
        second.products[0].product.facets[0].lineage_key
    );
    let mut state = DomainState::default();
    state.records.insert(
        first.observations[0].id,
        AuthoritativeRecordV1::ObservationVersion(first.observations[0].clone()),
    );
    state.records.insert(
        first.products[0].id,
        AuthoritativeRecordV1::AddressedParseProduct(first.products[0].clone()),
    );
    let original = state.records.clone();
    let plan =
        EvidenceEngine::publish(&state, ValidatedEvidenceBatchV1::new(second).unwrap()).unwrap();
    assert_eq!(state.records, original);
    assert!(plan.records.iter().any(|r|matches!(r,AuthoritativeRecordV1::AddressedParseProduct(p) if p.product.parser_version=="2")));
    assert!(
        !plan
            .records
            .iter()
            .any(|r| matches!(r, AuthoritativeRecordV1::ObservationVersion(_)))
    );
    let mut unsupported = profile(ImportFormatV1::SyntheticCsv);
    unsupported.parser_version = "999".into();
    assert!(
        FileParser::parse(
            BoundedEvidence::new("synthetic.csv", CSV.as_bytes().to_vec(), context(1, 102))
                .unwrap(),
            unsupported
        )
        .is_err()
    );
}
#[test]
fn cur_012_4() {
    use sha2::Digest as _;
    let mut unknown = parse(CSV.as_bytes().to_vec(), 1).unwrap();
    let mut metadata: ImportCaptureMetadataV1 =
        serde_json::from_slice(&unknown.contents[1].bytes).unwrap();
    metadata.schema_version = 2;
    unknown.contents[1].bytes = serde_json::to_vec(&metadata).unwrap();
    unknown.contents[1].digest = Digest(sha2::Sha256::digest(&unknown.contents[1].bytes).into());
    unknown.receipt.contents[1].digest = unknown.contents[1].digest;
    unknown.receipt.contents[1].byte_count = CommitSeq(unknown.contents[1].bytes.len() as u64);
    assert_eq!(
        ValidatedEvidenceBatchV1::new(unknown).err().unwrap().class,
        ErrorClass::UnsupportedVersion
    );

    let mut wrong_role = parse(CSV.as_bytes().to_vec(), 1).unwrap();
    wrong_role.receipt.contents[1].media_type = "text/plain".into();
    assert!(ValidatedEvidenceBatchV1::new(wrong_role).is_err());
    let mut swapped_time = parse(CSV.as_bytes().to_vec(), 1).unwrap();
    let mut metadata: ImportCaptureMetadataV1 =
        serde_json::from_slice(&swapped_time.contents[1].bytes).unwrap();
    metadata.original_capture_time = swapped_time.receipt.received_time.clone();
    swapped_time.contents[1].bytes = serde_json::to_vec(&metadata).unwrap();
    swapped_time.contents[1].digest =
        Digest(sha2::Sha256::digest(&swapped_time.contents[1].bytes).into());
    swapped_time.receipt.contents[1].digest = swapped_time.contents[1].digest;
    swapped_time.receipt.contents[1].byte_count =
        CommitSeq(swapped_time.contents[1].bytes.len() as u64);
    assert!(ValidatedEvidenceBatchV1::new(swapped_time).is_err());
    let mut missing = parse(CSV.as_bytes().to_vec(), 1).unwrap();
    missing.contents.remove(1);
    assert!(ValidatedEvidenceBatchV1::new(missing).is_err());
    let mut tampered = parse(CSV.as_bytes().to_vec(), 1).unwrap();
    tampered.contents[1].bytes.push(b'x');
    assert!(ValidatedEvidenceBatchV1::new(tampered).is_err());

    assert!(
        parse(
            CSV.replace(
                "synthetic purchase",
                "https://example.invalid/?token=synthetic-canary"
            )
            .into_bytes(),
            1
        )
        .is_err(),
        "secret URL parameters cannot enter captured history"
    );

    let mut amount_forgery = parse(CSV.as_bytes().to_vec(), 1).unwrap();
    amount_forgery.products[0].product.facets[0].amount =
        SourceValue::Known(Amount::new(99999, 2, UnitId(id(2).0)).unwrap());
    assert!(ValidatedEvidenceBatchV1::new(amount_forgery).is_err());
    let mut forged = parse(CSV.as_bytes().to_vec(), 1).unwrap();
    forged.observations[0].reported_fields[1].value = SourceValue::Known("999.99".into());
    assert!(
        ValidatedEvidenceBatchV1::new(forged).is_err(),
        "reported fields must match retained source representation"
    );

    let batch = parse(CSV.as_bytes().to_vec(), 1).unwrap();
    let plan = EvidenceEngine::publish(
        &DomainState::default(),
        ValidatedEvidenceBatchV1::new(batch).unwrap(),
    )
    .unwrap();
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
    assert!(plan.records.iter().all(|r| matches!(
        r,
        AuthoritativeRecordV1::ImportReceipt(_)
            | AuthoritativeRecordV1::ObservationVersion(_)
            | AuthoritativeRecordV1::AddressedParseProduct(_)
    )));
    assert!(
        parse(
            CSV.replace(
                "synthetic purchase",
                "Authorization: Bearer synthetic-secret"
            )
            .into_bytes(),
            1
        )
        .is_err()
    );
    let manual=br#"[{"original_id":{"state":"Absent"},"amount":{"state":"Null"},"unit":"USD","effective":{"state":"Unknown"},"description":{"state":"Deleted"}}]"#;
    let batch = FileParser::parse(
        BoundedEvidence::new("manual.json", manual.to_vec(), context(1, 103)).unwrap(),
        profile(ImportFormatV1::SyntheticManual),
    )
    .unwrap();
    assert_eq!(
        batch.observations[0].reported_fields[0].value,
        SourceValue::Absent
    );
    assert_eq!(
        batch.products[0].product.facets[0].amount,
        SourceValue::Null
    );
}

async fn persisted_reparse() {
    use currents_cipher::{CipherProfileV1, DatasetKey};
    use currents_storage::{EncryptedStore, HistoryAppend};
    let directory = tempfile::tempdir().unwrap();
    let store = EncryptedStore::open(
        &directory.path().join("synthetic.db"),
        DatasetKey::new([0x29; 32]),
        CipherProfileV1,
    )
    .await
    .unwrap();
    let policy = DatasetPolicyV1 {
        id: id(700),
        version: 1,
        policies: PolicyVersionsV1 {
            selection: 1,
            recognition: 1,
            query: 1,
            calendar: 1,
            timezone_data: "synthetic".into(),
        },
        source_precedence: vec![],
        freshness_seconds: CommitSeq(1),
        opening_policy: id(700),
        withholding_policy: id(700),
    };
    let result = |n| CommandResultV1 {
        command_id: CommandId(id(n).0),
        digest: Digest([n as u8; 32]),
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
    let bootstrap = store
        .writer()
        .append(HistoryAppend {
            prior_head: CommitSeq(0),
            records: vec![
                AuthoritativeRecordV1::UnitCatalog(UnitCatalogV1 {
                    id: id(701),
                    version: 1,
                    units: vec![profile(ImportFormatV1::SyntheticCsv).unit],
                }),
                AuthoritativeRecordV1::DatasetPolicy(policy),
            ],
            decisions: vec![],
            result: result(800),
            staged: vec![],
            native: None,
        })
        .await
        .unwrap();
    let mut state = DomainState::default();
    let mut first = None;
    for generation in [100, 101] {
        let mut p = profile(ImportFormatV1::SyntheticCsv);
        p.parser_version = if generation == 100 { "1" } else { "2" }.into();
        let mut capture_context = context(1, generation);
        if generation == 101 {
            capture_context.received_at = serde_json::from_str("\"2026-11-13T14:00:00Z\"").unwrap();
        }
        let capture = FileParser::parse(
            BoundedEvidence::new("synthetic.csv", CSV.as_bytes().to_vec(), capture_context)
                .unwrap(),
            p,
        )
        .unwrap();
        let validated = ValidatedEvidenceBatchV1::new(capture).unwrap();
        let mut staged = Vec::new();
        for content in validated.contents() {
            staged.push(store.stage(&content.bytes).await.unwrap());
        }
        let plan = EvidenceEngine::publish(&state, validated).unwrap();
        assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
        let r = result(generation + 800);
        let seq = if generation == 100 { 2 } else { 3 };
        let decision = DecisionEnvelopeV1 {
            id: id(generation + 900),
            commit: CommitSeq(seq),
            item_position: plan.records.len() as u32,
            decision_type: DecisionKind::Interpretation,
            schema_version: 1,
            actor: id(500),
            authority: AuthorityKind::Owner,
            grant_id: None,
            command_id: r.command_id,
            command_digest: r.digest,
            references: vec![id(20100)],
            recorded_time: context(0, 100).captured_at,
            adopted_result: vec![id(20100)],
            policy: PolicyReferenceV1 {
                id: id(700),
                version: 1,
            },
            corrections: vec![],
            supersedes: vec![],
        };
        let commit = store
            .writer()
            .append(HistoryAppend {
                prior_head: CommitSeq(seq - 1),
                records: plan.records,
                decisions: vec![decision],
                result: r,
                staged,
                native: None,
            })
            .await
            .unwrap();
        assert_eq!(commit.native_head_after, bootstrap.native_head_after);
        for record in &commit.records {
            match record {
                AuthoritativeRecordV1::ObservationVersion(o) => {
                    state.records.insert(o.id, record.clone());
                }
                AuthoritativeRecordV1::AddressedParseProduct(p) => {
                    state.records.insert(p.id, record.clone());
                }
                _ => {}
            }
        }
        if generation == 100 {
            first = Some(commit);
        }
    }
    store.close().await.unwrap();
    let store = EncryptedStore::open(
        &directory.path().join("synthetic.db"),
        DatasetKey::new([0x29; 32]),
        CipherProfileV1,
    )
    .await
    .unwrap();
    let history = store.history().read_prefix(CommitSeq(3)).await.unwrap();
    for commit in history.iter().skip(1) {
        let receipt = commit
            .records
            .iter()
            .find_map(|r| {
                if let AuthoritativeRecordV1::ImportReceipt(r) = r {
                    Some(r)
                } else {
                    None
                }
            })
            .unwrap();
        let expected_received = if receipt.id == id(100) {
            context(0, 100).received_at
        } else {
            serde_json::from_str("\"2026-11-13T14:00:00Z\"").unwrap()
        };
        assert_eq!(receipt.received_time, expected_received);
        let reference = receipt
            .contents
            .iter()
            .find(|c| c.media_type == CAPTURE_METADATA_MEDIA_TYPE)
            .unwrap();
        let mut stream = store
            .history()
            .stream_content(CommitSeq(3), reference.digest)
            .await
            .unwrap();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await.unwrap() {
            bytes.extend(chunk);
        }
        let metadata: ImportCaptureMetadataV1 = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(metadata.original_capture_time, context(0, 100).captured_at);
        assert_ne!(metadata.original_capture_time, receipt.received_time);
        assert_eq!(metadata.receipt_id, receipt.id);
    }

    assert_eq!(history[1], first.unwrap());
    assert_eq!(history[2].decisions[0].adopted_result, vec![id(20100)]);
    assert!(history[1].records.iter().any(|r|matches!(r,AuthoritativeRecordV1::AddressedParseProduct(p) if p.id==id(20100) && p.product.parser_version=="1")));
    // The addressed product role must not accept dangling or wrong-kind references.
    for (n, source) in [(950, id(999999)), (951, id(700))] {
        let mut bad = parse(CSV.as_bytes().to_vec(), 1)
            .unwrap()
            .products
            .remove(0);
        bad.id = id(n);
        bad.product.source_version_id = source;
        let rejected = store
            .writer()
            .append(HistoryAppend {
                prior_head: CommitSeq(3),
                records: vec![AuthoritativeRecordV1::AddressedParseProduct(bad)],
                decisions: vec![],
                result: result(n + 1000),
                staged: vec![],
                native: None,
            })
            .await;
        assert!(rejected.is_err());
        assert_eq!(
            store.history().read_prefix(CommitSeq(3)).await.unwrap(),
            history
        );
    }
    let count = store.history().content_count(CommitSeq(3)).await.unwrap();
    let rejected = parse(
        CSV.replace("synthetic purchase", "password=synthetic-canary")
            .into_bytes(),
        1,
    );
    assert!(rejected.is_err());
    assert_eq!(
        store.history().content_count(CommitSeq(3)).await.unwrap(),
        count
    );
    assert_eq!(
        store.history().read_prefix(CommitSeq(3)).await.unwrap(),
        history
    );
    store.close().await.unwrap();
}

async fn persisted_attachment() {
    use currents_cipher::{CipherProfileV1, DatasetKey};
    use currents_storage::{EncryptedStore, HistoryAppend};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("attachment.db");
    let store = EncryptedStore::open(&path, DatasetKey::new([0x28; 32]), CipherProfileV1)
        .await
        .unwrap();
    let capture = FileParser::parse(
        BoundedEvidence::new(
            "statement.txt",
            include_bytes!("../../../fixtures/imports/inert-statement.txt").to_vec(),
            context(0, 100),
        )
        .unwrap(),
        profile(ImportFormatV1::InertText),
    )
    .unwrap();
    let validated = ValidatedEvidenceBatchV1::new(capture).unwrap();
    let mut staged = Vec::new();
    for content in validated.contents() {
        staged.push(store.stage(&content.bytes).await.unwrap());
    }
    let plan = EvidenceEngine::publish(&DomainState::default(), validated).unwrap();
    assert_eq!(plan.records.len(), 1);
    let result = CommandResultV1 {
        command_id: CommandId(id(800).0),
        digest: Digest([0x28; 32]),
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
    store
        .writer()
        .append(HistoryAppend {
            prior_head: CommitSeq(0),
            records: plan.records,
            decisions: vec![],
            result,
            staged,
            native: None,
        })
        .await
        .unwrap();
    store.close().await.unwrap();
    let store = EncryptedStore::open(&path, DatasetKey::new([0x28; 32]), CipherProfileV1)
        .await
        .unwrap();
    let history = store.history().read_prefix(CommitSeq(1)).await.unwrap();
    let AuthoritativeRecordV1::ImportReceipt(receipt) = &history[0].records[0] else {
        panic!("attachment receipt")
    };
    let reference = receipt
        .contents
        .iter()
        .find(|c| c.media_type == CAPTURE_METADATA_MEDIA_TYPE)
        .unwrap();
    let mut stream = store
        .history()
        .stream_content(CommitSeq(1), reference.digest)
        .await
        .unwrap();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await.unwrap() {
        bytes.extend(chunk);
    }
    let metadata: ImportCaptureMetadataV1 = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(metadata.original_capture_time, context(0, 100).captured_at);
    assert_eq!(receipt.received_time, context(0, 100).received_at);
    assert_ne!(metadata.original_capture_time, receipt.received_time);
    assert_eq!(metadata.profile_id, "inert-text");
    drop(stream);
    store.close().await.unwrap();
}
