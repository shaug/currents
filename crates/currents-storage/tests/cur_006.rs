//! Public private-library boundaries, using synthetic files and keys only.
use currents_cipher::{CipherProfileV1, DatasetKey, connection_options, open_pool};
use currents_storage::EncryptedStore;
use sqlx::{ConnectOptions, Connection, Executor, Row, SqliteConnection};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

// Values are never printed in failures, logs, command arguments or reports.
fn key() -> DatasetKey {
    DatasetKey::new([0x73; 32])
}
fn profile() -> CipherProfileV1 {
    CipherProfileV1
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn clean(bytes: &[u8]) {
    for canary in [
        b"CUR006_SYNTHETIC_FINANCIAL_CANARY".as_slice(),
        &[0x73; 32],
        b"7373737373737373737373737373737373737373737373737373737373737373",
    ] {
        assert!(
            !bytes.windows(canary.len()).any(|part| part == canary),
            "private canary found in artifact"
        );
    }
}
fn scan(path: &Path) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan(&path);
        } else {
            clean(&std::fs::read(path).unwrap());
        }
    }
}
async fn configured(connection: &mut SqliteConnection) {
    for (pragma, expected) in [
        ("PRAGMA foreign_keys", 1),
        ("PRAGMA recursive_triggers", 1),
        ("PRAGMA synchronous", 2),
        ("PRAGMA temp_store", 2),
    ] {
        let value: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(pragma))
            .fetch_one(&mut *connection)
            .await
            .unwrap();
        assert_eq!(value, expected, "profile setting differs: {pragma}");
    }
    for (pragma, expected) in [
        ("PRAGMA cipher_page_size", "4096"),
        ("PRAGMA cipher_use_hmac", "1"),
        ("PRAGMA cipher_plaintext_header_size", "0"),
    ] {
        let value: String = sqlx::query_scalar(sqlx::AssertSqlSafe(pragma))
            .fetch_one(&mut *connection)
            .await
            .unwrap();
        assert_eq!(value, expected, "cipher setting differs");
    }
    let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    assert_eq!(journal, "wal");
    let hmac: String = sqlx::query_scalar("PRAGMA cipher_hmac_algorithm")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    assert_eq!(hmac, "HMAC_SHA512");
}

#[test]
fn cur_006_1() {
    // A build without the codec is rejected by the actual pinned sys build,
    // before an executable could silently create plaintext storage.
    let fixture = tempfile::tempdir().unwrap();
    std::fs::create_dir(fixture.path().join("src")).unwrap();
    let sys = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/libsqlite3-sys");
    std::fs::write(fixture.path().join("Cargo.toml"), format!(
        "[package]\nname=\"cur006-missing-codec\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\nlibsqlite3-sys={{path={:?},default-features=true}}\n", sys
    )).unwrap();
    std::fs::write(
        fixture.path().join("src/lib.rs"),
        "pub use libsqlite3_sys as sys;\n",
    )
    .unwrap();
    let manifest = fixture.path().join("Cargo.toml");
    let lock = std::process::Command::new("cargo")
        .args(["generate-lockfile", "--offline", "--manifest-path"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        lock.status.success(),
        "missing-codec fixture resolution failed"
    );
    let denied = std::process::Command::new("cargo")
        .args(["build", "--locked", "--offline", "--manifest-path"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        !denied.status.success(),
        "build without SQLCipher was accepted"
    );
    assert!(
        String::from_utf8_lossy(&denied.stderr)
            .contains("Currents requires bundled SQLCipher; stock/system SQLite is disabled"),
        "missing-codec guard was not reached: {}",
        String::from_utf8_lossy(&denied.stderr)
    );
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.db");
        let store = EncryptedStore::open(&path, key(), profile())
            .await
            .expect("encrypted native initialization");
        assert!(!store.attestation().schema_sha256.is_empty());
        assert!(!store.attestation().migration_sha256.is_empty());
        store.close().await.unwrap();
        let original = std::fs::read(&path).unwrap();
        assert!(!original.starts_with(b"SQLite format 3"));
        let secured = open_pool(&path, key(), profile(), 2).await.unwrap();
        let pool = secured.pool();
        let cipher: String = sqlx::query_scalar("PRAGMA cipher_version")
            .fetch_one(pool)
            .await
            .unwrap();
        assert!(cipher.starts_with("4.20.0"));
        let sqlite: String = sqlx::query_scalar("SELECT sqlite_version()")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(sqlite, "3.54.1");
        let native: i64 = sqlx::query_scalar("SELECT count(*) FROM meta")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(native, 1);
        let namespaced: i64 = sqlx::query_scalar("SELECT count(*) FROM currents_atomic_records")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(namespaced, 0);
        secured.close().await;
        assert!(
            EncryptedStore::open(&path, DatasetKey::new([0x46; 32]), profile())
                .await
                .is_err()
        );
        let mut no_key = connection_options(&path).connect().await.unwrap();
        assert!(
            sqlx::query("SELECT count(*) FROM sqlite_schema")
                .fetch_all(&mut no_key)
                .await
                .is_err()
        );
        no_key.close().await.unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            original,
            "rejected key changed ciphertext"
        );
        let reopened = EncryptedStore::open(&path, key(), profile()).await.unwrap();
        reopened.close().await.unwrap();
        // A keyed file with foreign objects must not be adopted as a native dataset.
        let foreign = dir.path().join("foreign.db");
        let secured = open_pool(&foreign, key(), profile(), 1).await.unwrap();
        let pool = secured.pool();
        pool.execute("CREATE TABLE unexpected (id INTEGER)")
            .await
            .unwrap();
        secured.close().await;
        assert!(
            EncryptedStore::open(&foreign, key(), profile())
                .await
                .is_err()
        );
        // Reopen must attest objects, not trust a stored version number alone.
        let secured = open_pool(&path, key(), profile(), 1).await.unwrap();
        let pool = secured.pool();
        pool.execute("DROP TRIGGER meta_no_delete").await.unwrap();
        secured.close().await;
        assert!(EncryptedStore::open(&path, key(), profile()).await.is_err());
        scan(dir.path());
    });
}

