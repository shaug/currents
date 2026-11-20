//! Synthetic C04 acceptance. Native-limit assertions use only the private seam;
//! durable closure assertions use real accepted C05 manifests end to end.
#[path = "support/closure.rs"]
mod fixtures;
use currents_accounting::{AccountingAdapter, AtomicStore, ClosureControl, test_fixtures};
use currents_cipher::{CipherProfileV1, DatasetKey};
use currents_storage::{EncryptedStore, HistoryAppend};
use currents_types::*;
fn same_commit(a: &DomainCommitV1, b: &DomainCommitV1) {
    assert_eq!(a.digest, b.digest);
    assert!(
        serde_json_canonicalizer::to_vec(a).unwrap()
            == serde_json_canonicalizer::to_vec(b).unwrap(),
        "canonical commit identity differs"
    );
}
fn input(n: u128, batch: &AccountingBatchV1, records: &[AuthoritativeRecordV1]) -> HistoryAppend {
    let mut records = records.to_vec();
    records.push(AuthoritativeRecordV1::FrozenAccountingManifest(
        AccountingAdapter::manifest(RecordId(uuid::Uuid::from_u128(900_000 + n)), batch).unwrap(),
    ));
    HistoryAppend {
        prior_head: batch.preconditions.through,
        records,
        decisions: vec![],
        staged: vec![],
        native: None,
        result: CommandResultV1 {
            command_id: CommandId(uuid::Uuid::from_u128(n)),
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
    }
}
async fn open(path: &std::path::Path) -> EncryptedStore {
    EncryptedStore::open(path, DatasetKey::new([0x37; 32]), CipherProfileV1)
        .await
        .unwrap()
}
async fn pool(path: &std::path::Path) -> currents_cipher::CipherPool {
    currents_cipher::open_pool(path, DatasetKey::new([0x37; 32]), CipherProfileV1, 1)
        .await
        .unwrap()
}
async fn leaves(pool: &sqlx::SqlitePool) -> Vec<Vec<u8>> {
    sqlx::query_scalar("SELECT bytes FROM leaves ORDER BY position")
        .fetch_all(pool)
        .await
        .unwrap()
}
async fn append(
    store: &EncryptedStore,
    n: u128,
    batch: &AccountingBatchV1,
    records: &[AuthoritativeRecordV1],
    control: &ClosureControl,
) -> Result<DomainCommitV1, currents_storage::CommitFailure> {
    store
        .writer()
        .append_closure(
            input(n, batch, records),
            AccountingAdapter::prepare_closure(batch).unwrap(),
            control,
        )
        .await
}
async fn seed(store: &EncryptedStore, path: &std::path::Path) -> DomainState {
    let reader = pool(path).await;
    let native = AtomicStore::from_initialized_pool(reader.pool().clone(), false)
        .await
        .unwrap();
    let tx = native
        .begin_authority(CommandId(uuid::Uuid::from_u128(999999)), None)
        .await
        .unwrap();
    let mut state = fixtures::initial_state(tx.native_head());
    tx.rollback().await.unwrap();
    reader.close().await;
    for (n, prepare) in [
        (
            1,
            fixtures::metadata
                as fn(&DomainState) -> (AccountingBatchV1, Vec<AuthoritativeRecordV1>),
        ),
        (
            2,
            fixtures::originals
                as fn(&DomainState) -> (AccountingBatchV1, Vec<AuthoritativeRecordV1>),
        ),
    ] {
        let (batch, records) = prepare(&state);
        let commit = append(store, n, &batch, &records, &ClosureControl::default())
            .await
            .unwrap();
        fixtures::advance(&mut state, &commit, &batch);
    }
    state
}
#[tokio::test]
async fn cur_007_1() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.db");
    let store = open(&path).await;
    let state = seed(&store, &path).await;
    let (batch, records) = fixtures::correction(&state);
    assert_eq!(batch.operations.len(), 1200);
    let reader = pool(&path).await;
    let before = leaves(reader.pool()).await;
    let control = test_fixtures::fault_control(test_fixtures::ClosureFault::PauseAfter600);
    let observe = async {
        while control.completed_operations() < 600 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            leaves(reader.pool()).await,
            before,
            "reader observed an intermediate chunk"
        );
        test_fixtures::release_control(&control);
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        tokio::join!(append(&store, 3, &batch, &records, &control), observe)
    })
    .await
    .expect("midpoint was not reached");
    let committed = result.unwrap();
    assert_eq!(committed.seq, CommitSeq(3));
    let after = leaves(reader.pool()).await;
    assert_eq!(after.len(), before.len() + 1200);
    let retry = append(&store, 3, &batch, &records, &ClosureControl::default())
        .await
        .unwrap();
    same_commit(&retry, &committed);
    assert_eq!(leaves(reader.pool()).await, after);
    // Same digest alone cannot authorize different manifest identity or records.
    let mut changed = input(3, &batch, &records);
    let AuthoritativeRecordV1::FrozenAccountingManifest(m) = changed.records.last_mut().unwrap()
    else {
        panic!()
    };
    m.id = RecordId(uuid::Uuid::from_u128(999998));
    assert!(
        store
            .writer()
            .append_closure(
                changed,
                AccountingAdapter::prepare_closure(&batch).unwrap(),
                &ClosureControl::default()
            )
            .await
            .is_err()
    );
    // Validate content binding, prebinding digest, prefix and fixed expectations.
    for case in 0..8 {
        let mut bad = input(4, &batch, &records);
        let AuthoritativeRecordV1::FrozenAccountingManifest(m) = bad.records.last_mut().unwrap()
        else {
            panic!()
        };
        match case {
            0 => m.instructions.digest = Digest([7; 32]),
            1 => m.instructions.byte_count.0 += 1,
            2 => m.decision_digest = Digest([7; 32]),
            3 => m.preconditions.through = CommitSeq(123),
            4 => m.result_expectations.financial_entry_count.0 += 1,
            5 => m.result_expectations.native_operation_count.0 += 1,
            6 => {
                bad.records.pop();
            }
            _ => {} // A valid but stale prefix must fail inside the locked transaction.
        }
        let control = ClosureControl::default();
        assert!(
            store
                .writer()
                .append_closure(
                    bad,
                    AccountingAdapter::prepare_closure(&batch).unwrap(),
                    &control
                )
                .await
                .is_err()
        );
        assert!(!control.request_cancel());
        assert_eq!(leaves(reader.pool()).await, after);
    }
    assert!(
        store
            .writer()
            .append(input(4, &batch, &records))
            .await
            .is_err(),
        "single-op path must not bypass manifest execution"
    );
    // Exact canonical instructions are retained as encrypted content, not inline.
    let manifest =
        AccountingAdapter::manifest(RecordId(uuid::Uuid::from_u128(900003)), &batch).unwrap();
    let mut content = store
        .history()
        .stream_content(CommitSeq(3), manifest.instructions.digest)
        .await
        .unwrap();
    let mut recovered = Vec::new();
    while let Some(chunk) = content.next().await.unwrap() {
        recovered.extend(chunk);
    }
    assert_eq!(
        recovered,
        AccountingAdapter::instruction_bytes(&batch).unwrap()
    );
    drop(content);
    // Independent native replay, same validated frozen instructions, one operation at a time.
    let replay_path = dir.path().join("replay.db");
    let replay = open(&replay_path).await;
    let replay_state = seed(&replay, &replay_path).await;
    assert_eq!(
        replay_state.accounting.as_ref().unwrap().prefix,
        state.accounting.as_ref().unwrap().prefix
    );
    let replay_pool = pool(&replay_path).await;
    let native = AtomicStore::from_initialized_pool(replay_pool.pool().clone(), false)
        .await
        .unwrap();
    let mut tx = native
        .begin_authority(CommandId(uuid::Uuid::from_u128(999997)), None)
        .await
        .unwrap();
    for operation in AccountingAdapter::prepare_operations(&batch).unwrap() {
        tx.apply_native(&operation).await.unwrap();
    }
    tx.commit().await.unwrap();
    assert_eq!(
        leaves(replay_pool.pool()).await,
        after,
        "native leaves differ across partitioning"
    );
    replay_pool.close().await;
    replay.close().await.unwrap();
    reader.close().await;
    store.close().await.unwrap();
    open(&path).await.close().await.unwrap();
}
async fn kill_child(path: &std::path::Path) {
    let (batch, records): (AccountingBatchV1, Vec<AuthoritativeRecordV1>) =
        serde_json::from_slice(&std::fs::read(path.with_extension("fixture")).unwrap()).unwrap();
    let store = open(path).await;
    let control = test_fixtures::fault_control(test_fixtures::ClosureFault::PauseAfter1200);
    let marker = async {
        while control.completed_operations() < 1200 {
            tokio::task::yield_now().await;
        }
        use std::io::Write;
        println!("CUR007_FINAL_CHUNK_UNCOMMITTED");
        std::io::stdout().flush().unwrap();
        std::future::pending::<()>().await;
    };
    let _ = tokio::join!(append(&store, 3, &batch, &records, &control), marker);
}
fn kill_at_final_chunk(path: &std::path::Path) {
    use std::io::BufRead;
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cur_007_2", "--nocapture"])
        .env("CUR007_SYNTHETIC_KILL_CHILD", path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if line
                .unwrap_or_default()
                .contains("CUR007_FINAL_CHUNK_UNCOMMITTED")
            {
                let _ = send.send(());
                return;
            }
        }
    });
    let ready = receive
        .recv_timeout(std::time::Duration::from_secs(60))
        .is_ok();
    let _ = child.kill();
    let status = child.wait().unwrap();
    reader.join().unwrap();
    assert!(ready, "child never reached final uncommitted chunk");
    assert!(!status.success());
}
#[tokio::test]
async fn cur_007_2() {
    if let Some(path) = std::env::var_os("CUR007_SYNTHETIC_KILL_CHILD") {
        kill_child(std::path::Path::new(&path)).await;
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.db");
    let store = open(&path).await;
    let state = seed(&store, &path).await;
    let (batch, records) = fixtures::correction(&state);
    let reader = pool(&path).await;
    let before = leaves(reader.pool()).await;
    let control = test_fixtures::fault_control(test_fixtures::ClosureFault::DiskFullAfter600);
    assert!(append(&store, 3, &batch, &records, &control).await.is_err());
    assert!(test_fixtures::observed_sqlite_full(&control));
    assert_eq!(control.completed_operations(), 600);
    reader.close().await;
    store.close().await.unwrap();
    let store = open(&path).await;
    assert!(
        store
            .writer()
            .command_result(CommandId(uuid::Uuid::from_u128(3)), Digest([3; 32]))
            .await
            .unwrap()
            .is_none()
    );
    store.close().await.unwrap();
    std::fs::write(
        path.with_extension("fixture"),
        serde_json::to_vec(&(&batch, &records)).unwrap(),
    )
    .unwrap();
    kill_at_final_chunk(&path);
    let store = open(&path).await;
    let reader = pool(&path).await;
    assert_eq!(leaves(reader.pool()).await, before);
    assert!(
        store
            .writer()
            .command_result(CommandId(uuid::Uuid::from_u128(3)), Digest([3; 32]))
            .await
            .unwrap()
            .is_none()
    );
    // Invalid final domain references fail after all native chunks, then roll back both.
    let mut invalid = input(3, &batch, &records);
    invalid
        .result
        .effects
        .records
        .push(RecordId(uuid::Uuid::from_u128(123456789)));
    let control = ClosureControl::default();
    assert!(matches!(
        store
            .writer()
            .append_closure(
                invalid,
                AccountingAdapter::prepare_closure(&batch).unwrap(),
                &control
            )
            .await,
        Err(currents_storage::CommitFailure::DefinitelyRolledBack(_))
    ));
    assert_eq!(control.completed_operations(), 1200);
    assert_eq!(leaves(reader.pool()).await, before);
    let committed = append(&store, 3, &batch, &records, &ClosureControl::default())
        .await
        .unwrap();
    same_commit(
        &append(&store, 3, &batch, &records, &ClosureControl::default())
            .await
            .unwrap(),
        &committed,
    );
    assert_eq!(leaves(reader.pool()).await.len(), before.len() + 1200);
    reader.close().await;
    store.close().await.unwrap();
    open(&path).await.close().await.unwrap();
}
#[tokio::test]
async fn cur_007_3() {
    use test_fixtures::NativeLimitCase::*;
    // Native and opaque execution-boundary qualification only. These fixed raw
    // fixtures are deliberately NOT accepted by the production manifest writer.
    for (case, accepted) in [
        (Batch1000, true),
        (Batch1001, false),
        (Postings1024, true),
        (Postings1025, false),
        (Documents16, true),
        (Documents17, false),
        (Leaf65535, true),
        (Leaf65536, false),
        (Closure20000, true),
        (Closure20001, false),
        (Bytes64MiB, true),
        (Bytes64MiBPlus1, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("native.db");
        let pool = pool(&path).await;
        let native = AtomicStore::from_initialized_pool(pool.pool().clone(), true)
            .await
            .unwrap();
        let mut tx = native
            .begin_authority(CommandId(uuid::Uuid::from_u128(1)), None)
            .await
            .unwrap();
        for operation in test_fixtures::closure_metadata() {
            tx.apply_native(&operation).await.unwrap();
        }
        tx.commit().await.unwrap();
        let before = leaves(pool.pool()).await;
        let batch = test_fixtures::limit_case(case);
        let size = batch.instruction_bytes().len();
        let logical = batch.operation_count().unwrap();
        let control = ClosureControl::default();
        let mut tx = native
            .begin_authority(CommandId(uuid::Uuid::from_u128(2)), None)
            .await
            .unwrap();
        let result = tx.apply_closure(&batch, &control).await;
        assert_eq!(
            result.is_ok(),
            accepted,
            "size={size},operations={logical}: {result:?}"
        );
        if accepted {
            tx.commit().await.unwrap();
        } else {
            tx.rollback().await.unwrap();
            assert_eq!(leaves(pool.pool()).await, before);
        }
        if size > currents_accounting::MAX_CLOSURE_BYTES || logical > 20000 {
            assert_eq!(control.completed_operations(), 0);
        }
        pool.close().await;
    }
}
#[tokio::test]
async fn cur_007_4() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.db");
    let store = open(&path).await;
    let state = seed(&store, &path).await;
    let (batch, records) = fixtures::correction(&state);
    let control = test_fixtures::fault_control(test_fixtures::ClosureFault::PauseAfter600);
    let cancel = async {
        while control.completed_operations() < 600 {
            tokio::task::yield_now().await;
        }
        assert!(control.request_cancel());
    };
    let (outcome, ()) = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        tokio::join!(append(&store, 3, &batch, &records, &control), cancel)
    })
    .await
    .expect("cancellation checkpoint not reached");
    assert!(matches!(
        outcome,
        Err(currents_storage::CommitFailure::DefinitelyRolledBack(
            EngineError {
                class: ErrorClass::Canceled,
                ..
            }
        ))
    ));
    assert_eq!(
        store
            .history()
            .read_prefix(CommitSeq(2))
            .await
            .unwrap()
            .len(),
        2
    );
    let control =
        test_fixtures::fault_control(test_fixtures::ClosureFault::LostCommitAcknowledgment);
    let race = async {
        while !control.commit_submitted() {
            tokio::task::yield_now().await;
        }
        assert!(!control.request_cancel());
        test_fixtures::release_control(&control);
    };
    let (outcome, ()) = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        tokio::join!(append(&store, 3, &batch, &records, &control), race)
    })
    .await
    .expect("COMMIT checkpoint not reached");
    assert!(matches!(
        outcome,
        Err(currents_storage::CommitFailure::OutcomeUnknown(_))
    ));
    store.close().await.unwrap();
    let store = open(&path).await;
    let resolved = store
        .writer()
        .command_result(CommandId(uuid::Uuid::from_u128(3)), Digest([3; 32]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.seq, CommitSeq(3));
    assert_eq!(
        append(&store, 3, &batch, &records, &ClosureControl::default())
            .await
            .unwrap(),
        resolved
    );
    let canceled = ClosureControl::default();
    assert!(canceled.request_cancel());
    assert_eq!(
        append(&store, 3, &batch, &records, &canceled)
            .await
            .unwrap(),
        resolved,
        "a canceled retry cannot claim a previous commit rolled back"
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn cur_015_multiple_corrections_never_publish_a_partial_native_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-association-remap.db");
    let store = open(&path).await;
    let state = seed(&store, &path).await;
    let (batch, records) = fixtures::association_corrections(&state);
    assert_eq!(batch.operations.len(), 1200);
    let reader = pool(&path).await;
    let before = leaves(reader.pool()).await;
    let fault = test_fixtures::fault_control(test_fixtures::ClosureFault::DiskFullAfter600);
    assert!(append(&store, 3, &batch, &records, &fault).await.is_err());
    assert!(test_fixtures::observed_sqlite_full(&fault));
    assert_eq!(fault.completed_operations(), 600);
    assert_eq!(leaves(reader.pool()).await, before);
    reader.close().await;
    store.close().await.unwrap();
    let store = open(&path).await;
    assert!(
        store
            .writer()
            .command_result(CommandId(uuid::Uuid::from_u128(3)), Digest([3; 32]))
            .await
            .unwrap()
            .is_none()
    );
    let reader = pool(&path).await;
    assert_eq!(leaves(reader.pool()).await, before);
    let committed = append(&store, 3, &batch, &records, &ClosureControl::default())
        .await
        .unwrap();
    assert_eq!(committed.seq, CommitSeq(3));
    let after = leaves(reader.pool()).await;
    assert_ne!(after, before);
    let retried = append(&store, 3, &batch, &records, &ClosureControl::default())
        .await
        .unwrap();
    same_commit(&committed, &retried);
    assert_eq!(leaves(reader.pool()).await, after);
    reader.close().await;
    store.close().await.unwrap();
}

#[tokio::test]
async fn cur_015_initial_multidate_batch_is_atomic_and_replayable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-initial-association.db");
    let store = open(&path).await;
    let reader = pool(&path).await;
    let native = AtomicStore::from_initialized_pool(reader.pool().clone(), false)
        .await
        .unwrap();
    let tx = native
        .begin_authority(CommandId(uuid::Uuid::from_u128(999999)), None)
        .await
        .unwrap();
    let mut state = fixtures::initial_state(tx.native_head());
    tx.rollback().await.unwrap();
    reader.close().await;
    let (metadata, metadata_records) = fixtures::metadata(&state);
    let metadata_commit = append(
        &store,
        1,
        &metadata,
        &metadata_records,
        &ClosureControl::default(),
    )
    .await
    .unwrap();
    fixtures::advance(&mut state, &metadata_commit, &metadata);
    let (batch, records) = fixtures::association_initial(&state);
    // Page cap is armed after600. A full second chunk forces native page growth;
    // a single extra entry may fit a page and only fail later in history append.
    assert_eq!(batch.operations.len(), 1200);
    let dates: std::collections::BTreeSet<_> = batch
        .operations
        .iter()
        .filter_map(|op| match op {
            FrozenNativeOperationV1::Entry(entry) => Some(entry.value_date.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(dates, ["2024-06-30", "2024-07-01"].into_iter().collect());
    let reader = pool(&path).await;
    let before = leaves(reader.pool()).await;
    let fault = test_fixtures::fault_control(test_fixtures::ClosureFault::DiskFullAfter600);
    let failed = append(&store, 2, &batch, &records, &fault).await;
    assert!(failed.is_err());
    assert!(
        test_fixtures::observed_sqlite_full(&fault),
        "expected injected fault, observed {failed:?}; completed {}",
        fault.completed_operations()
    );
    assert_eq!(leaves(reader.pool()).await, before);
    reader.close().await;
    store.close().await.unwrap();
    let store = open(&path).await;
    assert!(
        store
            .writer()
            .command_result(CommandId(uuid::Uuid::from_u128(2)), Digest([2; 32]))
            .await
            .unwrap()
            .is_none()
    );
    let commit = append(&store, 2, &batch, &records, &ClosureControl::default())
        .await
        .unwrap();
    let retry = append(&store, 2, &batch, &records, &ClosureControl::default())
        .await
        .unwrap();
    same_commit(&commit, &retry);
    let destination = open(&dir.path().join("rebuilt.db")).await;
    let prior = fixtures::initial_state(metadata.preconditions.native_head.clone());
    let rebuilt = store
        .history()
        .rebuild_commit(&destination, &prior, &metadata_commit)
        .await
        .unwrap();
    let mut replayed = prior;
    replayed.accounting = Some(rebuilt);
    replayed.through = metadata_commit.seq;
    let final_index = store
        .history()
        .rebuild_commit(&destination, &replayed, &commit)
        .await
        .unwrap();
    assert_eq!(final_index.prefix.native_head, commit.native_head_after);
    destination.close().await.unwrap();
    store.close().await.unwrap();
}

#[tokio::test]
async fn cur_015_revision_family_preserves_prefix_and_rejects_forks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-revision-family.db");
    let store = open(&path).await;
    let state = seed(&store, &path).await;
    let id = |n| RecordId(uuid::Uuid::from_u128(n));
    let policy = DatasetPolicyV1 {
        id: id(7000),
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
        opening_policy: id(7000),
        withholding_policy: id(7000),
    };
    let leg = FinancialLegV1 {
        id: LegId(id(7002).0),
        event_id: id(7001),
        account_id: AccountId(fixtures::id(2).0),
        movement: Amount::new(1, 0, UnitId(fixtures::id(1).0)).unwrap(),
        allocations: vec![],
        effective: TemporalValue::Unknown {
            original_text: String::new(),
        },
        posting: TemporalValue::Unknown {
            original_text: String::new(),
        },
        relationships: vec![],
        unresolved_residual: SourceValue::Absent,
        interpretation: SourceValue::Absent,
    };
    let event = FinancialEventV1 {
        id: id(7001),
        legs: vec![leg.id],
        relationships: vec![],
        interpretation: SourceValue::Absent,
    };
    let root = AssociationRevisionV1 {
        id: id(7010),
        schema_version: 1,
        root: id(7010),
        prior_revision: None,
        kind: RelationshipKind::Transfer,
        legs: vec![leg.id],
        allocations: vec![],
        captured_allocations: vec![],
        recognitions: vec![],
        residuals: vec![],
        superseded_adoptions: vec![],
        survivor: None,
        retired_recognitions: vec![],
        interpretation: InterpretationV1 {
            kind: DecisionKind::Interpretation,
            authority: AuthorityKind::Owner,
            evidence: vec![],
            reason: "Synthetic structural family only".into(),
        },
        governing_policy: PolicyReferenceV1 {
            id: policy.id,
            version: 1,
        },
    };
    let append_record = |n, prior, records| HistoryAppend {
        prior_head: CommitSeq(prior),
        records,
        decisions: vec![],
        staged: vec![],
        native: None,
        result: CommandResultV1 {
            command_id: CommandId(id(n).0),
            digest: Digest([7; 32]),
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
    };
    let first = store
        .writer()
        .append(append_record(
            7100,
            state.through.0,
            vec![
                AuthoritativeRecordV1::DatasetPolicy(policy),
                AuthoritativeRecordV1::FinancialEvent(event),
                AuthoritativeRecordV1::FinancialLeg(leg),
                AuthoritativeRecordV1::AssociationRevision(root.clone()),
            ],
        ))
        .await
        .unwrap();
    let mut revision = root.clone();
    revision.id = id(7011);
    revision.prior_revision = Some(root.id);
    let second = store
        .writer()
        .append(append_record(
            7101,
            first.seq.0,
            vec![AuthoritativeRecordV1::AssociationRevision(revision.clone())],
        ))
        .await
        .unwrap();
    let mut fork = revision;
    fork.id = id(7012);
    assert!(
        store
            .writer()
            .append(append_record(
                7102,
                second.seq.0,
                vec![AuthoritativeRecordV1::AssociationRevision(fork)]
            ))
            .await
            .is_err()
    );
    assert_eq!(
        store.history().read_prefix(first.seq).await.unwrap().last(),
        Some(&first)
    );
    store.close().await.unwrap();
    let store = open(&path).await;
    assert_eq!(
        store
            .history()
            .read_prefix(second.seq)
            .await
            .unwrap()
            .last(),
        Some(&second)
    );
    let mut projection = DomainState {
        through: CommitSeq(first.seq.0 - 1),
        domain_digest: first.previous_digest,
        ..DomainState::default()
    };
    let before = projection.clone();
    assert!(
        currents_domain::ReplayEngine::apply(&mut projection, &first).is_err(),
        "structural persistence is not complete financial replay admission"
    );
    assert_eq!(projection.records, before.records);
    assert_eq!(projection.through, before.through);
    assert_eq!(projection.domain_digest, before.domain_digest);
    store.close().await.unwrap();
}

#[tokio::test]
async fn cur_015_replay_checks_retained_capture_bytes_before_activation() {
    use sha2::{Digest as _, Sha256};
    for forged in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir.path().join("source-capture.db")).await;
        let raw = include_bytes!("../../../fixtures/journeys/s3/relationship-source.json").to_vec();
        let mut rows: Vec<RelationshipSourceRowV1> = serde_json::from_slice(&raw).unwrap();
        if forged {
            rows[0].facets[0].amount = "9999".into();
        }
        let content = ContentReferenceV1 {
            digest: Digest(Sha256::digest(&raw).into()),
            byte_count: CommitSeq(raw.len() as u64),
            media_type: RELATIONSHIP_SOURCE_MEDIA.into(),
        };
        let source = SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: RecordId(uuid::Uuid::from_u128(8000)),
            device_id: None,
        };
        let receipt = ImportReceiptV1 {
            id: RecordId(uuid::Uuid::from_u128(8001)),
            source: source.clone(),
            adapter_version: "synthetic-relationships/1".into(),
            parser_version: "1".into(),
            requested_coverage: vec![],
            reported_coverage: vec![],
            received_time: serde_json::from_str("\"2026-11-02T00:00:00Z\"").unwrap(),
            result: ImportResult::Complete,
            cursor: SourceValue::Absent,
            truncated: false,
            complete: true,
            contents: vec![content.clone()],
        };
        let capture = RelationshipCaptureV1 {
            id: RecordId(uuid::Uuid::from_u128(8002)),
            schema_version: 1,
            receipt_id: receipt.id,
            source,
            original_capture_time: serde_json::from_str("\"2026-11-01T00:00:00Z\"").unwrap(),
            raw: content,
            rows,
        };
        let staged = store.stage(&raw).await.unwrap();
        let empty =
            fixtures::initial_state(store.writer().audited_prefix().await.unwrap().native_head);
        let batch = AccountingAdapter::freeze(
            &AccountingIntent::NoEffect(NoEffectV1 {
                reason: "synthetic capture".into(),
            }),
            &empty,
        )
        .unwrap();
        let mut value = input(
            8003,
            &batch,
            &[
                AuthoritativeRecordV1::ImportReceipt(receipt),
                AuthoritativeRecordV1::RelationshipCapture(capture),
            ],
        );
        value.staged = vec![staged];
        store
            .writer()
            .append_closure(
                value,
                AccountingAdapter::prepare_closure(&batch).unwrap(),
                &ClosureControl::default(),
            )
            .await
            .unwrap();
        let destination = open(&dir.path().join("destination.db")).await;
        let result =
            currents_service::ReplayCoordinator::rebuild(store.history(), destination).await;
        assert_eq!(
            result.is_ok(),
            !forged,
            "typed capture row must match authenticated retained bytes"
        );
        store.close().await.unwrap();
    }
}

