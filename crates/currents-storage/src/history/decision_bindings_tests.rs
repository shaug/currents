//! Decision provenance must agree with its containing commit in both storage forms.
use super::*;
use crate::EncryptedStore;
use currents_cipher::{CipherPool, CipherProfileV1, DatasetKey};
use std::path::Path;

fn id(n: u128) -> RecordId {
    RecordId(uuid::Uuid::from_u128(n))
}
async fn open(path: &Path) -> EncryptedStore {
    EncryptedStore::open(path, DatasetKey::new([0x47; 32]), CipherProfileV1)
        .await
        .unwrap()
}
fn input(prior: u64) -> HistoryAppend {
    HistoryAppend {
        prior_head: CommitSeq(prior),
        records: vec![],
        decisions: vec![],
        result: CommandResultV1 {
            command_id: CommandId(id(u128::from(prior) + 100).0),
            digest: hash(&prior.to_be_bytes()),
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
        staged: vec![],
        native: None,
    }
}
fn decisions_input() -> HistoryAppend {
    let mut input = input(1);
    let decision = |n, position| DecisionEnvelopeV1 {
        id: id(n),
        commit: CommitSeq(2),
        item_position: position,
        decision_type: DecisionKind::Review,
        schema_version: 1,
        actor: id(800),
        authority: AuthorityKind::Owner,
        grant_id: None,
        command_id: input.result.command_id,
        command_digest: input.result.digest,
        references: vec![id(502)],
        recorded_time: serde_json::from_str("\"2026-01-01T00:00:00Z\"").unwrap(),
        adopted_result: vec![],
        policy: PolicyReferenceV1 {
            id: id(500),
            version: 1,
        },
        corrections: vec![],
        supersedes: vec![],
    };
    input.records = vec![
        AuthoritativeRecordV1::DatasetPolicy(DatasetPolicyV1 {
            id: id(500),
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
            opening_policy: id(500),
            withholding_policy: id(500),
        }),
        // A Decision between other records is legal; no placement rule is added.
        AuthoritativeRecordV1::Decision(decision(501, 1)),
        AuthoritativeRecordV1::Review(ReviewRecordV1 {
            id: id(502),
            targets: vec![],
            disposition: ReviewDisposition::Accepted,
            reason: "synthetic".into(),
        }),
    ];
    input.decisions = vec![decision(503, 3), decision(504, 4)];
    input
}

#[derive(Clone, Copy, Debug)]
enum Container {
    Records,
    Decisions,
}
impl Container {
    fn decision<'a>(
        self,
        records: &'a mut [AuthoritativeRecordV1],
        decisions: &'a mut [DecisionEnvelopeV1],
    ) -> &'a mut DecisionEnvelopeV1 {
        match self {
            Self::Records => match &mut records[1] {
                AuthoritativeRecordV1::Decision(decision) => decision,
                _ => panic!("decision fixture missing"),
            },
            Self::Decisions => &mut decisions[1],
        }
    }
    fn position(self) -> i64 {
        match self {
            Self::Records => 1,
            Self::Decisions => 4,
        }
    }
}
#[derive(Clone, Copy, Debug)]
enum Binding {
    Commit,
    Position,
    CommandId,
    CommandDigest,
}
impl Binding {
    fn corrupt(self, decision: &mut DecisionEnvelopeV1) {
        match self {
            Self::Commit => decision.commit = CommitSeq(99),
            Self::Position => decision.item_position += 1,
            Self::CommandId => decision.command_id = CommandId(id(999).0),
            Self::CommandDigest => decision.command_digest = hash(b"another command"),
        }
    }
}

#[tokio::test]
async fn decision_bindings_roundtrip_mixed_containers_and_positions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("roundtrip.db");
    let store = open(&path).await;
    let first = store.writer().append(input(0)).await.unwrap();
    let second = store.writer().append(decisions_input()).await.unwrap();
    let third = store.writer().append(input(2)).await.unwrap();
    assert_eq!(first.native_head_after, third.native_head_after);
    assert_eq!(
        store.history().read_prefix(second.seq).await.unwrap(),
        vec![first.clone(), second.clone()]
    );
    let expected = vec![first, second, third];
    assert_eq!(
        store.history().read_prefix(CommitSeq(3)).await.unwrap(),
        expected
    );
    store.close().await.unwrap();
    let reopened = open(&path).await;
    assert_eq!(
        reopened.history().read_prefix(CommitSeq(3)).await.unwrap(),
        expected
    );
    reopened.close().await.unwrap();
}

