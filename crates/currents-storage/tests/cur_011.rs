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
    let destination = open(&dir.path().join("destination.db")).await;
    let before = source.history().audit_report().await.unwrap();
    let rebuilt = currents_service::ReplayCoordinator::rebuild(source.history(), destination)
        .await
        .unwrap();
    assert_eq!(before, rebuilt);
    assert_eq!(rebuilt.through, CommitSeq(2));
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
            kind: OrganizationKind::Note,
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
