//! Source-refresh smoke test only, not key-initialization/encryption qualification.
use sqlx::{Connection, SqliteConnection};

#[tokio::test]
async fn cur_001_fixed_native_source() {
    let mut connection = SqliteConnection::connect("sqlite::memory:")
        .await
        .expect("synthetic in-memory connection");
    let sqlite: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(sqlite, "3.54.1");
    let cipher: String = sqlx::query_scalar("PRAGMA cipher_version")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert!(
        cipher.starts_with("4.20.0"),
        "unexpected SQLCipher: {cipher}"
    );
    connection.close().await.unwrap();
}
