//! Synthetic C04 authority behavior through the private storage API.
use currents_cipher::{CipherProfileV1, DatasetKey};
use currents_storage::{EncryptedStore, HistoryAppend};
use currents_types::*;
use sha2::{Digest as _, Sha256};

fn id(n: u128) -> RecordId {
    RecordId(uuid::Uuid::from_u128(n))
}
fn command(n: u128) -> CommandResultV1 {
    CommandResultV1 {
        command_id: CommandId(id(n).0),
        digest: Digest(Sha256::digest(n.to_be_bytes()).into()),
        state: CommandState::Committed,
        commit: None,
        effects: EffectsV1 {
            records: vec![],
            accounts: vec![],
            amounts: vec![],
            invalidations: vec![],
        },
        health: vec![],
    }
}
fn receipt(n: u128, digest: Digest, size: usize, complete: bool) -> AuthoritativeRecordV1 {
    AuthoritativeRecordV1::ImportReceipt(ImportReceiptV1 {
        id: id(n),
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: id(999),
            device_id: None,
        },
        adapter_version: "1".into(),
        parser_version: "1".into(),
        requested_coverage: vec![],
        reported_coverage: vec![],
        received_time: serde_json::from_str("\"2026-01-01T00:00:00Z\"").unwrap(),
        result: if complete {
            ImportResult::Complete
        } else {
            ImportResult::Partial
        },
        cursor: SourceValue::Known(format!("cursor-{n}")),
        truncated: !complete,
        complete,
        contents: vec![ContentReferenceV1 {
            digest,
            byte_count: CommitSeq(size as u64),
            media_type: "text/plain".into(),
        }],
    })
}
async fn open(path: &std::path::Path) -> EncryptedStore {
    EncryptedStore::open(path, DatasetKey::new([0x47; 32]), CipherProfileV1)
        .await
        .unwrap()
}
async fn append(
    store: &EncryptedStore,
    n: u128,
    prior: u64,
    data: &[u8],
    complete: bool,
) -> DomainCommitV1 {
    append_with_native(store, n, prior, data, complete, None).await
}
async fn append_with_native(
    store: &EncryptedStore,
    n: u128,
    prior: u64,
    data: &[u8],
    complete: bool,
    native: Option<currents_accounting::PreparedNativeOperation>,
) -> DomainCommitV1 {
    let staged = store.stage(data).await.unwrap();
    let record = receipt(n, staged.digest(), data.len(), complete);
    let policy = AuthoritativeRecordV1::DatasetPolicy(DatasetPolicyV1 {
        id: id(500),
        version: n as u32,
        policies: PolicyVersionsV1 {
            selection: 1,
            recognition: 1,
            query: 1,
            calendar: 1,
            timezone_data: "synthetic".into(),
        },
        source_precedence: vec![],
        freshness_seconds: CommitSeq(1),
        opening_policy: id(500),
        withholding_policy: id(500),
    });
    let observation = AuthoritativeRecordV1::ObservationVersion(ObservationVersionV1 {
        id: id(n + 1000),
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: id(999),
            device_id: None,
        },
        original_ids: vec!["source-record".into()],
        sanitized_representation: String::from_utf8(data.to_vec()).unwrap(),
        reported_fields: vec![],
        temporal: TemporalValue::Unknown {
            original_text: String::new(),
        },
        semantic_kind: ObservationKind::Movement,
        prior_version: if prior == 0 {
            None
        } else {
            Some(id(u128::from(prior) + 1000))
        },
        tombstone_of: None,
        content_digest: staged.digest(),
    });
    let decision = DecisionEnvelopeV1 {
        id: id(n + 2000),
        commit: CommitSeq(prior + 1),
        item_position: 3,
        decision_type: DecisionKind::Review,
        schema_version: 1,
        actor: id(800),
        authority: AuthorityKind::Owner,
        grant_id: None,
        command_id: command(n).command_id,
        command_digest: command(n).digest,
        references: vec![id(n), id(n + 1000)],
        recorded_time: serde_json::from_str("\"2026-01-01T00:00:00Z\"").unwrap(),
        adopted_result: vec![id(n + 1000)],
        policy: PolicyReferenceV1 {
            id: id(500),
            version: n as u32,
        },
        corrections: vec![],
        supersedes: vec![],
    };
    store
        .writer()
        .append(HistoryAppend {
            prior_head: CommitSeq(prior),
            records: vec![record, policy, observation],
            decisions: vec![decision],
            result: command(n),
            staged: vec![staged],
            native,
        })
        .await
        .unwrap()
}
async fn assert_native_prefix_visibility() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("native-prefix.db")).await;
    let first = append(&store, 1, 0, b"first", true).await;
    let mut pinned = store.history().stream_prefix(first.seq).await.unwrap();
    let second = append_with_native(
        &store,
        2,
        1,
        b"second",
        true,
        Some(currents_accounting::test_fixtures::currency_declaration()),
    )
    .await;
    assert_ne!(first.native_head_after, second.native_head_after);

    // C was pinned before the native-changing publication of C+1.
    assert_eq!(pinned.next().await.unwrap().unwrap(), first);
    assert!(pinned.next().await.unwrap().is_none());
    assert_eq!(
        store.history().read_prefix(first.seq).await.unwrap(),
        vec![first.clone()]
    );
    assert_eq!(
        store.history().read_prefix(second.seq).await.unwrap(),
        vec![first, second]
    );
    drop(pinned);
    store.close().await.unwrap();
}
#[tokio::test]
async fn prefix_visibility() {
    assert_native_prefix_visibility().await;
    let output = std::process::Command::new("cargo")
        .args([
            "test",
            "--locked",
            "-p",
            "currents-accounting",
            "authority::tests::native_head_mapping",
            "--",
            "--exact",
        ])
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed; 0 ignored"));

    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("data.db")).await;
    let first = append(&store, 1, 0, b"first", true).await;
    let mut stream = store.history().stream_prefix(CommitSeq(1)).await.unwrap();
    let second = append(&store, 2, 1, b"second", true).await;
    assert_eq!(first.native_head_after, second.native_head_after);
    assert_eq!(stream.next().await.unwrap().unwrap(), first);
    assert!(stream.next().await.unwrap().is_none());
    let invalid = AuthoritativeRecordV1::Review(ReviewRecordV1 {
        id: id(3),
        targets: vec![id(404)],
        disposition: ReviewDisposition::Accepted,
        reason: "synthetic".into(),
    });
    assert!(
        store
            .writer()
            .append(HistoryAppend {
                prior_head: CommitSeq(2),
                records: vec![invalid],
                decisions: vec![],
                result: command(3),
                staged: vec![],
                native: None
            })
            .await
            .is_err()
    );
    let AuthoritativeRecordV1::ObservationVersion(mut wrong) = second.records[2].clone() else {
        panic!("observation")
    };
    wrong.id = id(400);
    wrong.prior_version = Some(id(1));
    assert!(
        store
            .writer()
            .append(HistoryAppend {
                prior_head: CommitSeq(2),
                records: vec![AuthoritativeRecordV1::ObservationVersion(wrong)],
                decisions: vec![],
                result: command(4),
                staged: vec![],
                native: None
            })
            .await
            .is_err()
    );
    assert!(store.history().read_prefix(CommitSeq(3)).await.is_err());
    assert_eq!(
        store
            .history()
            .read_prefix(CommitSeq(2))
            .await
            .unwrap()
            .len(),
        2
    );
    let fixtures: Vec<AuthoritativeRecordV1> =
        serde_json::from_str(include_str!("../../../fixtures/contracts/v1/records.json")).unwrap();
    let mut recognition = fixtures
        .iter()
        .find(|r| matches!(r, AuthoritativeRecordV1::Recognition(_)))
        .unwrap()
        .clone();
    if let AuthoritativeRecordV1::Recognition(r) = &mut recognition {
        r.date_policy = PolicyReferenceV1 {
            id: id(500),
            version: 1,
        };
    }
    let mut commitment = fixtures
        .iter()
        .find(|r| matches!(r, AuthoritativeRecordV1::Commitment(_)))
        .unwrap()
        .clone();
    if let AuthoritativeRecordV1::Commitment(c) = &mut commitment {
        c.settlement = SettlementRuleV1::HolidayCalendar(PolicyReferenceV1 {
            id: id(500),
            version: 1,
        });
    }
    for unsupported in [recognition, commitment] {
        let err = store
            .writer()
            .append(HistoryAppend {
                prior_head: CommitSeq(2),
                records: vec![unsupported],
                decisions: vec![],
                result: command(5),
                staged: vec![],
                native: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            currents_storage::CommitFailure::DefinitelyRolledBack(EngineError {
                class: ErrorClass::UnsupportedVersion,
                ..
            })
        ));
    }
    let mut decision = second.decisions[0].clone();
    decision.id = id(6000);
    decision.commit = CommitSeq(3);
    decision.item_position = 0;
    decision.command_id = command(6).command_id;
    decision.command_digest = command(6).digest;
    decision.policy.version = 99;
    assert!(
        store
            .writer()
            .append(HistoryAppend {
                prior_head: CommitSeq(2),
                records: vec![],
                decisions: vec![decision],
                result: command(6),
                staged: vec![],
                native: None
            })
            .await
            .is_err()
    );
    let mut content = store
        .history()
        .stream_content(CommitSeq(1), Digest(Sha256::digest(b"first").into()))
        .await
        .unwrap();
    assert_eq!(content.next().await.unwrap().unwrap(), b"first");
    assert!(content.next().await.unwrap().is_none());
    assert!(
        store
            .history()
            .stream_content(CommitSeq(1), Digest(Sha256::digest(b"second").into()))
            .await
            .is_err()
    );
    drop(content);
    drop(stream);
    store.close().await.unwrap();
}
#[tokio::test]
async fn duplicate_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir.path().join("data.db")).await;
    append(&store, 1, 0, b"same", true).await;
    append(&store, 2, 1, b"same", true).await;
    append(&store, 3, 2, b"changed", true).await;
    let commits = store.history().read_prefix(CommitSeq(3)).await.unwrap();
    assert_eq!(commits.len(), 3);
    assert_eq!(
        store.history().content_count(CommitSeq(3)).await.unwrap(),
        2
    );
    let pool = tamper_pool(&dir.path().join("data.db")).await;
    for sql in [
        "UPDATE currents_items SET canonical=x'00'",
        "DELETE FROM currents_items",
        "DELETE FROM currents_blobs",
        "UPDATE currents_commits SET digest=zeroblob(32)",
    ] {
        assert!(
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                .execute(pool.pool())
                .await
                .is_err()
        );
    }
    pool.close().await;
    let AuthoritativeRecordV1::ObservationVersion(changed) = &commits[2].records[2] else {
        panic!("version")
    };
    assert_eq!(changed.prior_version, Some(id(1002)));
    assert_ne!(commits[0].records[2], commits[2].records[2]);
    store.close().await.unwrap();
}
#[tokio::test]
async fn staging_failure() {
    if let Some(path) = std::env::var_os("CUR008_CRASH_PATH") {
        let store = open(std::path::Path::new(&path)).await;
        let _stage = store.stage(b"CUR008_SYNTHETIC_CANARY").await.unwrap();
        std::process::exit(71);
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "staging_failure"])
        .env("CUR008_CRASH_PATH", &path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(71));
    let store = open(&path).await;
    let _unpublished = store.stage(b"CUR008_SYNTHETIC_CANARY").await.unwrap();
    assert!(
        store
            .history()
            .read_prefix(CommitSeq(0))
            .await
            .unwrap()
            .is_empty()
    );
    store.close().await.unwrap();
    let store = open(&path).await;
    assert!(
        store
            .history()
            .read_prefix(CommitSeq(0))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(store.stage(&vec![0; 100 * 1_048_576 + 1]).await.is_err());
    let corrupt = store.stage(b"corrupt-staging").await.unwrap();
    let pool = tamper_pool(&path).await;
    sqlx::query("UPDATE currents_staging_chunks SET bytes=x'ff'")
        .execute(pool.pool())
        .await
        .unwrap();
    pool.close().await;
    let record = receipt(7, corrupt.digest(), 15, false);
    assert!(
        store
            .writer()
            .append(HistoryAppend {
                prior_head: CommitSeq(0),
                records: vec![record],
                decisions: vec![],
                result: command(7),
                staged: vec![corrupt],
                native: None
            })
            .await
            .is_err()
    );
    assert!(
        store
            .history()
            .read_prefix(CommitSeq(0))
            .await
            .unwrap()
            .is_empty()
    );
    let c = append(&store, 1, 0, b"page-one", false).await;
    let AuthoritativeRecordV1::ImportReceipt(r) = &c.records[0] else {
        panic!("receipt missing")
    };
    assert!(!r.complete);
    store.close().await.unwrap();
    assert!(
        !std::fs::read(path)
            .unwrap()
            .windows(b"CUR008_SYNTHETIC_CANARY".len())
            .any(|v| v == b"CUR008_SYNTHETIC_CANARY")
    );
}
async fn assert_domain_chain_continuity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("domain-chain.db");
    let store = open(&path).await;
    let first = append(&store, 1, 0, b"first evidence", true).await;
    let second = append(&store, 2, 1, b"second evidence", true).await;
    assert_eq!(second.previous_digest, first.digest);
    assert_eq!(
        store.history().read_prefix(second.seq).await.unwrap(),
        vec![first.clone(), second.clone()]
    );

    // Break only the link to the prior domain commit. Keep the second commit's
    // canonical digest and stored columns consistent; its items remain unchanged.
    let mut unlinked = second;
    unlinked.previous_digest = Digest::default();
    assert_ne!(unlinked.previous_digest, first.digest);
    unlinked.digest = Digest::default();
    unlinked.digest =
        Digest(Sha256::digest(serde_json_canonicalizer::to_vec(&unlinked).unwrap()).into());
    let pool = tamper_pool(&path).await;
    sqlx::raw_sql("DROP TRIGGER currents_commits_no_update")
        .execute(pool.pool())
        .await
        .unwrap();
    let updated = sqlx::query(
        "UPDATE currents_commits SET canonical=?,digest=?,previous_digest=? WHERE seq=2",
    )
    .bind(serde_json_canonicalizer::to_vec(&unlinked).unwrap())
    .bind(unlinked.digest.0.as_slice())
    .bind(unlinked.previous_digest.0.as_slice())
    .execute(pool.pool())
    .await
    .unwrap();
    assert_eq!(updated.rows_affected(), 1);
    pool.close().await;

    let mut stream = store.history().stream_prefix(unlinked.seq).await.unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap(), first);
    assert_eq!(
        stream.next().await.unwrap_err().class,
        ErrorClass::IntegrityFailure
    );
    drop(stream);
    store.close().await.unwrap();
}