async fn rejects_binding_and_rolls_back_native(container: Container, binding: Binding) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rollback.db");
    let store = open(&path).await;
    let first = store.writer().append(input(0)).await.unwrap();
    let mut invalid = decisions_input();
    binding.corrupt(container.decision(&mut invalid.records, &mut invalid.decisions));
    invalid.native = Some(currents_accounting::test_fixtures::currency_declaration());
    let error = store.writer().append(invalid).await.unwrap_err();
    assert!(matches!(
        error,
        CommitFailure::DefinitelyRolledBack(EngineError {
            class: ErrorClass::InvalidReference,
            next_action: SafeNextAction::CorrectInput,
            ..
        })
    ));
    assert_eq!(
        store.history().read_prefix(first.seq).await.unwrap(),
        vec![first.clone()]
    );
    assert!(store.history().stream_prefix(CommitSeq(2)).await.is_err());
    let probe = store
        .writer()
        .accounting
        .begin_authority(CommandId(id(600).0), None)
        .await
        .unwrap();
    assert_eq!(probe.native_head(), first.native_head_after);
    probe.rollback().await.unwrap();

    // Reuse all IDs and the actual native operation: neither side may survive
    // rejection, whether it occurred before or after inserting the commit row.
    let mut retry = decisions_input();
    retry.native = Some(currents_accounting::test_fixtures::currency_declaration());
    let second = store.writer().append(retry).await.unwrap();
    assert_ne!(second.native_head_after, first.native_head_after);
    store.close().await.unwrap();
    let reopened = open(&path).await;
    assert_eq!(
        reopened.history().read_prefix(second.seq).await.unwrap(),
        vec![first, second]
    );
    reopened.close().await.unwrap();
}

