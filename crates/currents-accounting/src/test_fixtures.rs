//! Test-only fixed synthetic inputs, not a native command construction API.
//! Enabled only by downstream dev-dependencies, never by production defaults.
use crate::PreparedNativeOperation;
use doubleentry::{currency::USD, date, store::Op};

/// A single native event for qualifying domain-prefix isolation on a fresh dataset.
/// No caller-controlled native values, wire bytes, SQL or handles cross the boundary.
pub fn currency_declaration() -> PreparedNativeOperation {
    PreparedNativeOperation {
        operation: Op::Currency {
            code: USD,
            exponent: 2,
            effective: date!("0001-01-01"),
            actor: "synthetic".into(),
        },
    }
}

/// Fixed synthetic metadata, recorded before previewing the correction.
pub fn closure_metadata() -> Vec<PreparedNativeOperation> {
    use doubleentry::{AccountKind, AccountSpec};
    vec![
        currency_declaration(),
        PreparedNativeOperation {
            operation: Op::Register {
                spec: AccountSpec::new("Assets:Checking", AccountKind::Asset, date!("0001-01-01")),
                actor: "synthetic".into(),
            },
        },
        PreparedNativeOperation {
            operation: Op::Register {
                spec: AccountSpec::new(
                    "Expenses:Purchase",
                    AccountKind::Expense,
                    date!("0001-01-01"),
                ),
                actor: "synthetic".into(),
            },
        },
        PreparedNativeOperation {
            operation: Op::Period {
                id: "all".into(),
                start: date!("0001-01-01"),
                end: date!("9999-12-31"),
            },
        },
    ]
}
/// Fixed deterministic fault sites, compiled only for synthetic qualification.
pub enum ClosureFault {
    DiskFullAfter600,
    PauseAfter600,
    PauseAfter1200,
    LostCommitAcknowledgment,
    PauseBeforeCommit,
}
pub fn fault_control(fault: ClosureFault) -> crate::ClosureControl {
    let control = crate::ClosureControl::default();
    control.test_fault.store(
        match fault {
            ClosureFault::DiskFullAfter600 => 1,
            ClosureFault::PauseAfter600 => 2,
            ClosureFault::PauseAfter1200 => 3,
            ClosureFault::LostCommitAcknowledgment => 4,
            ClosureFault::PauseBeforeCommit => 5,
        },
        std::sync::atomic::Ordering::SeqCst,
    );
    control
}
pub fn release_control(control: &crate::ClosureControl) {
    control.test_release();
}

pub fn observed_sqlite_full(control: &crate::ClosureControl) -> bool {
    control
        .observed_full
        .load(std::sync::atomic::Ordering::SeqCst)
}

