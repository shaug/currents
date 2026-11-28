//! Complete replay qualification uses only synthetic encrypted stores.
#[path = "support/closure.rs"]
#[allow(dead_code)]
mod fixture;
use currents_accounting::{AccountingAdapter, ClosureControl};
use currents_cipher::{CipherProfileV1, DatasetKey};
use currents_storage::{EncryptedStore, HistoryAppend};
use currents_types::*;

async fn open(path: &std::path::Path) -> EncryptedStore {
    EncryptedStore::open(path, DatasetKey::new([0x61; 32]), CipherProfileV1)
        .await
        .unwrap()
}
async fn metadata(store: &EncryptedStore) -> DomainCommitV1 {
    let prefix = store.writer().audited_prefix().await.unwrap();
    let state = fixture::initial_state(prefix.native_head);
    let (batch, mut records) = fixture::metadata(&state);
    records.push(AuthoritativeRecordV1::FrozenAccountingManifest(
        AccountingAdapter::manifest(fixture::id(90000), &batch).unwrap(),
    ));
    store
        .writer()
        .append_closure(
            HistoryAppend {
                prior_head: CommitSeq(0),
                records,
                decisions: vec![],
                staged: vec![],
                native: None,
                result: CommandResultV1 {
                    command_id: CommandId(fixture::id(90001).0),
                    digest: Digest([1; 32]),
                    state: CommandState::Committed,
                    commit: None,
                    effects: EffectsV1 {
                        records: vec![],
                        accounts: vec![],
                        amounts: vec![],
                        invalidations: vec![],
                    },
                    health: vec![],
                },
            },
            AccountingAdapter::prepare_closure(&batch).unwrap(),
            &ClosureControl::default(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn cur_011_3() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.db");
    let store = open(&path).await;
    metadata(&store).await;
    store.close().await.unwrap();
    let pool = currents_cipher::open_pool(&path, DatasetKey::new([0x61; 32]), CipherProfileV1, 1)
        .await
        .unwrap();
    let triggers: Vec<(String, String)> = sqlx::query_as(
        "SELECT name,sql FROM sqlite_schema WHERE type='trigger' AND tbl_name='accounts'",
    )
    .fetch_all(pool.pool())
    .await
    .unwrap();
    for (name, _) in &triggers {
        // SQLite-generated fixture names, not external input.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP TRIGGER \"{}\"",
            name.replace('"', "\"\"")
        )))
        .execute(pool.pool())
        .await
        .unwrap();
    }
    sqlx::query("UPDATE accounts SET path='Assets:corrupt-cache' WHERE handle=(SELECT min(handle) FROM accounts)").execute(pool.pool()).await.unwrap();
    for (_, sql) in triggers {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.as_str()))
            .execute(pool.pool())
            .await
            .unwrap();
    }
    pool.close().await;
    let result = EncryptedStore::open(&path, DatasetKey::new([0x61; 32]), CipherProfileV1).await;
    let rejected = result.is_err();
    if let Ok(store) = result {
        store.close().await.unwrap();
    }
    assert!(
        rejected,
        "startup must not admit a divergent native projection"
    );
    let forensic =
        currents_storage::ForensicStore::open(&path, DatasetKey::new([0x61; 32]), CipherProfileV1)
            .await
            .unwrap();
    assert_eq!(forensic.failure().class, ErrorClass::IntegrityFailure);
    let destination = open(&dir.path().join("rebuilt.db")).await;
    let rebuilt = currents_service::ReplayCoordinator::rebuild(forensic.history(), destination)
        .await
        .unwrap();
    assert_eq!(rebuilt.through, CommitSeq(1));
    assert!(
        !rebuilt.problems.is_empty(),
        "source damage must remain disclosed"
    );
    forensic.close().await;
    assert!(
        EncryptedStore::open(&path, DatasetKey::new([0x61; 32]), CipherProfileV1)
            .await
            .is_err(),
        "rebuild must not repair source in place"
    );
    let read_only = currents_cipher::open_pool_read_only(
        &path,
        DatasetKey::new([0x61; 32]),
        CipherProfileV1,
        1,
    )
    .await
    .unwrap();
    assert!(
        sqlx::query("DELETE FROM accounts")
            .execute(read_only.pool())
            .await
            .is_err()
    );
    read_only.close().await;
    let missing = dir.path().join("missing.db");
    assert!(
        currents_cipher::open_pool_read_only(
            &missing,
            DatasetKey::new([0x61; 32]),
            CipherProfileV1,
            1
        )
        .await
        .is_err()
    );
    assert!(!missing.exists());
    let leaves_path = dir.path().join("native-leaf-cache.db");
    let store = open(&leaves_path).await;
    metadata(&store).await;
    store.close().await.unwrap();
    let pool = currents_cipher::open_pool(
        &leaves_path,
        DatasetKey::new([0x61; 32]),
        CipherProfileV1,
        1,
    )
    .await
    .unwrap();
    let triggers: Vec<(String, String)> = sqlx::query_as(
        "SELECT name,sql FROM sqlite_schema WHERE type='trigger' AND tbl_name='leaves'",
    )
    .fetch_all(pool.pool())
    .await
    .unwrap();
    for (name, _) in &triggers {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP TRIGGER \"{}\"",
            name.replace('"', "\"\"")
        )))
        .execute(pool.pool())
        .await
        .unwrap();
    }
    sqlx::query("UPDATE leaves SET bytes=zeroblob(length(bytes)) WHERE position=1")
        .execute(pool.pool())
        .await
        .unwrap();
    for (_, sql) in triggers {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.as_str()))
            .execute(pool.pool())
            .await
            .unwrap();
    }
    pool.close().await;
    let forensic = currents_storage::ForensicStore::open(
        &leaves_path,
        DatasetKey::new([0x61; 32]),
        CipherProfileV1,
    )
    .await
    .unwrap();
    let destination = open(&dir.path().join("leaf-cache-rebuilt.db")).await;
    let rebuilt = currents_service::ReplayCoordinator::rebuild(forensic.history(), destination)
        .await
        .unwrap();
    assert_eq!(rebuilt.through, CommitSeq(1));
    assert!(!rebuilt.problems.is_empty());
    forensic.close().await;
    for kind in 0..2 {
        let corrupt = dir.path().join(format!("authority-{kind}.db"));
        let store = open(&corrupt).await;
        metadata(&store).await;
        store.close().await.unwrap();
        damage_authority(&corrupt, kind).await;
        let forensic = currents_storage::ForensicStore::open(
            &corrupt,
            DatasetKey::new([0x61; 32]),
            CipherProfileV1,
        )
        .await
        .unwrap();
        let destination = open(&dir.path().join(format!("failed-destination-{kind}.db"))).await;
        assert!(
            currents_service::ReplayCoordinator::rebuild(forensic.history(), destination)
                .await
                .is_err(),
            "missing authority must never become a healthy empty prefix"
        );
        forensic.close().await;
    }
}