#[tokio::test]
async fn chain_tamper() {
    assert_domain_chain_continuity().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");
    let store = open(&path).await;
    let c = append(&store, 1, 0, b"evidence", true).await;
    assert!(
        store
            .history()
            .verify_checkpoint(CommitSeq(1), c.digest)
            .await
            .is_ok()
    );
    assert!(
        store
            .history()
            .verify_checkpoint(CommitSeq(1), Digest([0x55; 32]))
            .await
            .is_err()
    );
    // A privileged attacker can recompute every local digest. Only an independent
    // saved checkpoint distinguishes that coherent rewrite from the original.
    let pool = tamper_pool(&path).await;
    sqlx::raw_sql(
        "DROP TRIGGER currents_commits_no_update; DROP TRIGGER currents_items_no_update;",
    )
    .execute(pool.pool())
    .await
    .unwrap();
    let mut rewritten = c.clone();
    let AuthoritativeRecordV1::ImportReceipt(r) = &mut rewritten.records[0] else {
        panic!("receipt")
    };
    r.cursor = SourceValue::Known("rewritten".into());
    rewritten.digest = Digest::default();
    rewritten.digest =
        Digest(Sha256::digest(serde_json_canonicalizer::to_vec(&rewritten).unwrap()).into());
    sqlx::query("UPDATE currents_commits SET canonical=?,digest=? WHERE seq=1")
        .bind(serde_json_canonicalizer::to_vec(&rewritten).unwrap())
        .bind(rewritten.digest.0.as_slice())
        .execute(pool.pool())
        .await
        .unwrap();
    let item = serde_json_canonicalizer::to_vec(&rewritten.records[0]).unwrap();
    let digest: [u8; 32] = Sha256::digest(&item).into();
    sqlx::query("UPDATE currents_items SET canonical=?,digest=? WHERE seq=1 AND position=0")
        .bind(item)
        .bind(digest.as_slice())
        .execute(pool.pool())
        .await
        .unwrap();
    assert!(store.history().read_prefix(CommitSeq(1)).await.is_ok());
    assert!(
        store
            .history()
            .verify_checkpoint(CommitSeq(1), c.digest)
            .await
            .is_err()
    );
    pool.close().await;
    store.close().await.unwrap();
    for mutation in [
        "DROP TRIGGER currents_commits_no_update; UPDATE currents_commits SET canonical=x'00' WHERE seq=1",
        "DROP TRIGGER currents_items_no_update; UPDATE currents_items SET digest=zeroblob(32) WHERE seq=1",
        "DROP TRIGGER currents_chunks_no_delete; DELETE FROM currents_chunks",
        "DROP TRIGGER currents_references_no_delete; DELETE FROM currents_references",
        "DROP TRIGGER currents_references_no_update; UPDATE currents_references SET target_kind='ObservationVersion',target_id='00000000-0000-0000-0000-0000000003e9',target_version=0 WHERE rowid=(SELECT min(rowid) FROM currents_references WHERE position=1)",
        "INSERT INTO currents_definitions(kind,id,version,seq,position) VALUES('Review','00000000-0000-0000-0000-000000000194',0,1,0)",
        "DROP TRIGGER currents_items_no_update; UPDATE currents_items SET canonical=(SELECT canonical FROM currents_items WHERE position=1) WHERE position=0",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("tampered.db");
        let store = open(&file).await;
        append(&store, 1, 0, b"evidence", true).await;
        let pool = tamper_pool(&file).await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(mutation))
            .execute(pool.pool())
            .await
            .unwrap();
        pool.close().await;
        assert!(store.history().read_prefix(CommitSeq(1)).await.is_err());
        store.close().await.unwrap();
    }
}

async fn tamper_pool(path: &std::path::Path) -> currents_cipher::CipherPool {
    currents_cipher::open_pool(path, DatasetKey::new([0x47; 32]), CipherProfileV1, 1)
        .await
        .unwrap()
}
