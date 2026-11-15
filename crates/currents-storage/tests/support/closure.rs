//! Real C05 fixtures for C04 multi-operation closure qualification.
//!
//! Callers supply the actual empty native head, persist each returned batch with
//! its records and manifest, then advance from the resulting storage commit.
use currents_accounting::AccountingAdapter;
use currents_types::*;

pub const RECOGNITION_COUNT: usize = 600;

/// Keep every fixture identifier away from the command and manifest identifiers
/// chosen by the integration test, and keep each record family disjoint.
pub fn id(n: u128) -> RecordId {
    RecordId(uuid::Uuid::from_u128(
        0xc007_0000_0000_0000_0000_0000_0000_0000 | n,
    ))
}

pub fn initial_state(empty_native_head: NativeHead) -> DomainState {
    DomainState {
        accounting: Some(AccountingIndexV1::empty(empty_native_head)),
        ..DomainState::default()
    }
}

fn date(text: &str) -> TemporalValue {
    serde_json::from_value(serde_json::json!({
        "kind": "Date", "civil": text, "original_text": text
    }))
    .unwrap()
}

fn policy() -> DateConversionPolicyV1 {
    DateConversionPolicyV1 {
        id: id(900),
        version: 1,
        conversion: DateConversionV1::CivilOnly,
        bookkeeping: BookkeepingFallbackV1::Explicit,
    }
}

fn account(n: u128, class: AccountClass) -> AccountComponentV1 {
    AccountComponentV1 {
        id: id(n + 100),
        account_id: AccountId(id(n).0),
        account_class: class,
        unit_id: UnitId(id(1).0),
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
        display_name: format!("Synthetic closure {class:?}"),
    }
}

/// The date policy, unit catalog and account components are authority records,
/// not merely native declarations, so later recognition references resolve.
pub fn metadata(state: &DomainState) -> (AccountingBatchV1, Vec<AuthoritativeRecordV1>) {
    let unit = UnitDefinition {
        id: UnitId(id(1).0),
        exponent: 2,
        native_code: "USD".into(),
        kind: UnitKind::Currency,
    };
    let accounts = vec![
        account(2, AccountClass::Expense),
        account(3, AccountClass::Asset),
    ];
    let batch = AccountingAdapter::freeze(
        &AccountingIntent::MetadataDeclaration(MetadataDeclarationV1 {
            accounts: accounts.clone(),
            units: vec![unit.clone()],
        }),
        state,
    )
    .unwrap();
    let mut records = vec![
        AuthoritativeRecordV1::DateConversionPolicy(policy()),
        AuthoritativeRecordV1::UnitCatalog(UnitCatalogV1 {
            id: id(4),
            version: 1,
            units: vec![unit],
        }),
    ];
    records.extend(
        accounts
            .into_iter()
            .map(AuthoritativeRecordV1::AccountComponent),
    );
    (batch, records)
}

fn recognition(position: usize, replacement: bool) -> RecognitionV1 {
    let offset = position as u128;
    let recognition_base = if replacement { 0x2000 } else { 0x1000 };
    let posting_base = if replacement { 0x20000 } else { 0x10000 };
    let amount = if replacement { 200 } else { 100 } + position as i128;
    RecognitionV1 {
        id: RecognitionId(id(recognition_base + offset).0),
        lineage: id(0x3000 + offset),
        postings: vec![
            PostingV1 {
                id: id(posting_base + 2 * offset),
                account_id: AccountId(id(2).0),
                amount: Amount {
                    coefficient: amount,
                    scale: 2,
                    unit_id: UnitId(id(1).0),
                },
                effective: date("2024-06-30"),
                posting: date("2024-07-02"),
            },
            PostingV1 {
                id: id(posting_base + 2 * offset + 1),
                account_id: AccountId(id(3).0),
                amount: Amount {
                    coefficient: -amount,
                    scale: 2,
                    unit_id: UnitId(id(1).0),
                },
                effective: date("2024-06-30"),
                posting: date("2024-07-02"),
            },
        ],
        kind: if replacement {
            RecognitionKind::Replacement
        } else {
            RecognitionKind::Activity
        },
        evidence_allocations: vec![],
        leg_ids: vec![],
        scope: DateScope {
            start: None,
            end: None,
            basis: DateBasis::Effective,
        },
        bookkeeping_date: date("2024-07-02"),
        date_policy: PolicyReferenceV1 {
            id: policy().id,
            version: 1,
        },
    }
}

