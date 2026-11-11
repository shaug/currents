//! C05 public synthetic acceptance. No institution or production host access.
use currents_accounting::{AccountingAdapter, numbers};
use currents_types::*;
fn id(n: u128) -> RecordId {
    RecordId(format!("00000000-0000-0000-0000-{n:012x}").parse().unwrap())
}
fn date(s: &str) -> TemporalValue {
    serde_json::from_value(serde_json::json!({"kind":"Date","civil":s,"original_text":s})).unwrap()
}
fn unknown() -> TemporalValue {
    TemporalValue::Unknown {
        original_text: String::new(),
    }
}
fn unit(n: u128, code: &str, exponent: u8) -> UnitDefinition {
    UnitDefinition {
        id: UnitId(id(n).0),
        native_code: code.into(),
        exponent,
        kind: if code == "USD" {
            UnitKind::Currency
        } else {
            UnitKind::Security
        },
    }
}
fn account(n: u128, u: UnitId) -> AccountComponentV1 {
    AccountComponentV1 {
        id: id(n + 100),
        account_id: AccountId(id(n).0),
        account_class: AccountClass::Asset,
        unit_id: u,
        normal_sign: NormalSign::Debit,
        aliases: vec![],
        ownership: OwnershipV1 {
            account_id: AccountId(id(n).0),
            numerator: CommitSeq(1),
            denominator: CommitSeq(1),
        },
        coverage: vec![],
        opening_boundary: SourceValue::Unknown,
        overlapping_components: vec![],
        lifecycle: Lifecycle::Open,
        display_name: "Synthetic".into(),
    }
}
fn policy() -> DateConversionPolicyV1 {
    DateConversionPolicyV1 {
        id: id(900),
        version: 1,
        conversion: DateConversionV1::CivilOnly,
        bookkeeping: BookkeepingFallbackV1::Explicit,
    }
}
fn state() -> DomainState {
    let mut state = DomainState {
        accounting: Some(AccountingIndexV1::empty(NativeHead {
            semantics_version: 1,
            checkpoint: Digest([0; 32]),
        })),
        ..DomainState::default()
    };
    state.records.insert(
        id(900),
        AuthoritativeRecordV1::DateConversionPolicy(policy()),
    );
    state
}
fn metadata(state: &DomainState) -> AccountingBatchV1 {
    let usd = unit(1, "USD", 2);
    AccountingAdapter::freeze(
        &AccountingIntent::MetadataDeclaration(MetadataDeclarationV1 {
            accounts: vec![account(2, usd.id), account(3, usd.id)],
            units: vec![usd],
        }),
        state,
    )
    .unwrap()
}
fn apply(state: &mut DomainState, batch: &AccountingBatchV1, native: NativeHead) {
    let manifest = AccountingAdapter::manifest(id(700 + state.through.0 as u128), batch).unwrap();
    let mut commit = DomainCommitV1 {
        seq: CommitSeq(state.through.0 + 1),
        previous_digest: state.accounting.as_ref().unwrap().prefix.domain_digest,
        records: vec![AuthoritativeRecordV1::FrozenAccountingManifest(manifest)],
        decisions: vec![],
        native_head_after: native,
        command_result: CommandResultV1 {
            command_id: CommandId(id(800 + state.through.0 as u128).0),
            digest: Digest([1; 32]),
            state: CommandState::Committed,
            commit: Some(CommitSeq(state.through.0 + 1)),
            effects: EffectsV1 {
                records: vec![],
                accounts: vec![],
                amounts: vec![],
                invalidations: vec![],
            },
            health: vec![],
        },
        digest: Digest([0; 32]),
    };
    use sha2::{Digest as _, Sha256};
    commit.digest =
        Digest(Sha256::digest(serde_json_canonicalizer::to_vec(&commit).unwrap()).into());
    AccountingAdapter::advance_index(
        state,
        &commit,
        &AccountingAdapter::instruction_bytes(batch).unwrap(),
    )
    .unwrap();
}
fn declared() -> DomainState {
    let mut s = state();
    let batch = metadata(&s);
    apply(
        &mut s,
        &batch,
        NativeHead {
            semantics_version: 1,
            checkpoint: Digest([2; 32]),
        },
    );
    s
}
fn recognition(n: u128, amount: i128, scale: u8) -> RecognitionV1 {
    RecognitionV1 {
        id: RecognitionId(id(n).0),
        lineage: id(n + 1000),
        postings: vec![
            PostingV1 {
                id: id(n + 2000),
                account_id: AccountId(id(2).0),
                amount: Amount {
                    coefficient: amount,
                    scale,
                    unit_id: UnitId(id(1).0),
                },
                effective: date("2024-06-30"),
                posting: date("2024-07-02"),
            },
            PostingV1 {
                id: id(n + 3000),
                account_id: AccountId(id(3).0),
                amount: Amount {
                    coefficient: -amount,
                    scale,
                    unit_id: UnitId(id(1).0),
                },
                effective: date("2024-06-30"),
                posting: date("2024-07-02"),
            },
        ],
        kind: RecognitionKind::Activity,
        evidence_allocations: vec![],
        leg_ids: vec![],
        scope: DateScope {
            start: None,
            end: None,
            basis: DateBasis::Effective,
        },
        bookkeeping_date: date("2024-07-02"),
        date_policy: PolicyReferenceV1 {
            id: id(900),
            version: 1,
        },
    }
}
#[test]
fn cur_009_1() {
    let s = declared();
    assert!(
        AccountingAdapter::freeze(&AccountingIntent::Recognition(recognition(10, 1, 3)), &s)
            .is_err()
    );
    assert!(
        numbers::native_amount(
            &Amount {
                coefficient: 1,
                scale: 18,
                unit_id: UnitId(id(9).0)
            },
            &unit(9, "SECURITY", 18)
        )
        .is_ok()
    );
    for (n, scale) in [(i128::MIN, 18), (i128::MAX, 0)] {
        assert!(
            numbers::native_amount(
                &Amount {
                    coefficient: n,
                    scale,
                    unit_id: UnitId(id(1).0)
                },
                &unit(1, "USD", 2)
            )
            .is_err()
        );
    }
    assert!(numbers::checked_sum(&[i128::MAX, 1, -1, -i128::MAX]).is_err());
    let mut securities = s.clone();
    let security = unit(9, "SECURITY", 18);
    let declaration = AccountingAdapter::freeze(
        &AccountingIntent::MetadataDeclaration(MetadataDeclarationV1 {
            accounts: vec![account(20, security.id), account(21, security.id)],
            units: vec![security],
        }),
        &securities,
    )
    .unwrap();
    let mut share = recognition(20, 1, 18);
    for (i, p) in share.postings.iter_mut().enumerate() {
        p.account_id = AccountId(id(20 + i as u128).0);
        p.amount.unit_id = UnitId(id(9).0);
    }
    assert!(
        AccountingAdapter::freeze(&AccountingIntent::Recognition(share.clone()), &securities)
            .is_err()
    );
    apply(
        &mut securities,
        &declaration,
        NativeHead {
            semantics_version: 1,
            checkpoint: Digest([4; 32]),
        },
    );
    assert!(
        AccountingAdapter::freeze(&AccountingIntent::Recognition(share.clone()), &securities)
            .is_ok()
    );
    let mut overflow = share.clone();
    overflow.postings = (0..4)
        .map(|i| {
            let mut p = share.postings[i % 2].clone();
            p.id = id(9000 + i as u128);
            p.amount.coefficient = if i < 2 { i128::MAX } else { -i128::MAX };
            p
        })
        .collect();
    assert!(
        AccountingAdapter::freeze(&AccountingIntent::Recognition(overflow), &securities).is_err()
    );

    // Net zero and every signed prefix fit, but native gross totals overflow.
    let mut boundary = share.clone();
    boundary.postings[0].amount.coefficient = i128::MAX;
    boundary.postings[1].amount.coefficient = -i128::MAX;
    let mut frozen = AccountingAdapter::freeze(
        &AccountingIntent::Recognition(boundary.clone()),
        &securities,
    )
    .unwrap();
    let mut manifest = AccountingAdapter::manifest(id(8800), &frozen).unwrap();
    let mut extra = boundary.postings.clone();
    for (i, posting) in extra.iter_mut().enumerate() {
        posting.id = id(8801 + i as u128);
    }
    boundary.postings.extend(extra.clone());
    assert!(
        AccountingAdapter::freeze(&AccountingIntent::Recognition(boundary), &securities,).is_err()
    );
    let FrozenNativeOperationV1::Entry(entry) = &mut frozen.operations[0] else {
        panic!("financial entry expected")
    };
    let mut frozen_extra = entry.postings.clone();
    for (posting, original) in frozen_extra.iter_mut().zip(extra) {
        posting.original = original;
    }
    entry.postings.extend(frozen_extra);
    frozen.decision_digest = AccountingAdapter::prebinding_digest(&frozen).unwrap();
    let FrozenNativeOperationV1::Entry(entry) = &mut frozen.operations[0] else {
        panic!("financial entry expected")
    };
    entry.generated_document = frozen.decision_digest;
    let bytes = AccountingAdapter::instruction_bytes(&frozen).unwrap();
    use sha2::{Digest as _, Sha256};
    manifest.instructions.digest = Digest(Sha256::digest(&bytes).into());
    manifest.instructions.byte_count = CommitSeq(bytes.len() as u64);
    manifest.decision_digest = frozen.decision_digest;
    assert!(AccountingAdapter::from_manifest(&manifest, &bytes).is_err());

    assert_eq!(numbers::half_even(5, 2).unwrap().rounded, 2);
    assert_eq!(numbers::half_even(7, 2).unwrap().rounded, 4);
    assert_eq!(numbers::half_even(-5, 2).unwrap().rounded, -2);
    assert_eq!(
        numbers::largest_remainder(1, &[(id(5), 1), (id(4), 1)]).unwrap(),
        vec![(id(4), 1), (id(5), 0)]
    );
}
#[test]
fn cur_009_2() {
    let mut s = declared();
    let r = recognition(10, 100, 2);
    let b = AccountingAdapter::freeze(&AccountingIntent::Recognition(r), &s).unwrap();
    let entry = b
        .operations
        .iter()
        .find_map(|op| {
            if let FrozenNativeOperationV1::Entry(e) = op {
                Some(e)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(entry.booking_date, "2024-07-02");
    assert_eq!(entry.value_date, "2024-06-30");
    apply(
        &mut s,
        &b,
        NativeHead {
            semantics_version: 1,
            checkpoint: Digest([3; 32]),
        },
    );
    // Conversion is explicit and frozen; source timestamps/precision remain intact.
    let mut instant_recognition = recognition(12, 100, 2);
    let instant:TemporalValue=serde_json::from_value(serde_json::json!({"kind":"Instant","utc":"2024-07-01T02:30:00Z","original_offset":"-07:00","original_zone":"America/Los_Angeles","precision":"Second","original_text":"2024-06-30T19:30:00-07:00"})).unwrap();
    for p in &mut instant_recognition.postings {
        p.effective = instant.clone();
    }
    assert!(
        AccountingAdapter::freeze(
            &AccountingIntent::Recognition(instant_recognition.clone()),
            &s
        )
        .is_err()
    );
    let mut explicit = policy();
    explicit.conversion =
        DateConversionV1::Instants(InstantConversionV1::Resolved(vec![ResolvedInstantV1 {
            utc: "2024-07-01T02:30:00Z".into(),
            zone: "America/Los_Angeles".into(),
            data_version: "synthetic-2024".into(),
            offset: "-07:00".into(),
        }]));
    s.records.insert(
        explicit.id,
        AuthoritativeRecordV1::DateConversionPolicy(explicit),
    );
    let frozen =
        AccountingAdapter::freeze(&AccountingIntent::Recognition(instant_recognition), &s).unwrap();
    let FrozenNativeOperationV1::Entry(e) = &frozen.operations[0] else {
        panic!()
    };
    assert_eq!(e.value_date, "2024-06-30");
    assert_eq!(e.postings[0].original.effective, instant);
    // A reversal uses the original frozen policy, not this newly changed policy cache.
    let correction = AccountingIntent::Correction(CorrectionV1 {
        active_recognition: RecognitionId(id(10).0),
        replacement: vec![],
        remapping: vec![],
        reason: "Synthetic correction".into(),
    });
    let reversed = AccountingAdapter::freeze(&correction, &s).unwrap();
    let FrozenNativeOperationV1::Entry(e) = &reversed.operations[0] else {
        panic!("missing reversal")
    };
    assert_eq!(e.booking_date, "2024-07-02");
    assert_eq!(e.value_date, "2024-06-30");
    assert!(e.reverses.is_some());
    let mut r = recognition(11, 100, 2);
    for p in &mut r.postings {
        p.effective = unknown();
    }
    let b = AccountingAdapter::freeze(&AccountingIntent::Recognition(r), &s).unwrap();
    let FrozenNativeOperationV1::Entry(e) = &b.operations[0] else {
        panic!("missing entry")
    };
    assert_eq!(e.value_date, e.booking_date);
    assert!(matches!(
        e.postings[0].original.effective,
        TemporalValue::Unknown { .. }
    ));
}
#[test]
fn cur_009_3() {
    let s = declared();
    for u in [
        unit(9, "USD", 2),
        unit(1, "USD", 3),
        unit(9, "TOOLONGTICKER123456789", 18),
        unit(9, "00000000-0000-0000-0000-000000000009", 18),
    ] {
        assert!(
            AccountingAdapter::freeze(
                &AccountingIntent::MetadataDeclaration(MetadataDeclarationV1 {
                    accounts: vec![],
                    units: vec![u]
                }),
                &s
            )
            .is_err()
        );
    }
    let mut stale = s.clone();
    stale.through = CommitSeq(99);
    assert!(
        AccountingAdapter::freeze(
            &AccountingIntent::Recognition(recognition(10, 1, 2)),
            &stale
        )
        .is_err()
    );
    let mut wrong_role = s.clone();
    wrong_role.records.remove(&id(900));
    assert!(
        AccountingAdapter::freeze(
            &AccountingIntent::Recognition(recognition(10, 1, 2)),
            &wrong_role
        )
        .is_err()
    );
    let mut too_many = recognition(10, 1, 2);
    too_many.postings = (0..1026)
        .map(|i| {
            let mut p = too_many.postings[i % 2].clone();
            p.id = id(10_000 + i as u128);
            p
        })
        .collect();
    assert!(AccountingAdapter::freeze(&AccountingIntent::Recognition(too_many), &s).is_err());
    let mut r = recognition(10, 1, 2);
    r.bookkeeping_date = date("0000-01-01");
    for p in &mut r.postings {
        p.posting = date("0000-01-01");
    }
    assert!(AccountingAdapter::freeze(&AccountingIntent::Recognition(r), &s).is_err());
    let same = unit(9, "ABCDEFGHIJKLMNOP", 18);
    let other = unit(10, "ABCDEFGHIJKLMNOP", 18);
    assert!(
        AccountingAdapter::freeze(
            &AccountingIntent::MetadataDeclaration(MetadataDeclarationV1 {
                accounts: vec![],
                units: vec![same, other]
            }),
            &s
        )
        .is_err()
    );
}
fn result(n: u64) -> CommandResultV1 {
    CommandResultV1 {
        command_id: CommandId(id(80_000 + u128::from(n)).0),
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
    }
}
async fn persist(
    store: &currents_storage::EncryptedStore,
    state: &mut DomainState,
    batch: &AccountingBatchV1,
    extra: Vec<AuthoritativeRecordV1>,
) -> DomainCommitV1 {
    assert_eq!(batch.operations.len(), 1);
    let manifest =
        AccountingAdapter::manifest(id(70_000 + u128::from(state.through.0)), batch).unwrap();
    let bytes = AccountingAdapter::instruction_bytes(batch).unwrap();
    let staged = store.stage(&bytes).await.unwrap();
    let mut records = extra;
    records.push(AuthoritativeRecordV1::FrozenAccountingManifest(manifest));
    let committed = store
        .writer()
        .append(currents_storage::HistoryAppend {
            prior_head: state.through,
            records,
            decisions: vec![],
            result: result(state.through.0 + 1),
            staged: vec![staged],
            native: AccountingAdapter::prepare_operations(batch).unwrap().pop(),
        })
        .await
        .unwrap();
    AccountingAdapter::advance_index(state, &committed, &bytes).unwrap();
    committed
}
#[tokio::test]
async fn cur_009_4() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic.db");
    let store = currents_storage::EncryptedStore::open(
        &path,
        currents_cipher::DatasetKey::new([7; 32]),
        currents_cipher::CipherProfileV1,
    )
    .await
    .unwrap();
    let empty = store
        .writer()
        .append(currents_storage::HistoryAppend {
            prior_head: CommitSeq(0),
            records: vec![],
            decisions: vec![],
            result: result(1),
            staged: vec![],
            native: None,
        })
        .await
        .unwrap();
    let mut s = state();
    s.accounting = Some(AccountingIndexV1::empty(empty.native_head_after.clone()));
    AccountingAdapter::advance_index(&mut s, &empty, &[]).unwrap();
    // Existing C04 seam commits one metadata operation at a time. No plural executor.
    let declarations = metadata(&s);
    for (i, op) in declarations.operations.into_iter().enumerate() {
        let mut one = AccountingBatchV1 {
            schema_version: 1,
            native_semantics: 1,
            operations: vec![op],
            preconditions: s.accounting.as_ref().unwrap().prefix.clone(),
            decision_digest: Digest([0; 32]),
            result_expectations: AccountingExpectationsV1 {
                native_operation_count: CommitSeq(1),
                financial_entry_count: CommitSeq(0),
            },
        };
        one.decision_digest = AccountingAdapter::prebinding_digest(&one).unwrap();
        let extra = if i == 0 {
            vec![AuthoritativeRecordV1::DateConversionPolicy(policy())]
        } else {
            vec![]
        };
        let commit = persist(&store, &mut s, &one, extra).await;
        assert_eq!(
            s.accounting.as_ref().unwrap().prefix.native_head,
            commit.native_head_after
        );
    }
    let b = AccountingAdapter::freeze(&AccountingIntent::Recognition(recognition(10, 100, 2)), &s)
        .unwrap();
    let encoded = serde_json_canonicalizer::to_vec(&b).unwrap();
    let replay: AccountingBatchV1 = serde_json::from_slice(&encoded).unwrap();
    let manifest = AccountingAdapter::manifest(id(999), &b).unwrap();
    let instructions = AccountingAdapter::instruction_bytes(&b).unwrap();
    assert_eq!(
        AccountingAdapter::from_manifest(&manifest, &instructions).unwrap(),
        b
    );
    let mut corrupted_bytes = instructions.clone();
    corrupted_bytes.push(b' ');
    assert!(AccountingAdapter::from_manifest(&manifest, &corrupted_bytes).is_err());
    if let Ok(path) = std::env::var("CURRENTS_C05_VECTOR_OUTPUT") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({"manifest":manifest,"batch":b})).unwrap(),
        )
        .unwrap();
    }
    let golden: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/v1/accounting-freeze.json"
    ))
    .unwrap();
    assert_eq!(golden, serde_json::json!({"manifest":manifest,"batch":b}));
    assert_eq!(
        AccountingAdapter::instruction_bytes(&b).unwrap(),
        AccountingAdapter::instruction_bytes(&replay).unwrap()
    );
    assert_eq!(
        AccountingAdapter::prebinding_digest(&b).unwrap(),
        b.decision_digest
    );
    let mut corrupted = b.clone();
    let FrozenNativeOperationV1::Entry(e) = &mut corrupted.operations[0] else {
        panic!()
    };
    e.generated_document = Digest([9; 32]);
    assert_eq!(
        AccountingAdapter::prebinding_digest(&corrupted).unwrap(),
        b.decision_digest
    );
    assert!(AccountingAdapter::validate_limits(&corrupted).is_err());
    let mut r = recognition(11, 0, 0);
    assert!(AccountingAdapter::freeze(&AccountingIntent::Recognition(r.clone()), &s).is_err());
    r.postings.clear();
    assert!(AccountingAdapter::freeze(&AccountingIntent::Recognition(r), &s).is_err());
    persist(&store, &mut s, &b, vec![]).await;
    let reverse = AccountingAdapter::freeze(
        &AccountingIntent::Correction(CorrectionV1 {
            active_recognition: RecognitionId(id(10).0),
            replacement: vec![],
            remapping: vec![],
            reason: "Synthetic".into(),
        }),
        &s,
    )
    .unwrap();
    persist(&store, &mut s, &reverse, vec![]).await;
    let final_head = s.accounting.as_ref().unwrap().prefix.native_head.clone();
    let commits = store.history().read_prefix(s.through).await.unwrap();
    assert_eq!(commits.last().unwrap().native_head_after, final_head);
    store.close().await.unwrap();
    let reopened = currents_storage::EncryptedStore::open(
        &path,
        currents_cipher::DatasetKey::new([7; 32]),
        currents_cipher::CipherProfileV1,
    )
    .await
    .unwrap();
    assert_eq!(
        reopened.history().read_prefix(s.through).await.unwrap(),
        commits
    );
    let replay_pool = currents_cipher::open_pool(
        &directory.path().join("replay.db"),
        currents_cipher::DatasetKey::new([8; 32]),
        currents_cipher::CipherProfileV1,
        2,
    )
    .await
    .unwrap();
    let replay_store =
        currents_accounting::AtomicStore::from_initialized_pool(replay_pool.pool().clone(), true)
            .await
            .unwrap();
    for commit in &commits {
        for record in &commit.records {
            if let AuthoritativeRecordV1::FrozenAccountingManifest(m) = record {
                let mut content = reopened
                    .history()
                    .stream_content(commit.seq, m.instructions.digest)
                    .await
                    .unwrap();
                let mut bytes = vec![];
                while let Some(chunk) = content.next().await.unwrap() {
                    bytes.extend(chunk);
                }
                let frozen = AccountingAdapter::from_manifest(m, &bytes).unwrap();
                for op in AccountingAdapter::prepare_operations(&frozen).unwrap() {
                    let tx = replay_store
                        .begin_authority(CommandId(id(90_000).0), Some(&op))
                        .await
                        .unwrap();
                    assert_eq!(tx.native_head(), commit.native_head_after);
                    tx.commit().await.unwrap();
                }
            }
        }
    }
    replay_pool.close().await;
    reopened.close().await.unwrap();
}