#[tokio::test]
async fn cur_011_4() {
    let dir = tempfile::tempdir().unwrap();
    let source = open(&dir.path().join("source.db")).await;
    let empty = source.writer().audited_prefix().await.unwrap();
    let mut state = fixture::initial_state(empty.native_head);
    let (batch, records) = fixture::metadata(&state);
    let commit = append_batch(&source, 1, &batch, records).await;
    fixture::advance(&mut state, &commit, &batch);
    let (_, records) = fixture::originals(&state);
    let AuthoritativeRecordV1::Recognition(recognition) = records[0].clone() else {
        panic!("fixture recognition")
    };
    let batch =
        AccountingAdapter::freeze(&AccountingIntent::Recognition(recognition.clone()), &state)
            .unwrap();
    append_batch(
        &source,
        2,
        &batch,
        vec![AuthoritativeRecordV1::Recognition(recognition)],
    )
    .await;
    let (capture_reference, capture_bytes, products) = append_import_history(&source).await;
    let history = source.history().read_prefix(CommitSeq(4)).await.unwrap();
    let mut projection = DomainState::default();
    for commit in &history {
        currents_domain::ReplayEngine::apply(&mut projection, commit).unwrap();
    }
    for product in &products {
        assert_eq!(
            projection.records.get(&product.id),
            Some(&AuthoritativeRecordV1::AddressedParseProduct(
                product.clone()
            )),
            "replay must index both immutable parse versions by their own identity"
        );
    }
    assert!(
        matches!(
            projection.records.get(&fixture::id(71001)),
            Some(AuthoritativeRecordV1::ObservationVersion(_))
        ),
        "legacy anonymous product must not overwrite source observation identity"
    );
    let mut prefix = DomainState::default();
    for commit in &history[..2] {
        currents_domain::ReplayEngine::apply(&mut prefix, commit).unwrap();
    }
    for invalid_version in [true, false] {
        let mut invalid = history[2].clone();
        for record in &mut invalid.records {
            if let AuthoritativeRecordV1::AddressedParseProduct(product) = record {
                if invalid_version {
                    product.schema_version = 3;
                } else {
                    product.id = product.product.source_version_id;
                }
            }
        }
        let mut unchanged = prefix.clone();
        let error = currents_domain::ReplayEngine::apply(&mut unchanged, &invalid).unwrap_err();
        assert_eq!(
            error.class,
            if invalid_version {
                ErrorClass::UnsupportedVersion
            } else {
                ErrorClass::InvalidReference
            }
        );
        assert_eq!(unchanged.records, prefix.records);
        assert_eq!(unchanged.through, prefix.through);
        assert_eq!(unchanged.domain_digest, prefix.domain_digest);
    }
    let destination_path = dir.path().join("destination.db");
    let destination = open(&destination_path).await;
    let before = source.history().audit_report().await.unwrap();
    let rebuilt = currents_service::ReplayCoordinator::rebuild(source.history(), destination)
        .await
        .unwrap();
    assert_eq!(before, rebuilt);
    assert_eq!(rebuilt.through, CommitSeq(4));
    let reopened = open(&destination_path).await;
    assert_eq!(
        reopened.history().read_prefix(CommitSeq(4)).await.unwrap(),
        history
    );
    let mut stream = reopened
        .history()
        .stream_content(CommitSeq(4), capture_reference.digest)
        .await
        .unwrap();
    let mut actual = Vec::new();
    while let Some(chunk) = stream.next().await.unwrap() {
        actual.extend(chunk);
    }
    assert_eq!(actual, capture_bytes);
    let retained: ImportCaptureMetadataV1 = serde_json::from_slice(&actual).unwrap();
    assert_ne!(
        retained.original_capture_time,
        serde_json::from_str::<RecordedInstant>("\"2026-11-13T14:00:00Z\"").unwrap()
    );
    drop(stream);
    reopened.close().await.unwrap();
    println!("replay-v1 {rebuilt:?}");
    source.close().await.unwrap();
}

