//! Real storage-boundary and descriptor-error parity, without relaxed limits.
use super::*;
use crate::EncryptedStore;
use currents_cipher::{CipherPool, CipherProfileV1, DatasetKey};
use std::path::Path;

fn id(n: u128) -> RecordId {
    RecordId(uuid::Uuid::from_u128(n))
}
fn fixture(kind: &str) -> AuthoritativeRecordV1 {
    let records: Vec<AuthoritativeRecordV1> = serde_json::from_str(include_str!(
        "../../../../fixtures/contracts/v1/records.json"
    ))
    .unwrap();
    records
        .into_iter()
        .find(|record| serde_json::to_value(record).unwrap()["kind"] == kind)
        .unwrap()
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
fn review(n: u128) -> AuthoritativeRecordV1 {
    AuthoritativeRecordV1::Review(ReviewRecordV1 {
        id: id(n),
        targets: vec![],
        disposition: ReviewDisposition::Accepted,
        reason: String::new(),
    })
}
fn decision(n: u128, position: u32, prior: u64) -> DecisionEnvelopeV1 {
    let result = input(prior).result;
    DecisionEnvelopeV1 {
        id: id(n),
        commit: CommitSeq(prior + 1),
        item_position: position,
        decision_type: DecisionKind::Review,
        schema_version: 1,
        actor: id(800),
        authority: AuthorityKind::Owner,
        grant_id: None,
        command_id: result.command_id,
        command_digest: result.digest,
        references: vec![],
        recorded_time: serde_json::from_str("\"2026-01-01T00:00:00Z\"").unwrap(),
        adopted_result: vec![],
        policy: PolicyReferenceV1 {
            id: id(1),
            version: 1,
        },
        corrections: vec![],
        supersedes: vec![],
    }
}
async fn baseline(store: &EncryptedStore) -> DomainCommitV1 {
    let mut initial = input(0);
    initial.records.push(fixture("DatasetPolicy"));
    // Distribute large Decision references among these real prior definitions.
    initial.decisions = (0..256)
        .map(|n| decision(1000 + n, n as u32 + 1, 0))
        .collect();
    initial.staged.push(store.stage(b"shared").await.unwrap());
    store.writer().append(initial).await.unwrap()
}
fn copy_input(source: &HistoryAppend) -> HistoryAppend {
    let mut copy = input(source.prior_head.0);
    copy.records = source.records.clone();
    copy.decisions = source.decisions.clone();
    copy
}
async fn rejected_native_is_rolled_back(
    store: &EncryptedStore,
    first: &DomainCommitV1,
    mut invalid: HistoryAppend,
    class: ErrorClass,
) {
    invalid.native = Some(currents_accounting::test_fixtures::currency_declaration());
    let error = store.writer().append(invalid).await.unwrap_err();
    assert!(matches!(error, CommitFailure::DefinitelyRolledBack(error) if error.class == class));
    assert_eq!(
        store.history().read_prefix(first.seq).await.unwrap(),
        vec![first.clone()]
    );
    assert!(store.history().stream_prefix(CommitSeq(2)).await.is_err());
    let probe = store
        .writer()
        .accounting
        .begin_authority(CommandId(id(700).0), None)
        .await
        .unwrap();
    assert_eq!(probe.native_head(), first.native_head_after);
    probe.rollback().await.unwrap();
}
async fn accepted_boundary(
    store: EncryptedStore,
    path: &Path,
    first: &DomainCommitV1,
    mut valid: HistoryAppend,
) -> (EncryptedStore, DomainCommitV1) {
    valid.native = Some(currents_accounting::test_fixtures::currency_declaration());
    let second = store.writer().append(valid).await.unwrap();
    assert_ne!(second.native_head_after, first.native_head_after);
    assert_eq!(
        store.history().read_prefix(second.seq).await.unwrap(),
        vec![first.clone(), second.clone()]
    );
    store.close().await.unwrap();
    // Reopen performs the full boundary audit before yielding the store.
    (open(path).await, second)
}

// The final commit and exactly one item are rehashed together. New-item metadata
// is inserted faithfully. No schema changes or disabled guards survive this helper.
async fn rewrite_item(
    pool: &CipherPool,
    commit: &mut DomainCommitV1,
    position: usize,
    new_item: bool,
) {
    let original_schema = schema::attest(pool.pool()).await.unwrap();
    let item = if position < commit.records.len() {
        commit.records[position].clone()
    } else {
        AuthoritativeRecordV1::Decision(commit.decisions[position - commit.records.len()].clone())
    };
    let bytes = canonical(&item).unwrap();
    let kind = serde_json::to_value(&item).unwrap()["kind"]
        .as_str()
        .unwrap()
        .to_string();
    commit.digest = Digest::default();
    commit.digest = hash(&canonical(commit).unwrap());
    let guards: Vec<String> = sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE name IN ('currents_commits_no_update','currents_items_no_update') ORDER BY name")
        .fetch_all(pool.pool()).await.unwrap();
    assert_eq!(guards.len(), 2);
    // This one-connection tamper pool alone bypasses the table's size CHECK.
    // Restore its setting before read/reopen, in addition to exact schema guards.
    sqlx::query("PRAGMA ignore_check_constraints=ON")
        .execute(pool.pool())
        .await
        .unwrap();
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
    let query = if new_item {
        "INSERT INTO currents_items(canonical,digest,kind,seq,position) VALUES(?,?,?,2,?)"
    } else {
        "UPDATE currents_items SET canonical=?,digest=?,kind=? WHERE seq=2 AND position=?"
    };
    assert_eq!(
        sqlx::query(query)
            .bind(&bytes)
            .bind(hash(&bytes).0.as_slice())
            .bind(kind)
            .bind(position as i64)
            .execute(&mut *tx)
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    if new_item {
        let meta = crate::references::describe(&item).unwrap();
        for definition in meta.definitions {
            sqlx::query(
                "INSERT INTO currents_definitions(kind,id,version,seq,position) VALUES(?,?,?,2,?)",
            )
            .bind(definition.kind)
            .bind(definition.id)
            .bind(definition.version)
            .bind(position as i64)
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        for reference in meta.references {
            let mut targets = vec![];
            for kind in reference.kinds {
                let version: Option<i64> = sqlx::query_scalar("SELECT version FROM currents_definitions WHERE kind=? AND id=? AND seq<=2 AND (? IS NULL OR version=?) ORDER BY version LIMIT 1")
                    .bind(&kind).bind(&reference.id).bind(reference.version).bind(reference.version)
                    .fetch_optional(&mut *tx).await.unwrap();
                if let Some(version) = version {
                    targets.push((kind, version));
                }
            }
            assert_eq!(targets.len(), 1);
            let (kind, version) = targets.pop().unwrap();
            sqlx::query("INSERT INTO currents_references(seq,position,target_kind,target_id,target_version) VALUES(2,?,?,?,?)")
                .bind(position as i64).bind(kind).bind(reference.id).bind(version)
                .execute(&mut *tx).await.unwrap();
        }
        for content in meta.contents {
            sqlx::query("INSERT INTO currents_content_refs(seq,position,digest) VALUES(2,?,?)")
                .bind(position as i64)
                .bind(content.digest.0.as_slice())
                .execute(&mut *tx)
                .await
                .unwrap();
        }
    }
    for guard in guards {
        sqlx::raw_sql(sqlx::AssertSqlSafe(guard.as_str()))
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    sqlx::query("PRAGMA ignore_check_constraints=OFF")
        .execute(pool.pool())
        .await
        .unwrap();
    let disabled: i64 = sqlx::query_scalar("PRAGMA ignore_check_constraints")
        .fetch_one(pool.pool())
        .await
        .unwrap();
    assert_eq!(disabled, 0);
    assert_eq!(
        schema::attest(pool.pool()).await.unwrap().schema_sha256,
        original_schema.schema_sha256
    );
    let (stored, digest): (Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT canonical,digest FROM currents_commits WHERE seq=2")
            .fetch_one(pool.pool())
            .await
            .unwrap();
    assert_eq!(stored, canonical(commit).unwrap());
    assert_eq!(digest, commit.digest.0);
    let (stored, digest): (Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT canonical,digest FROM currents_items WHERE seq=2 AND position=?")
            .bind(position as i64)
            .fetch_one(pool.pool())
            .await
            .unwrap();
    assert_eq!(stored, bytes);
    assert_eq!(digest, hash(&bytes).0);
}
async fn tamper_pool(path: &Path) -> CipherPool {
    currents_cipher::open_pool(path, DatasetKey::new([0x47; 32]), CipherProfileV1, 1)
        .await
        .unwrap()
}
async fn assert_read_and_reopen_error(store: EncryptedStore, path: &Path, class: ErrorClass) {
    let action = if class == ErrorClass::UnsupportedVersion {
        SafeNextAction::Upgrade
    } else {
        SafeNextAction::Recover
    };
    let mut failures = vec![];
    match store.history().read_prefix(CommitSeq(2)).await {
        Ok(_) => failures.push("accepted rehashed prefix".to_string()),
        Err(error) if error.class == class && error.next_action == action => {}
        Err(error) => failures.push(format!("prefix returned {error:?}")),
    }
    store.close().await.unwrap();
    match EncryptedStore::open(path, DatasetKey::new([0x47; 32]), CipherProfileV1).await {
        Ok(reopened) => {
            failures.push("accepted rehashed reopen".to_string());
            reopened.close().await.unwrap();
        }
        Err(error) if error.class == class && error.next_action == action => {}
        Err(error) => failures.push(format!("reopen returned {error:?}")),
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[derive(Clone, Copy)]
enum CountBound {
    Records,
    Decisions,
    Observations,
}
async fn count_boundary(bound: CountBound) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("count.db");
    let store = open(&path).await;
    let first = baseline(&store).await;
    let mut valid = input(1);
    let limit = match bound {
        CountBound::Observations => 10_000,
        _ => 20_000,
    };
    let extra = match bound {
        CountBound::Records => {
            valid.records = (0..limit).map(|n| review(10_000 + n as u128)).collect();
            review(90_000)
        }
        CountBound::Decisions => {
            valid.records.push(review(90_000));
            valid.decisions = (1..limit)
                .map(|n| decision(10_000 + n as u128, n as u32, 1))
                .collect();
            AuthoritativeRecordV1::Decision(decision(90_001, limit as u32, 1))
        }
        CountBound::Observations => {
            let AuthoritativeRecordV1::ObservationVersion(mut observation) =
                fixture("ObservationVersion")
            else {
                unreachable!()
            };
            observation.content_digest = hash(b"shared");
            valid.records = (0..limit)
                .map(|n| {
                    observation.id = id(10_000 + n as u128);
                    AuthoritativeRecordV1::ObservationVersion(observation.clone())
                })
                .collect();
            observation.id = id(90_000);
            AuthoritativeRecordV1::ObservationVersion(observation)
        }
    };
    assert_eq!(valid.records.len() + valid.decisions.len(), limit);
    let mut invalid = copy_input(&valid);
    if let (CountBound::Decisions, AuthoritativeRecordV1::Decision(d)) = (bound, &extra) {
        invalid.decisions.push(d.clone());
    } else {
        invalid.records.push(extra.clone());
    }
    rejected_native_is_rolled_back(&store, &first, invalid, ErrorClass::InvalidReference).await;
    let (store, mut commit) = accepted_boundary(store, &path, &first, valid).await;
    if let (CountBound::Decisions, AuthoritativeRecordV1::Decision(d)) = (bound, &extra) {
        commit.decisions.push(d.clone());
    } else {
        commit.records.push(extra);
    }
    let pool = tamper_pool(&path).await;
    rewrite_item(&pool, &mut commit, limit, true).await;
    pool.close().await;
    assert_eq!(
        store.history().read_prefix(first.seq).await.unwrap(),
        vec![first]
    );
    assert_read_and_reopen_error(store, &path, ErrorClass::IntegrityFailure).await;
}
#[tokio::test]
async fn records_count_boundary_and_rehashed_overflow() {
    count_boundary(CountBound::Records).await;
}
#[tokio::test]
async fn combined_decisions_count_boundary_and_rehashed_overflow() {
    count_boundary(CountBound::Decisions).await;
}
#[tokio::test]
async fn observation_count_boundary_and_rehashed_overflow() {
    count_boundary(CountBound::Observations).await;
}

// UUID references have a 39-byte stride. Select supported scalar encodings to
// hit exactly 1 MiB, then a schema-version digit adds exactly one corrupt byte.
fn large_decision() -> DecisionEnvelopeV1 {
    let mut d = decision(90_000, 0, 1);
    d.corrections.push(id(1000));
    for authority in [
        AuthorityKind::Owner,
        AuthorityKind::Source,
        AuthorityKind::StandingRule,
    ] {
        d.authority = authority;
        for kind in [
            DecisionKind::Review,
            DecisionKind::Interpretation,
            DecisionKind::Correction,
        ] {
            d.decision_type = kind;
            for grant in [None, Some(GrantId(id(800).0))] {
                d.grant_id = grant;
                for digits in 0..9 {
                    d.schema_version = 10_u32.pow(digits);
                    for fraction in 0..10 {
                        let time = if fraction == 0 {
                            "2026-01-01T00:00:00Z".to_string()
                        } else {
                            format!("2026-01-01T00:00:00.{}Z", "1".repeat(fraction))
                        };
                        d.recorded_time =
                            serde_json::from_value(serde_json::Value::String(time)).unwrap();
                        let base = canonical(&AuthoritativeRecordV1::Decision(d.clone()))
                            .unwrap()
                            .len();
                        if (staging::CHUNK - base).is_multiple_of(39) {
                            let count = 1 + (staging::CHUNK - base) / 39;
                            d.corrections =
                                (0..count).map(|n| id(1000 + (n % 256) as u128)).collect();
                            assert_eq!(
                                canonical(&AuthoritativeRecordV1::Decision(d.clone()))
                                    .unwrap()
                                    .len(),
                                staging::CHUNK
                            );
                            return d;
                        }
                    }
                }
            }
        }
    }
    panic!("no exact Decision byte-boundary fixture")
}
#[derive(Clone, Copy)]
enum ItemForm {
    Review,
    RecordDecision,
    AppendedDecision,
}
async fn item_boundary(form: ItemForm) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("item.db");
    let store = open(&path).await;
    let first = baseline(&store).await;
    let mut valid = input(1);
    let mut item = match form {
        ItemForm::Review => {
            let mut record = review(90_000);
            let padding = staging::CHUNK - canonical(&record).unwrap().len();
            let AuthoritativeRecordV1::Review(review) = &mut record else {
                unreachable!()
            };
            review.reason = "a".repeat(padding);
            record
        }
        _ => AuthoritativeRecordV1::Decision(large_decision()),
    };
    assert_eq!(canonical(&item).unwrap().len(), staging::CHUNK);
    if let (ItemForm::AppendedDecision, AuthoritativeRecordV1::Decision(d)) = (form, &item) {
        valid.decisions.push(d.clone());
    } else {
        valid.records.push(item.clone());
    }
    match &mut item {
        AuthoritativeRecordV1::Review(review) => review.reason.push('a'),
        AuthoritativeRecordV1::Decision(d) => d.schema_version *= 10,
        _ => unreachable!(),
    }
    assert_eq!(canonical(&item).unwrap().len(), staging::CHUNK + 1);
    let mut invalid = input(1);
    if let (ItemForm::AppendedDecision, AuthoritativeRecordV1::Decision(d)) = (form, &item) {
        invalid.decisions.push(d.clone());
    } else {
        invalid.records.push(item.clone());
    }
    rejected_native_is_rolled_back(&store, &first, invalid, ErrorClass::InvalidReference).await;
    let (store, mut commit) = accepted_boundary(store, &path, &first, valid).await;
    if let (ItemForm::AppendedDecision, AuthoritativeRecordV1::Decision(d)) = (form, &item) {
        commit.decisions[0] = d.clone();
    } else {
        commit.records[0] = item;
    }
    let pool = tamper_pool(&path).await;
    rewrite_item(&pool, &mut commit, 0, false).await;
    pool.close().await;
    assert_read_and_reopen_error(store, &path, ErrorClass::IntegrityFailure).await;
}
#[tokio::test]
async fn record_item_byte_boundary_and_rehashed_overflow() {
    item_boundary(ItemForm::Review).await;
}
#[tokio::test]
async fn record_decision_byte_boundary_and_rehashed_overflow() {
    item_boundary(ItemForm::RecordDecision).await;
}
#[tokio::test]
async fn appended_decision_byte_boundary_and_rehashed_overflow() {
    item_boundary(ItemForm::AppendedDecision).await;
}

async fn receipt_descriptor_error(truncated: bool) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("descriptor.db");
    let store = open(&path).await;
    let first = baseline(&store).await;
    let mut valid = input(1);
    let AuthoritativeRecordV1::ImportReceipt(mut receipt) = fixture("ImportReceipt") else {
        unreachable!()
    };
    receipt.id = id(90_000);
    receipt.complete = true;
    valid
        .records
        .push(AuthoritativeRecordV1::ImportReceipt(receipt.clone()));
    if truncated {
        receipt.truncated = true;
    } else {
        receipt.result = ImportResult::Partial;
    }
    let invalid_record = AuthoritativeRecordV1::ImportReceipt(receipt);
    let mut invalid = input(1);
    invalid.records.push(invalid_record.clone());
    rejected_native_is_rolled_back(&store, &first, invalid, ErrorClass::InvalidReference).await;
    let (store, mut commit) = accepted_boundary(store, &path, &first, valid).await;
    commit.records[0] = invalid_record;
    let pool = tamper_pool(&path).await;
    rewrite_item(&pool, &mut commit, 0, false).await;
    pool.close().await;
    assert_read_and_reopen_error(store, &path, ErrorClass::IntegrityFailure).await;
}
#[tokio::test]
async fn complete_truncated_receipt_is_stored_integrity_failure() {
    receipt_descriptor_error(true).await;
}
#[tokio::test]
async fn complete_partial_receipt_is_stored_integrity_failure() {
    receipt_descriptor_error(false).await;
}

#[tokio::test]
async fn unsupported_descriptor_preserves_upgrade_classification() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("unsupported.db");
    let store = open(&path).await;
    let first = baseline(&store).await;
    let mut valid = input(1);
    let AuthoritativeRecordV1::Commitment(mut commitment) = fixture("Commitment") else {
        unreachable!()
    };
    commitment.id = CommitmentId(id(90_000).0);
    valid
        .records
        .push(AuthoritativeRecordV1::Commitment(commitment.clone()));
    commitment.settlement = SettlementRuleV1::HolidayCalendar(PolicyReferenceV1 {
        id: id(1),
        version: 1,
    });
    let unsupported = AuthoritativeRecordV1::Commitment(commitment);
    let mut invalid = input(1);
    invalid.records.push(unsupported.clone());
    rejected_native_is_rolled_back(&store, &first, invalid, ErrorClass::UnsupportedVersion).await;
    let (store, mut commit) = accepted_boundary(store, &path, &first, valid).await;
    commit.records[0] = unsupported;
    let pool = tamper_pool(&path).await;
    rewrite_item(&pool, &mut commit, 0, false).await;
    pool.close().await;
    assert_read_and_reopen_error(store, &path, ErrorClass::UnsupportedVersion).await;
}

#[tokio::test]
async fn stored_sequence_overflow_is_integrity_failure() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sequence.db");
    let store = open(&path).await;
    let first = baseline(&store).await;
    let mut valid = input(1);
    valid.records.push(review(90_000));
    let (store, mut commit) = accepted_boundary(store, &path, &first, valid).await;
    // DomainCommit permits a u64; SQLite sequence keys require a signed i64.
    // Keep the final physical row at 2 and recompute the malformed envelope hash.
    commit.seq = CommitSeq(i64::MAX as u64 + 1);
    commit.command_result.commit = Some(commit.seq);
    let pool = tamper_pool(&path).await;
    rewrite_item(&pool, &mut commit, 0, false).await;
    pool.close().await;
    assert_read_and_reopen_error(store, &path, ErrorClass::IntegrityFailure).await;
}

#[tokio::test]
async fn caller_sequence_overflow_remains_invalid_reference() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cutoff.db");
    let store = open(&path).await;
    let first = baseline(&store).await;
    let cutoff = CommitSeq(i64::MAX as u64 + 1);
    let error = store.history().stream_prefix(cutoff).await.err().unwrap();
    assert_eq!(error, EngineError::invalid());
    assert_eq!(
        store.history().read_prefix(cutoff).await.unwrap_err(),
        EngineError::invalid()
    );
    assert_eq!(
        store.history().read_prefix(first.seq).await.unwrap(),
        vec![first]
    );
    store.close().await.unwrap();
}