#[derive(Clone)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for LogCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn cur_006_2() {
    if let Ok(directory) = std::env::var("CURRENTS_CUR006_CRASH_DIR") {
        runtime().block_on(async {
            let secured = open_pool(&Path::new(&directory).join("crash.db"), key(), profile(), 1)
                .await
                .unwrap();
            let pool = secured.pool();
            pool.execute("CREATE TABLE staging (payload BLOB)")
                .await
                .unwrap();
            sqlx::query("INSERT INTO staging VALUES (?)")
                .bind(b"CUR006_SYNTHETIC_FINANCIAL_CANARY".as_slice())
                .execute(pool)
                .await
                .unwrap();
            let mut tx = pool.begin().await.unwrap();
            tx.execute("UPDATE staging SET payload = zeroblob(8192)")
                .await
                .unwrap();
            // Immediate process exit deliberately skips pool/transaction destructors.
            std::process::exit(23);
        });
    }
    let dir = tempfile::tempdir().unwrap();
    runtime().block_on(async {
        let path = dir.path().join("canary.db");
        let secured = open_pool(&path, key(), profile(), 1).await.unwrap();
        let pool = secured.pool();
        let mut connection = pool.acquire().await.unwrap();
        configured(&mut connection).await;
        connection
            .execute("CREATE TABLE staging (payload BLOB)")
            .await
            .unwrap();
        sqlx::query("INSERT INTO staging VALUES (?)")
            .bind(b"CUR006_SYNTHETIC_FINANCIAL_CANARY".as_slice())
            .execute(&mut *connection)
            .await
            .unwrap();
        assert!(
            dir.path().join("canary.db-wal").exists(),
            "WAL artifact not exercised"
        );
        connection
            .execute("CREATE TEMP TABLE memory_only AS SELECT payload FROM staging")
            .await
            .unwrap();
        let temp: String = sqlx::query("PRAGMA database_list")
            .fetch_all(&mut *connection)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.get::<String, _>("name") == "temp")
            .unwrap()
            .get("file");
        assert!(temp.is_empty(), "temporary storage is on disk");
        scan(dir.path());
        connection
            .execute("PRAGMA wal_checkpoint(TRUNCATE)")
            .await
            .unwrap();
        // Archive rollback-journal artifacts also use the initialized codec.
        connection
            .execute("PRAGMA journal_mode=DELETE")
            .await
            .unwrap();
        connection.execute("BEGIN IMMEDIATE").await.unwrap();
        let changed_payload = b"CUR006_SYNTHETIC_FINANCIAL_CANARY".repeat(4096);
        sqlx::query("UPDATE staging SET payload = ?")
            .bind(changed_payload)
            .execute(&mut *connection)
            .await
            .unwrap();
        assert!(
            dir.path().join("canary.db-journal").exists(),
            "rollback artifact not exercised"
        );
        scan(dir.path());
        connection.execute("ROLLBACK").await.unwrap();
        drop(connection);
        secured.close().await;
        scan(dir.path());
    });
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cur_006_2"])
        .env("CURRENTS_CUR006_CRASH_DIR", dir.path())
        .output()
        .unwrap();
    assert_eq!(
        child.status.code(),
        Some(23),
        "crash scenario did not reach its boundary"
    );
    clean(&child.stdout);
    clean(&child.stderr);
    assert!(
        dir.path().join("crash.db-wal").exists(),
        "crash WAL not retained"
    );
    scan(dir.path());
}