#[tokio::test]
async fn cur_011_2() {
    let dir = tempfile::tempdir().unwrap();
    let source = open(&dir.path().join("source.db")).await;
    let commit = metadata(&source).await;
    let mut first = DomainState::default();
    let mut second = DomainState::default();
    currents_domain::ReplayEngine::apply(&mut first, &commit).unwrap();
    currents_domain::ReplayEngine::apply(&mut second, &commit).unwrap();
    assert_eq!(first.through, commit.seq);
    assert_eq!(first.records, second.records);
    assert_eq!(
        first.records.get(&fixture::id(900)),
        Some(&commit.records[0])
    );
    let mut unsupported = commit.clone();
    unsupported.native_head_after.semantics_version = 999;
    let mut untouched = DomainState::default();
    assert_eq!(
        currents_domain::ReplayEngine::apply(&mut untouched, &unsupported)
            .unwrap_err()
            .class,
        ErrorClass::UnsupportedVersion
    );
    assert_eq!(untouched.through, CommitSeq(0));
    assert!(untouched.records.is_empty());
    source.close().await.unwrap();
}

async fn append_batch(
    store: &EncryptedStore,
    n: u128,
    batch: &AccountingBatchV1,
    mut records: Vec<AuthoritativeRecordV1>,
) -> DomainCommitV1 {
    records.push(AuthoritativeRecordV1::FrozenAccountingManifest(
        AccountingAdapter::manifest(fixture::id(90000 + n), batch).unwrap(),
    ));
    store
        .writer()
        .append_closure(
            HistoryAppend {
                prior_head: batch.preconditions.through,
                records,
                decisions: vec![],
                staged: vec![],
                native: None,
                result: CommandResultV1 {
                    command_id: CommandId(fixture::id(90001 + n).0),
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
                },
            },
            AccountingAdapter::prepare_closure(batch).unwrap(),
            &ClosureControl::default(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn cur_011_1() {
    let mut reports = Vec::new();
    for variant in 0..6 {
        let dir = tempfile::tempdir().unwrap();
        let source = open(&dir.path().join("source.db")).await;
        let empty = source.writer().audited_prefix().await.unwrap();
        let mut state = fixture::initial_state(empty.native_head);
        let (batch, records) = fixture::metadata(&state);
        let commit = append_batch(&source, 1, &batch, records).await;
        fixture::advance(&mut state, &commit, &batch);
        let (_, records) = fixture::originals(&state);
        let AuthoritativeRecordV1::Recognition(mut recognition) = records[0].clone() else {
            panic!("fixture recognition")
        };
        match variant {
            1 => recognition.postings.swap(0, 1),
            2 => recognition.postings[0].id = fixture::id(99999),
            3 => {
                for posting in &mut recognition.postings {
                    posting.effective = TemporalValue::Date {
                        civil: time_date("2024-06-29"),
                        original_text: "2024-06-29".into(),
                    };
                }
            }
            _ => {}
        }
        assert_eq!(
            recognition
                .postings
                .iter()
                .map(|p| p.amount.coefficient)
                .sum::<i128>(),
            0
        );
        let batch =
            AccountingAdapter::freeze(&AccountingIntent::Recognition(recognition.clone()), &state)
                .unwrap();
        if variant == 5 {
            recognition.postings[0].id = fixture::id(888888);
        }
        let commit = append_batch(
            &source,
            2,
            &batch,
            vec![AuthoritativeRecordV1::Recognition(recognition)],
        )
        .await;
        fixture::advance(&mut state, &commit, &batch);
        let batch = AccountingAdapter::freeze(
            &AccountingIntent::NoEffect(NoEffectV1 {
                reason: "organization".into(),
            }),
            &state,
        )
        .unwrap();
        let record = AuthoritativeRecordV1::Organization(OrganizationRecordV1 {
            id: fixture::id(50000),
            version: 1,
            kind: OrganizationKind::Party,
            targets: vec![],
            parent: None,
            text: if variant == 4 { "changed" } else { "original" }.into(),
            allocations: vec![],
            supersedes: None,
        });
        append_batch(&source, 3, &batch, vec![record]).await;
        let report = source.history().audit_report().await.unwrap();
        let destination = open(&dir.path().join("destination.db")).await;
        if variant == 5 {
            assert!(
                currents_service::ReplayCoordinator::rebuild(source.history(), destination)
                    .await
                    .is_err(),
                "same balances and matching native heads do not excuse domain/native posting ID disagreement"
            );
            source.close().await.unwrap();
            continue;
        }
        assert_eq!(
            report,
            currents_service::ReplayCoordinator::rebuild(source.history(), destination)
                .await
                .unwrap()
        );
        reports.push(report);
        source.close().await.unwrap();
    }
    for changed in &reports[1..] {
        assert_ne!(reports[0].domain_hash, changed.domain_hash);
    }
    for changed in &reports[1..4] {
        assert_ne!(reports[0].posting_hash, changed.posting_hash);
    }
    assert_eq!(reports[0].posting_hash, reports[4].posting_hash);
}
fn time_date(text: &str) -> Date {
    let value: TemporalValue = serde_json::from_value(
        serde_json::json!({"kind":"Date", "civil":text,"original_text":text}),
    )
    .unwrap();
    match value {
        TemporalValue::Date { civil, .. } => civil,
        _ => panic!("fixture date"),
    }
}

async fn damage_authority(path: &std::path::Path, kind: u8) {
    let pool = currents_cipher::open_pool(path, DatasetKey::new([0x61; 32]), CipherProfileV1, 1)
        .await
        .unwrap();
    let table = if kind == 0 {
        "currents_chunks"
    } else {
        "currents_commits"
    };
    let triggers: Vec<(String, String)> =
        sqlx::query_as("SELECT name,sql FROM sqlite_schema WHERE type='trigger' AND tbl_name=?")
            .bind(table)
            .fetch_all(pool.pool())
            .await
            .unwrap();
    for (name, _) in &triggers {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP TRIGGER \"{}\"",
            name.replace('"', "\"\"")
        )))
        .execute(pool.pool())
        .await
        .unwrap();
    }
    if kind == 0 {
        sqlx::query("DELETE FROM currents_chunks")
            .execute(pool.pool())
            .await
            .unwrap();
    } else {
        sqlx::query("UPDATE currents_commits SET checkpoint='invalid-checkpoint'")
            .execute(pool.pool())
            .await
            .unwrap();
    }
    for (_, sql) in triggers {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.as_str()))
            .execute(pool.pool())
            .await
            .unwrap();
    }
    pool.close().await;
}