async fn rewrite_decision(pool: &CipherPool, commit: &mut DomainCommitV1, container: Container) {
    let before = schema::attest(pool.pool()).await.unwrap();
    let guards: Vec<String> = sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE name IN ('currents_commits_no_update','currents_items_no_update') ORDER BY name")
        .fetch_all(pool.pool()).await.unwrap();
    assert_eq!(guards.len(), 2);
    commit.digest = Digest::default();
    commit.digest = hash(&canonical(commit).unwrap());
    let item = AuthoritativeRecordV1::Decision(
        container
            .decision(&mut commit.records, &mut commit.decisions)
            .clone(),
    );
    let bytes = canonical(&item).unwrap();
    let mut tx = pool.pool().begin().await.unwrap();
    sqlx::raw_sql(
        "DROP TRIGGER currents_commits_no_update; DROP TRIGGER currents_items_no_update;",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    assert_eq!(
        sqlx::query("UPDATE currents_commits SET canonical=?,digest=? WHERE seq=2")
            .bind(canonical(commit).unwrap())
            .bind(commit.digest.0.as_slice())
            .execute(&mut *tx)
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    assert_eq!(
        sqlx::query("UPDATE currents_items SET canonical=?,digest=? WHERE seq=2 AND position=?")
            .bind(&bytes)
            .bind(hash(&bytes).0.as_slice())
            .bind(container.position())
            .execute(&mut *tx)
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    for guard in guards {
        sqlx::raw_sql(sqlx::AssertSqlSafe(guard.as_str()))
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    // Exact production schema attestation must pass before any rejection claim.
    let after = schema::attest(pool.pool()).await.unwrap();
    assert_eq!(after.schema_sha256, before.schema_sha256);
    assert_eq!(after.migration_sha256, before.migration_sha256);
    let (stored, digest): (Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT canonical,digest FROM currents_commits WHERE seq=2")
            .fetch_one(pool.pool())
            .await
            .unwrap();
    assert_eq!(stored, canonical(commit).unwrap());
    assert_eq!(digest, commit.digest.0);
    let (stored, digest): (Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT canonical,digest FROM currents_items WHERE seq=2 AND position=?")
            .bind(container.position())
            .fetch_one(pool.pool())
            .await
            .unwrap();
    assert_eq!(stored, bytes);
    assert_eq!(digest, hash(&bytes).0);
}

async fn rejects_rehashed_binding_in_read_and_reopen(container: Container, binding: Binding) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rehashed.db");
    let store = open(&path).await;
    let first = store.writer().append(input(0)).await.unwrap();
    let mut second = store.writer().append(decisions_input()).await.unwrap();
    let expected = vec![first.clone(), second.clone()];
    assert_eq!(
        store.history().read_prefix(second.seq).await.unwrap(),
        expected
    );
    let mut pinned = store.history().stream_prefix(second.seq).await.unwrap();
    binding.corrupt(container.decision(&mut second.records, &mut second.decisions));
    let pool = currents_cipher::open_pool(&path, DatasetKey::new([0x47; 32]), CipherProfileV1, 1)
        .await
        .unwrap();
    rewrite_decision(&pool, &mut second, container).await;
    assert_eq!(pinned.next().await.unwrap(), Some(expected[0].clone()));
    assert_eq!(pinned.next().await.unwrap(), Some(expected[1].clone()));
    assert!(pinned.next().await.unwrap().is_none());
    drop(pinned);
    assert_eq!(
        store.history().read_prefix(first.seq).await.unwrap(),
        vec![first.clone()]
    );
    let mut failures = vec![];
    match store.history().read_prefix(second.seq).await {
        Ok(_) => failures.push("accepted rehashed prefix"),
        Err(error) => assert_eq!(error.class, ErrorClass::IntegrityFailure),
    }
    let mut stream = store.history().stream_prefix(second.seq).await.unwrap();
    assert_eq!(stream.next().await.unwrap(), Some(first));
    match stream.next().await {
        Ok(_) => failures.push("accepted rehashed stream item"),
        Err(error) => assert_eq!(error.class, ErrorClass::IntegrityFailure),
    }
    drop(stream);
    pool.close().await;
    store.close().await.unwrap();
    match EncryptedStore::open(&path, DatasetKey::new([0x47; 32]), CipherProfileV1).await {
        Ok(reopened) => {
            failures.push("accepted rehashed reopen");
            reopened.close().await.unwrap();
        }
        Err(error) => assert_eq!(error.class, ErrorClass::IntegrityFailure),
    }
    assert!(
        failures.is_empty(),
        "{container:?} {binding:?}: {failures:?}"
    );
}

macro_rules! binding_tests {
    ($write:ident, $read:ident, $container:ident, $binding:ident) => {
        #[tokio::test]
        async fn $write() {
            rejects_binding_and_rolls_back_native(Container::$container, Binding::$binding).await;
        }
        #[tokio::test]
        async fn $read() {
            rejects_rehashed_binding_in_read_and_reopen(Container::$container, Binding::$binding)
                .await;
        }
    };
}
binding_tests!(
    records_commit_rollback,
    records_commit_audit,
    Records,
    Commit
);
binding_tests!(
    records_position_rollback,
    records_position_audit,
    Records,
    Position
);
binding_tests!(
    records_command_id_rollback,
    records_command_id_audit,
    Records,
    CommandId
);
binding_tests!(
    records_command_digest_rollback,
    records_command_digest_audit,
    Records,
    CommandDigest
);
binding_tests!(
    decisions_commit_rollback,
    decisions_commit_audit,
    Decisions,
    Commit
);
binding_tests!(
    decisions_position_rollback,
    decisions_position_audit,
    Decisions,
    Position
);
binding_tests!(
    decisions_command_id_rollback,
    decisions_command_id_audit,
    Decisions,
    CommandId
);
binding_tests!(
    decisions_command_digest_rollback,
    decisions_command_digest_audit,
    Decisions,
    CommandDigest
);
