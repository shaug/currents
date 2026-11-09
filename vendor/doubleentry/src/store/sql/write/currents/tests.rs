#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::store::Store;
use crate::{AccountKind, Amount, Entry, Ledger, LedgerId, currency::USD, date};
use sqlx::Connection;

async fn setup() -> (
    tempfile::TempDir,
    String,
    SqlStore<Sqlite, 18>,
    Ledger<SqlStore<Sqlite, 18>, 18>,
) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("synthetic.db").display());
    let store = SqlStore::<Sqlite, 18>::sqlite(&url).await.unwrap();
    let books = Ledger::<_, 18>::with_precision(
        store.clone(),
        LedgerId::new("synthetic.currents/atomic").unwrap(),
    )
    .await
    .unwrap();
    books
        .declare_currency(USD, 2, date!("0001-01-01"))
        .await
        .unwrap();
    books
        .open("Assets:Checking", AccountKind::Asset, date!("0001-01-01"))
        .await
        .unwrap();
    books
        .open(
            "Expenses:Purchase",
            AccountKind::Expense,
            date!("0001-01-01"),
        )
        .await
        .unwrap();
    books
        .define_period("all", date!("0001-01-01"), date!("9999-12-31"))
        .await
        .unwrap();
    store.currents_initialize().await.unwrap();
    (dir, url, store, books)
}
fn entry(key: &str, credit: &str) -> Op<18> {
    Op::Record(Box::new(
        Entry::new(key, date!("2026-11-09"))
            .debit(
                "Expenses:Purchase",
                "96.00".parse::<Amount<18>>().unwrap(),
                USD,
            )
            .credit(
                "Assets:Checking",
                credit.parse::<Amount<18>>().unwrap(),
                USD,
            ),
    ))
}
async fn count(url: &str) -> i64 {
    let mut connection = sqlx::SqliteConnection::connect(url).await.unwrap();
    let count = sqlx::query_scalar("SELECT count(*) FROM currents_atomic_records")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    count
}
#[tokio::test]
async fn atomic_rollback() {
    let (_dir, url, store, books) = setup().await;
    let before = books.head().await.unwrap();
    // Native persistence precedes this real SQL CHECK failure.
    let error = store
        .currents_write(&entry("write-failure", "96.00"), &[1; 16], b"")
        .await
        .unwrap_err();
    assert!(!error.may_have_applied());
    assert_eq!(count(&url).await, 0);
    assert_eq!(books.head().await.unwrap(), before);
    assert!(
        store
            .currents_write(
                &entry("native-failure", "95.00"),
                &[2; 16],
                b"synthetic record"
            )
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(count(&url).await, 0);
    assert_eq!(books.head().await.unwrap(), before);
    let Op::Record(invalid) = entry("batch-invalid", "95.00") else {
        unreachable!()
    };
    let rejected = store
        .currents_write(
            &Op::Batch(vec![crate::core::BatchItem::Record(*invalid)]),
            &[9; 16],
            b"synthetic rejected batch",
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(rejected, Done::BatchRejected { .. }));
    assert_eq!(count(&url).await, 0);
    assert_eq!(books.head().await.unwrap(), before);
    let op = entry("success", "96.00");
    let first = store
        .currents_write(&op, &[3; 16], b"synthetic record")
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(first, Done::Entry(_)));
    let head = books.head().await.unwrap();
    assert!(
        matches!(store.currents_write(&op,&[3;16],b"synthetic record").await.unwrap().unwrap(),Done::Entry(r) if r.replayed)
    );
    assert_eq!(books.head().await.unwrap(), head);
    assert_eq!(count(&url).await, 1);
    assert!(
        store
            .currents_write(&entry("other", "96.00"), &[3; 16], b"synthetic record")
            .await
            .is_err()
    );
    assert!(
        store
            .currents_write(&op, &[3; 16], b"changed record")
            .await
            .is_err()
    );
    assert_eq!(books.head().await.unwrap(), head);
    assert_eq!(count(&url).await, 1);
    books.audit().await.unwrap();
    store.close().await;
}
#[tokio::test]
async fn commit_uncertainty() {
    for (mode, expected) in [(CommitMode::IoBefore, 0), (CommitMode::IoAfter, 1)] {
        let (_dir, url, store, books) = setup().await;
        let before = books.head().await.unwrap();
        let error = store
            .currents_write_inner(
                &entry("uncertain", "96.00"),
                &[4; 16],
                b"synthetic record",
                mode,
            )
            .await
            .unwrap_err();
        assert!(
            error.may_have_applied(),
            "COMMIT I/O must not imply rollback"
        );
        // No retry or success classification on the uncertain live handle.
        store.close().await;
        let reopened = SqlStore::<Sqlite, 18>::sqlite(&url).await.unwrap();
        reopened.open().await.unwrap();
        let audited = Ledger::<_, 18>::with_precision(
            reopened.clone(),
            LedgerId::new("synthetic.currents/atomic").unwrap(),
        )
        .await
        .unwrap();
        audited.audit().await.unwrap();
        assert_eq!(count(&url).await, expected);
        assert_eq!(
            audited.head().await.unwrap().size,
            before.size + u64::try_from(expected).unwrap()
        );
        reopened.close().await;
    }
}
#[tokio::test]
async fn upstream_conformance() {
    // The unchanged complete upstream suite is invoked separately by cur_005_3.
    let (_dir, _url, store, books) = setup().await;
    let op = entry("canonical", "96.00");
    store
        .currents_write(&op, &[5; 16], b"synthetic record")
        .await
        .unwrap()
        .unwrap();
    books.audit().await.unwrap();
    store.close().await;
}
#[tokio::test]
async fn schema_guard() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("foreign.db").display());
    let store = SqlStore::<Sqlite, 18>::sqlite(&url).await.unwrap();
    let mut connection = sqlx::SqliteConnection::connect(&url).await.unwrap();
    sqlx::query("CREATE TABLE foreign_data(value TEXT)")
        .execute(&mut connection)
        .await
        .unwrap();
    let genesis = crate::record::Genesis {
        ledger: LedgerId::new("synthetic.currents/foreign").unwrap(),
    };
    assert!(matches!(
        store.create(&genesis).await.unwrap_err().kind(),
        crate::store::StoreErrorKind::ForeignSchema { .. }
    ));
    assert!(store.currents_initialize().await.is_err());
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table'")
            .fetch_all(&mut connection)
            .await
            .unwrap();
    assert_eq!(names, vec!["foreign_data"]);
    connection.close().await.unwrap();
    store.close().await;
    let (_dir, _url, store, books) = setup().await;
    store.open().await.unwrap();
    store.currents_initialize().await.unwrap();
    books.audit().await.unwrap();
    store.close().await;
}