async fn append_import_history(
    source: &EncryptedStore,
) -> (ContentReferenceV1, Vec<u8>, Vec<AddressedParseProductV1>) {
    let content = b"synthetic retained capture";
    let raw = source.stage(content).await.unwrap();
    let identity = SourceIdentityV1 {
        source: "synthetic-replay".into(),
        connection_id: fixture::id(71000),
        device_id: None,
    };
    let metadata = ImportCaptureMetadataV1 {
        schema_version: 1,
        receipt_id: fixture::id(71002),
        source_identity: identity.clone(),
        original_capture_time: serde_json::from_str("\"2026-11-12T12:00:00Z\"").unwrap(),
        file_digest: raw.digest(),
        sanitizer_version: "1".into(),
        profile_id: "synthetic".into(),
        parser_version: "1".into(),
        unit: UnitDefinition {
            id: UnitId(fixture::id(1).0),
            exponent: 2,
            native_code: "USD".into(),
            kind: UnitKind::Currency,
        },
    };
    let capture_bytes = serde_json::to_vec(&metadata).unwrap();
    let staged_metadata = source.stage(&capture_bytes).await.unwrap();
    let capture_reference = ContentReferenceV1 {
        digest: staged_metadata.digest(),
        byte_count: CommitSeq(capture_bytes.len() as u64),
        media_type: CAPTURE_METADATA_MEDIA_TYPE.into(),
    };
    let product = ParseProductV1 {
        source_version_id: fixture::id(71001),
        parser_id: "synthetic".into(),
        parser_version: "1".into(),
        facets: vec![],
    };
    let first = AddressedParseProductV1 {
        id: fixture::id(71003),
        schema_version: 1,
        product: product.clone(),
        balance_kinds: vec![],
    };
    let second = AddressedParseProductV1 {
        id: fixture::id(71004),
        schema_version: 1,
        product: ParseProductV1 {
            parser_version: "2".into(),
            ..product.clone()
        },
        balance_kinds: vec![],
    };
    let records = vec![
        AuthoritativeRecordV1::ImportReceipt(ImportReceiptV1 {
            id: metadata.receipt_id,
            source: identity.clone(),
            adapter_version: "1".into(),
            parser_version: "1".into(),
            requested_coverage: vec![],
            reported_coverage: vec![],
            received_time: serde_json::from_str("\"2026-11-13T14:00:00Z\"").unwrap(),
            result: ImportResult::Complete,
            cursor: SourceValue::Unknown,
            truncated: false,
            complete: true,
            contents: vec![
                ContentReferenceV1 {
                    digest: raw.digest(),
                    byte_count: CommitSeq(content.len() as u64),
                    media_type: "text/plain".into(),
                },
                capture_reference.clone(),
            ],
        }),
        AuthoritativeRecordV1::ObservationVersion(ObservationVersionV1 {
            id: product.source_version_id,
            source: identity,
            original_ids: vec!["synthetic-row".into()],
            sanitized_representation: "synthetic retained capture".into(),
            reported_fields: vec![],
            temporal: TemporalValue::Date {
                civil: time_date("2024-06-29"),
                original_text: "2024-06-29".into(),
            },
            semantic_kind: ObservationKind::Movement,
            prior_version: None,
            tombstone_of: None,
            content_digest: raw.digest(),
        }),
        AuthoritativeRecordV1::ParseProduct(product),
        AuthoritativeRecordV1::AddressedParseProduct(first.clone()),
    ];
    let mut result = source.history().read_prefix(CommitSeq(2)).await.unwrap()[1]
        .command_result
        .clone();
    result.command_id = CommandId(fixture::id(71005).0);
    result.digest = Digest([73; 32]);
    result.commit = None;
    let prior_native = source.writer().audited_prefix().await.unwrap().native_head;
    let commit = source
        .writer()
        .append(HistoryAppend {
            prior_head: CommitSeq(2),
            records,
            decisions: vec![],
            staged: vec![raw, staged_metadata],
            native: None,
            result: result.clone(),
        })
        .await
        .unwrap();
    assert_eq!(commit.native_head_after, prior_native);
    result.command_id = CommandId(fixture::id(71006).0);
    result.digest = Digest([74; 32]);
    let commit = source
        .writer()
        .append(HistoryAppend {
            prior_head: CommitSeq(3),
            records: vec![AuthoritativeRecordV1::AddressedParseProduct(second.clone())],
            decisions: vec![],
            staged: vec![],
            native: None,
            result,
        })
        .await
        .unwrap();
    assert_eq!(commit.native_head_after, prior_native);
    (capture_reference, capture_bytes, vec![first, second])
}