/// Exact native and C04 boundaries; only fixed synthetic cases are exposed.
pub enum NativeLimitCase {
    Batch1000,
    Batch1001,
    Postings1024,
    Postings1025,
    Documents16,
    Documents17,
    Leaf65535,
    Leaf65536,
    Closure20000,
    Closure20001,
    Bytes64MiB,
    Bytes64MiBPlus1,
}
pub fn limit_case(case: NativeLimitCase) -> crate::PreparedNativeOperations {
    use NativeLimitCase::*;
    use doubleentry::{BatchItem, Document, Entry, Posting};
    if matches!(case, Bytes64MiB | Bytes64MiBPlus1) {
        return byte_boundary(matches!(case, Bytes64MiBPlus1));
    }
    let mut entry = Entry::new("boundary", date!("2026-01-02"));
    match case {
        Postings1024 | Postings1025 => {
            let count = if matches!(case, Postings1024) {
                1024
            } else {
                1025
            };
            for _ in 0..count - 1 {
                entry = entry.debit("Expenses:Purchase", "1.00".parse().unwrap(), USD);
            }
            entry = entry.credit(
                "Assets:Checking",
                format!("{}.00", count - 1).parse().unwrap(),
                USD,
            );
        }
        Leaf65535 | Leaf65536 => {
            for _ in 0..500 {
                entry = entry.posting(
                    Posting::debit("Expenses:Purchase", "1.00".parse().unwrap(), USD)
                        .dimension("x", "aaaaaa"),
                );
                entry = entry.posting(
                    Posting::credit("Assets:Checking", "1.00".parse().unwrap(), USD)
                        .dimension("x", "aaaaaa"),
                );
            }
            for _ in 0..5 {
                entry = entry.document(Document::new("x", "a".repeat(256)).unwrap());
            }
            entry = entry.document(
                Document::new(
                    "x",
                    "a".repeat(if matches!(case, Leaf65535) { 87 } else { 88 }),
                )
                .unwrap(),
            );
            assert_eq!(
                entry.encoded_len(),
                if matches!(case, Leaf65535) {
                    65535
                } else {
                    65536
                }
            );
        }
        _ => {
            entry = entry
                .debit("Expenses:Purchase", "1.00".parse().unwrap(), USD)
                .credit("Assets:Checking", "1.00".parse().unwrap(), USD);
            if matches!(case, Documents16 | Documents17) {
                for i in 0..if matches!(case, Documents16) { 16 } else { 17 } {
                    entry = entry.document(Document::new("x", format!("document-{i}")).unwrap());
                }
            }
        }
    }
    let operations = match case {
        Batch1000 | Batch1001 => vec![PreparedNativeOperation {
            operation: Op::Batch(vec![
                BatchItem::Record(entry);
                if matches!(case, Batch1000) {
                    1000
                } else {
                    1001
                }
            ]),
        }],
        Closure20000 | Closure20001 => {
            let count = if matches!(case, Closure20000) {
                20000
            } else {
                20001
            };
            let items = vec![BatchItem::Record(entry); count];
            items
                .chunks(1000)
                .map(|items| PreparedNativeOperation {
                    operation: Op::Batch(items.to_vec()),
                })
                .collect()
        }
        _ => vec![PreparedNativeOperation {
            operation: Op::Record(Box::new(entry)),
        }],
    };
    crate::PreparedNativeOperations {
        operations,
        canonical_instructions: b"{\"fixture\":\"native-boundary\"}".to_vec(),
    }
}
fn byte_boundary(over: bool) -> crate::PreparedNativeOperations {
    use doubleentry::{BatchItem, Document, Entry};
    // Deterministic canonical synthetic instruction encoding. All variable bytes
    // are actual native Document references, not padding ignored by execution.
    let prefix = |_i: usize| String::from("{\"booking\":\"2026-01-02\",\"documents\":[");
    let suffix = |i: usize| {
        format!(
            "],\"key\":\"byte-{i}\",\"postings\":[[\"Expenses:Purchase\",\"debit\",\"1.00\",\"USD\"],[\"Assets:Checking\",\"credit\",\"1.00\",\"USD\"]],\"value\":\"2026-01-01\"}}"
        )
    };
    let mut base = 2 + 19999;
    for i in 0..20000 {
        base += prefix(i).len() + suffix(i).len() + 12 * 28 + 11;
    }
    // {"kind":"x","reference":"a"} is28 bytes.
    let target = crate::MAX_CLOSURE_BYTES + usize::from(over);
    let mut remaining = target - base;
    let mut bytes = Vec::with_capacity(target);
    bytes.push(b'[');
    let mut operations = Vec::new();
    let mut batch = Vec::new();
    for i in 0..20000 {
        if i > 0 {
            bytes.push(b',');
        }
        bytes.extend_from_slice(prefix(i).as_bytes());
        let mut entry = Entry::new(format!("byte-{i}"), date!("2026-01-02"))
            .value_date(date!("2026-01-01"))
            .debit("Expenses:Purchase", "1.00".parse().unwrap(), USD)
            .credit("Assets:Checking", "1.00".parse().unwrap(), USD);
        for j in 0..12 {
            if j > 0 {
                bytes.push(b',');
            }
            let extra = remaining.min(255);
            remaining -= extra;
            let reference = "a".repeat(1 + extra);
            bytes.extend_from_slice(
                format!("{{\"kind\":\"x\",\"reference\":\"{reference}\"}}").as_bytes(),
            );
            entry = entry.document(Document::new("x", reference).unwrap());
        }
        bytes.extend_from_slice(suffix(i).as_bytes());
        assert!(entry.encoded_len() <= 65535);
        batch.push(BatchItem::Record(entry));
        if batch.len() == 1000 {
            operations.push(PreparedNativeOperation {
                operation: Op::Batch(std::mem::take(&mut batch)),
            });
        }
    }
    bytes.push(b']');
    assert_eq!(remaining, 0);
    assert_eq!(bytes.len(), target);
    crate::PreparedNativeOperations {
        operations,
        canonical_instructions: bytes,
    }
}