#[test]
fn cur_006_3() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let capture = LogCapture(log.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || capture.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();
    tracing::info!("capture established");
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pool.db");
        let options = format!("{:?}", connection_options(&path));
        clean(options.as_bytes());
        let secured = open_pool(&path, key(), profile(), 4).await.unwrap();
        let pool = secured.pool();
        pool.execute("CREATE TABLE sentinel (value INTEGER)")
            .await
            .unwrap();
        pool.execute("INSERT INTO sentinel VALUES (7)")
            .await
            .unwrap();
        // Exercise the slow-query logger above SQLx's default one-second limit.
        let start = std::time::Instant::now();
        let _: i64 = sqlx::query_scalar("WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<50000000) SELECT sum(x) FROM n")
            .fetch_one(pool).await.unwrap();
        assert!(start.elapsed() >= std::time::Duration::from_secs(1), "slow-query boundary was not exercised");
        // Hold every slot simultaneously, then destroy each underlying connection.
        for _ in 0..3 {
            let mut slots = Vec::new();
            for _ in 0..4 {
                slots.push(pool.acquire().await.unwrap());
            }
            for mut slot in slots {
                configured(&mut slot).await;
                let n: i64 = sqlx::query_scalar("SELECT value FROM sentinel")
                    .fetch_one(&mut *slot)
                    .await
                    .unwrap();
                assert_eq!(n, 7);
                // Bound payload values must not enter query diagnostics.
                sqlx::query("SELECT ?")
                    .bind(b"CUR006_SYNTHETIC_FINANCIAL_CANARY".as_slice())
                    .fetch_one(&mut *slot)
                    .await
                    .unwrap();
                slot.close().await.unwrap();
            }
        }
        secured.close().await;
    });
    let captured = log.lock().unwrap();
    assert!(!captured.is_empty(), "log capture was not active");
    clean(&captured);
    assert!(
        !String::from_utf8_lossy(&captured).contains("sqlx::query"),
        "SQL statement logging was enabled"
    );
}

#[test]
fn cur_006_4() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cycles.db");
    for cycle in 0..1000 {
        runtime().block_on(async {
            let secured = open_pool(&path, key(), profile(), 1).await.unwrap();
            let pool = secured.pool();
            pool.execute("CREATE TABLE IF NOT EXISTS cycles (n INTEGER)")
                .await
                .unwrap();
            sqlx::query("INSERT INTO cycles VALUES (?)")
                .bind(cycle)
                .execute(pool)
                .await
                .unwrap();
            secured.close().await;
        });
    }
    runtime().block_on(async {
        let secured = open_pool(&path, key(), profile(), 1).await.unwrap();
        let pool = secured.pool();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM cycles")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(n, 1000);
        secured.close().await;
        let unavailable = dir.path().join("unavailable.db");
        assert!(
            EncryptedStore::open_with_key_result(
                &unavailable,
                Err(currents_types::EngineError::invalid()),
                profile()
            )
            .await
            .is_err()
        );
        assert!(!unavailable.exists(), "host key failure created a database");
    });
    scan(dir.path());
}
