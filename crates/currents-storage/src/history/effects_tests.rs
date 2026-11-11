//! Command-result references are authority even when no record was changed.
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
fn empty_effects() -> EffectsV1 {
    EffectsV1 {
        records: vec![],
        accounts: vec![],
        amounts: vec![],
        invalidations: vec![],
    }
}
fn append_input(prior: u64, records: Vec<AuthoritativeRecordV1>) -> HistoryAppend {
    HistoryAppend {
        prior_head: CommitSeq(prior),
        records,
        decisions: vec![],
        result: CommandResultV1 {
            command_id: CommandId(id(u128::from(prior) + 100).0),
            digest: hash(&prior.to_be_bytes()),
            state: CommandState::Committed,
            commit: None,
            effects: empty_effects(),
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
        reason: "synthetic".into(),
    })
}
fn definitions(base: u128) -> Vec<AuthoritativeRecordV1> {
    let account = AccountId(id(base + 4).0);
    // The account's unit is defined later in this same commit.
    vec![
        AuthoritativeRecordV1::AccountComponent(AccountComponentV1 {
            id: id(base + 3),
            account_id: account,
            account_class: AccountClass::Asset,
            unit_id: UnitId(id(base + 2).0),
            normal_sign: NormalSign::Debit,
            aliases: vec![],
            ownership: OwnershipV1 {
                account_id: account,
                numerator: CommitSeq(1),
                denominator: CommitSeq(1),
            },
            coverage: vec![],
            opening_boundary: SourceValue::Unknown,
            overlapping_components: vec![],
            lifecycle: Lifecycle::Open,
            display_name: "synthetic".into(),
        }),
        AuthoritativeRecordV1::UnitCatalog(UnitCatalogV1 {
            id: id(base + 1),
            version: 1,
            units: vec![UnitDefinition {
                id: UnitId(id(base + 2).0),
                exponent: 2,
                native_code: "USD".into(),
                kind: UnitKind::Currency,
            }],
        }),
        review(base + 5),
    ]
}
#[derive(Clone, Copy, Debug)]
enum Role {
    Records,
    Accounts,
    Amounts,
    Invalidations,
}
impl Role {
    fn target(self, base: u128) -> RecordId {
        id(base
            + match self {
                Self::Records | Self::Invalidations => 5,
                Self::Accounts => 4,
                Self::Amounts => 2,
            })
    }
    fn wrong_kind(self) -> RecordId {
        id(match self {
            // Unit and Account are not generic authoritative record families.
            Self::Records => 1002,
            Self::Invalidations => 1004,
            Self::Accounts => 1003, // Component ID is not Account ID.
            Self::Amounts => 1001,  // Catalog ID is not Unit ID.
        })
    }
    fn set(self, effects: &mut EffectsV1, target: RecordId) {
        match self {
            Self::Records => effects.records = vec![target],
            Self::Accounts => effects.accounts = vec![AccountId(target.0)],
            Self::Amounts => effects.amounts = vec![Amount::new(125, 2, UnitId(target.0)).unwrap()],
            Self::Invalidations => effects.invalidations = vec![target],
        }
    }
    fn ambiguous(self) -> bool {
        matches!(self, Self::Records | Self::Invalidations)
    }
}
fn all_effects(base: u128) -> EffectsV1 {
    let mut effects = empty_effects();
    for role in [
        Role::Records,
        Role::Accounts,
        Role::Amounts,
        Role::Invalidations,
    ] {
        role.set(&mut effects, role.target(base));
    }
    effects
}

#[tokio::test]
async fn command_effects_roundtrip_same_and_prior_prefix_families() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("effects.db");
    let store = open(&path).await;
    let mut input = append_input(0, definitions(1000));
    input.result.effects = all_effects(1000);
    input.result.effects.records.extend([id(1001), id(1003)]);
    let first = store.writer().append(input).await.unwrap();
    let mut pinned = store.history().stream_prefix(first.seq).await.unwrap();

    let mut later_definitions = definitions(1000);
    let AuthoritativeRecordV1::AccountComponent(component) = &mut later_definitions[0] else {
        unreachable!()
    };
    component.id = id(1006); // Another component defines the same Account family.
    let AuthoritativeRecordV1::UnitCatalog(catalog) = &mut later_definitions[1] else {
        unreachable!()
    };
    catalog.version = 2; // Multiple versions do not make one kind ambiguous.
    later_definitions.pop();
    let mut input = append_input(1, later_definitions);
    input.result.effects = first.command_result.effects.clone();
    let second = store.writer().append(input).await.unwrap();
    // Results may refer to prior families with no changed records or native effect.
    let mut input = append_input(2, vec![]);
    input.result.effects = first.command_result.effects.clone();
    let third = store.writer().append(input).await.unwrap();
    assert_eq!(pinned.next().await.unwrap(), Some(first.clone()));
    assert!(pinned.next().await.unwrap().is_none());
    drop(pinned);
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