#[tokio::test]
async fn cur_015_replay_checks_retained_parser_profile_bytes_before_activation() {
    use sha2::{Digest as _, Sha256};
    for forged in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir.path().join("source-capture.db")).await;
        let raw = include_bytes!("../../../fixtures/journeys/s3/relationship-source.json").to_vec();
        let rows: Vec<RelationshipSourceRowV1> = serde_json::from_slice(&raw).unwrap();
        let content = ContentReferenceV1 {
            digest: Digest(Sha256::digest(&raw).into()),
            byte_count: CommitSeq(raw.len() as u64),
            media_type: RELATIONSHIP_SOURCE_MEDIA.into(),
        };
        let source = SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: RecordId(uuid::Uuid::from_u128(8000)),
            device_id: None,
        };
        let receipt = ImportReceiptV1 {
            id: RecordId(uuid::Uuid::from_u128(8001)),
            source: source.clone(),
            adapter_version: "synthetic-relationships/1".into(),
            parser_version: "1".into(),
            requested_coverage: vec![],
            reported_coverage: vec![],
            received_time: serde_json::from_str("\"2026-11-02T00:00:00Z\"").unwrap(),
            result: ImportResult::Complete,
            cursor: SourceValue::Absent,
            truncated: false,
            complete: true,
            contents: vec![content.clone()],
        };
        let capture = RelationshipCaptureV1 {
            id: RecordId(uuid::Uuid::from_u128(8002)),
            schema_version: 1,
            receipt_id: receipt.id,
            source,
            original_capture_time: serde_json::from_str("\"2026-11-01T00:00:00Z\"").unwrap(),
            raw: content,
            rows,
        };
        let staged = store.stage(&raw).await.unwrap();
        let mut empty =
            fixtures::initial_state(store.writer().audited_prefix().await.unwrap().native_head);
        let batch = AccountingAdapter::freeze(
            &AccountingIntent::NoEffect(NoEffectV1 {
                reason: "synthetic capture".into(),
            }),
            &empty,
        )
        .unwrap();
        let mut value = input(
            8003,
            &batch,
            &[
                AuthoritativeRecordV1::ImportReceipt(receipt.clone()),
                AuthoritativeRecordV1::RelationshipCapture(capture.clone()),
            ],
        );
        value.staged = vec![staged];
        let first = store
            .writer()
            .append_closure(
                value,
                AccountingAdapter::prepare_closure(&batch).unwrap(),
                &ClosureControl::default(),
            )
            .await
            .unwrap();
        fixtures::advance(&mut empty, &first, &batch);
        let profile = RelationshipCorrectionProfileV1 {
            schema_version: 1,
            source_record: "paystub-1".into(),
            source_revision: "original".into(),
            corrections: vec![RelationshipAmountCorrectionV1 {
                facet: "gross".into(),
                expected_amount: "5000".into(),
                amount: "5200".into(),
            }],
        };
        let bytes = serde_json::to_vec(&profile).unwrap();
        let content = ContentReferenceV1 {
            digest: Digest(Sha256::digest(&bytes).into()),
            byte_count: CommitSeq(bytes.len() as u64),
            media_type: RELATIONSHIP_CORRECTION_MEDIA.into(),
        };
        let mut typed = profile;
        if forged {
            typed.corrections[0].amount = "9999".into();
        }
        let mut receipt = receipt;
        receipt.id = RecordId(uuid::Uuid::from_u128(8010));
        receipt.parser_version = RELATIONSHIP_CORRECTION_PARSER.into();
        receipt.contents = vec![content.clone()];
        let revision = RelationshipParseRevisionV1 {
            id: RecordId(uuid::Uuid::from_u128(8011)),
            schema_version: 1,
            receipt_id: receipt.id,
            capture: capture.id,
            prior_parse: None,
            profile: content,
            row: typed.apply(&capture.rows[0]).unwrap(),
            correction: typed,
        };
        let batch = AccountingAdapter::freeze(
            &AccountingIntent::NoEffect(NoEffectV1 {
                reason: "synthetic parse correction".into(),
            }),
            &empty,
        )
        .unwrap();
        let mut value = input(
            8012,
            &batch,
            &[
                AuthoritativeRecordV1::ImportReceipt(receipt),
                AuthoritativeRecordV1::RelationshipParseRevision(revision),
            ],
        );
        value.staged = vec![store.stage(&bytes).await.unwrap()];
        store
            .writer()
            .append_closure(
                value,
                AccountingAdapter::prepare_closure(&batch).unwrap(),
                &ClosureControl::default(),
            )
            .await
            .unwrap();
        let destination = open(&dir.path().join("destination.db")).await;
        let result =
            currents_service::ReplayCoordinator::rebuild(store.history(), destination).await;
        assert_eq!(
            result.is_ok(),
            !forged,
            "typed parser profile must match authenticated retained bytes even when derived row is internally consistent"
        );
        store.close().await.unwrap();
    }
}