/// Combine already-frozen C05 operations at their shared committed prefix and
/// bind their generated documents to the aggregate decision digest. This never
/// constructs native operations or modifies the replay index.
fn combine(state: &DomainState, operations: Vec<FrozenNativeOperationV1>) -> AccountingBatchV1 {
    let mut batch = AccountingAdapter::freeze(
        &AccountingIntent::NoEffect(NoEffectV1 {
            reason: "Synthetic closure composition".into(),
        }),
        state,
    )
    .unwrap();
    batch.operations = operations;
    batch.result_expectations = AccountingExpectationsV1 {
        native_operation_count: CommitSeq(batch.operations.len() as u64),
        financial_entry_count: CommitSeq(
            batch
                .operations
                .iter()
                .filter(|operation| matches!(operation, FrozenNativeOperationV1::Entry(_)))
                .count() as u64,
        ),
    };
    batch.decision_digest = AccountingAdapter::prebinding_digest(&batch).unwrap();
    for operation in &mut batch.operations {
        if let FrozenNativeOperationV1::Entry(entry) = operation {
            entry.generated_document = batch.decision_digest;
        }
    }
    AccountingAdapter::validate_limits(&batch).unwrap();
    batch
}

pub fn originals(state: &DomainState) -> (AccountingBatchV1, Vec<AuthoritativeRecordV1>) {
    let mut operations = Vec::with_capacity(RECOGNITION_COUNT);
    let mut records = Vec::with_capacity(RECOGNITION_COUNT);
    for position in 0..RECOGNITION_COUNT {
        let recognition = recognition(position, false);
        let batch =
            AccountingAdapter::freeze(&AccountingIntent::Recognition(recognition.clone()), state)
                .unwrap();
        operations.extend(batch.operations);
        records.push(AuthoritativeRecordV1::Recognition(recognition));
    }
    (combine(state, operations), records)
}

/// Freeze every correction against replayed originals, then order all reversals
/// before all replacements to exercise the 600-operation native chunk boundary.
pub fn correction(state: &DomainState) -> (AccountingBatchV1, Vec<AuthoritativeRecordV1>) {
    let mut reversals = Vec::with_capacity(RECOGNITION_COUNT);
    let mut replacements = Vec::with_capacity(RECOGNITION_COUNT);
    let mut records = Vec::with_capacity(RECOGNITION_COUNT);
    for position in 0..RECOGNITION_COUNT {
        let replacement = recognition(position, true);
        let batch = AccountingAdapter::freeze(
            &AccountingIntent::Correction(CorrectionV1 {
                active_recognition: recognition(position, false).id,
                replacement: vec![replacement.clone()],
                remapping: vec![],
                reason: format!("Synthetic closure correction {position}"),
            }),
            state,
        )
        .unwrap();
        assert_eq!(batch.operations.len(), 2);
        for operation in batch.operations {
            match &operation {
                FrozenNativeOperationV1::Entry(entry) if entry.reverses.is_some() => {
                    reversals.push(operation);
                }
                FrozenNativeOperationV1::Entry(_) => replacements.push(operation),
                _ => panic!("correction must contain only financial entries"),
            }
        }
        records.push(AuthoritativeRecordV1::Recognition(replacement));
    }
    assert_eq!(reversals.len(), RECOGNITION_COUNT);
    assert_eq!(replacements.len(), RECOGNITION_COUNT);
    reversals.extend(replacements);
    (combine(state, reversals), records)
}

/// Advance only from the real commit returned by storage. A fresh replay uses
/// the persisted manifest's instruction bytes with the same production API.
pub fn advance(state: &mut DomainState, committed: &DomainCommitV1, batch: &AccountingBatchV1) {
    AccountingAdapter::advance_index(
        state,
        committed,
        &AccountingAdapter::instruction_bytes(batch).unwrap(),
    )
    .unwrap();
}