// Removing any effect-role validation lets the corresponding counterexamples
// commit a real native operation. Each failure must leave both histories intact.
async fn rejects_invalid_effects_and_native_operation(role: Role) {
    let mut cases = vec![("missing", id(9999)), ("wrong kind", role.wrong_kind())];
    if role.ambiguous() {
        cases.push(("ambiguous", id(1001)));
    }
    let mut failures = vec![];
    for (case, target) in cases {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rollback.db");
        let store = open(&path).await;
        let baseline = store
            .writer()
            .append(append_input(0, vec![]))
            .await
            .unwrap();
        let mut records = definitions(1000);
        if case == "ambiguous" {
            records.push(review(1001)); // Same UUID is both Review and UnitCatalog.
        }
        let mut input = append_input(1, records);
        role.set(&mut input.result.effects, target);
        input.native = Some(currents_accounting::test_fixtures::currency_declaration());
        let rejected = store.writer().append(input).await;
        if !matches!(&rejected, Err(CommitFailure::DefinitelyRolledBack(e)) if e.class == ErrorClass::InvalidReference)
        {
            failures.push(format!("{role:?} {case}: {rejected:?}"));
            store.close().await.unwrap();
            continue;
        }
        assert_eq!(
            store.history().read_prefix(CommitSeq(1)).await.unwrap(),
            vec![baseline.clone()]
        );
        assert!(store.history().stream_prefix(CommitSeq(2)).await.is_err());
        let probe = store
            .writer()
            .accounting
            .begin_authority(CommandId(id(500).0), None)
            .await
            .unwrap();
        assert_eq!(
            probe.native_head(),
            baseline.native_head_after,
            "{role:?} {case}"
        );
        probe.rollback().await.unwrap();

        // Same IDs and command can now commit: no definitions, authority row,
        // or native currency declaration survived the rejected transaction.
        let mut retry = append_input(1, definitions(1000));
        retry.result.effects = all_effects(1000);
        retry.native = Some(currents_accounting::test_fixtures::currency_declaration());
        let committed = store.writer().append(retry).await.unwrap();
        assert_ne!(committed.native_head_after, baseline.native_head_after);
        store.close().await.unwrap();
        let reopened = open(&path).await;
        assert_eq!(
            reopened.history().read_prefix(CommitSeq(2)).await.unwrap(),
            vec![baseline, committed]
        );
        reopened.close().await.unwrap();
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// Change only the result and chain digests, then restore the exact guard before
// reopen. Rejection must come from authority auditing, not schema attestation.
async fn rewrite_commits(pool: &CipherPool, commits: &mut [DomainCommitV1]) {
    let trigger: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE name='currents_commits_no_update'")
            .fetch_one(pool.pool())
            .await
            .unwrap();
    sqlx::query("DROP TRIGGER currents_commits_no_update")
        .execute(pool.pool())
        .await
        .unwrap();
    let mut previous = Digest::default();
    for commit in commits {
        commit.previous_digest = previous;
        commit.digest = Digest::default();
        commit.digest = hash(&canonical(commit).unwrap());
        sqlx::query(
            "UPDATE currents_commits SET canonical=?,digest=?,previous_digest=? WHERE seq=?",
        )
        .bind(canonical(commit).unwrap())
        .bind(commit.digest.0.as_slice())
        .bind(previous.0.as_slice())
        .bind(seq(commit.seq).unwrap())
        .execute(pool.pool())
        .await
        .unwrap();
        previous = commit.digest;
    }
    sqlx::raw_sql(sqlx::AssertSqlSafe(trigger.as_str()))
        .execute(pool.pool())
        .await
        .unwrap();
    schema::attest(pool.pool()).await.unwrap();
}
async fn rejects_rehashed_effects_in_read_and_reopen(role: Role) {
    let mut cases = vec![
        ("missing", id(9999)),
        ("wrong kind", role.wrong_kind()),
        ("future", role.target(2000)),
    ];
    if role.ambiguous() {
        cases.push(("ambiguous", id(1001)));
    }
    let mut failures = vec![];
    for (case, target) in cases {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rehashed.db");
        let store = open(&path).await;
        let mut records = definitions(1000);
        if case == "ambiguous" {
            records.push(review(1001));
        }
        let mut input = append_input(0, records);
        input.result.effects = all_effects(1000);
        let first = store.writer().append(input).await.unwrap();
        let second = store
            .writer()
            .append(append_input(1, definitions(2000)))
            .await
            .unwrap();
        let mut commits = vec![first, second];
        assert_eq!(
            store.history().read_prefix(CommitSeq(2)).await.unwrap(),
            commits
        );
        let mut pinned = store.history().stream_prefix(CommitSeq(2)).await.unwrap();
        let original = commits.clone();
        role.set(&mut commits[0].command_result.effects, target);
        let pool =
            currents_cipher::open_pool(&path, DatasetKey::new([0x47; 32]), CipherProfileV1, 1)
                .await
                .unwrap();
        rewrite_commits(&pool, &mut commits).await;
        // A previously pinned snapshot still sees the original, valid results.
        assert_eq!(pinned.next().await.unwrap(), Some(original[0].clone()));
        assert_eq!(pinned.next().await.unwrap(), Some(original[1].clone()));
        assert!(pinned.next().await.unwrap().is_none());
        drop(pinned);
        for through in [CommitSeq(1), CommitSeq(2)] {
            if store.history().read_prefix(through).await.is_ok() {
                failures.push(format!("{role:?} {case}: accepted prefix {through:?}"));
            }
        }
        pool.close().await;
        store.close().await.unwrap();
        match EncryptedStore::open(&path, DatasetKey::new([0x47; 32]), CipherProfileV1).await {
            Ok(reopened) => {
                failures.push(format!("{role:?} {case}: accepted reopen"));
                reopened.close().await.unwrap();
            }
            Err(error) => assert_eq!(error.class, ErrorClass::IntegrityFailure),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

macro_rules! role_tests {
    ($write:ident, $read:ident, $role:ident) => {
        #[tokio::test]
        async fn $write() {
            rejects_invalid_effects_and_native_operation(Role::$role).await;
        }
        #[tokio::test]
        async fn $read() {
            rejects_rehashed_effects_in_read_and_reopen(Role::$role).await;
        }
    };
}
role_tests!(
    command_effect_records_rollback,
    command_effect_records_audit,
    Records
);
role_tests!(
    command_effect_accounts_rollback,
    command_effect_accounts_audit,
    Accounts
);
role_tests!(
    command_effect_amounts_rollback,
    command_effect_amounts_audit,
    Amounts
);
role_tests!(
    command_effect_invalidations_rollback,
    command_effect_invalidations_audit,
    Invalidations
);

#[tokio::test]
async fn command_effects_cannot_resolve_another_dataset() {
    let directory = tempfile::tempdir().unwrap();
    let other = open(&directory.path().join("other.db")).await;
    other
        .writer()
        .append(append_input(0, definitions(1000)))
        .await
        .unwrap();
    let mut failures = vec![];
    for role in [
        Role::Records,
        Role::Accounts,
        Role::Amounts,
        Role::Invalidations,
    ] {
        let local = open(&directory.path().join(format!("{role:?}.db"))).await;
        let mut input = append_input(0, vec![]);
        role.set(&mut input.result.effects, role.target(1000));
        if !matches!(
            local.writer().append(input).await,
            Err(CommitFailure::DefinitelyRolledBack(_))
        ) {
            failures.push(format!("{role:?}"));
        }
        local.close().await.unwrap();
    }
    other.close().await.unwrap();
    assert!(
        failures.is_empty(),
        "accepted foreign-dataset targets: {failures:?}"
    );
}

#[tokio::test]
async fn command_effect_amount_validation_is_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(&directory.path().join("amount.db")).await;
    for amount in [
        Amount {
            coefficient: 1,
            scale: 19,
            unit_id: UnitId(id(1002).0),
        },
        Amount {
            coefficient: i128::MIN,
            scale: 0,
            unit_id: UnitId(id(1002).0),
        },
    ] {
        let mut input = append_input(0, definitions(1000));
        input.result.effects.amounts.push(amount);
        assert!(matches!(
            store.writer().append(input).await,
            Err(CommitFailure::DefinitelyRolledBack(_))
        ));
    }
    assert!(store.history().stream_prefix(CommitSeq(1)).await.is_err());
    store.close().await.unwrap();
}
