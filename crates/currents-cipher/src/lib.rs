//! Private, shared SQLCipher initialization. No owner, domain or accounting dependencies.
use sqlx::{
    AssertSqlSafe, ConnectOptions, SqliteConnection, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{
    fmt,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use zeroize::Zeroizing;

/// Host-supplied raw 256-bit secret. It is neither serializable, Debug nor Clone.
/// Clearing is best effort: this cannot erase caller copies, swap or crash memory.
/// ```compile_fail
/// let key = currents_cipher::DatasetKey::new([0; 32]);
/// println!("{key:?}");
/// ```
pub struct DatasetKey(Zeroizing<[u8; 32]>);
impl DatasetKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }
}

/// The only supported profile. Public fields cannot select weaker parameters.
#[derive(Clone, Copy, Default)]
pub struct CipherProfileV1;
impl CipherProfileV1 {
    pub fn crypto_provider(self) -> Result<&'static str, CipherError> {
        if cfg!(target_os = "macos") {
            Ok("commoncrypto")
        } else if cfg!(target_os = "linux") {
            Ok("openssl")
        } else {
            Err(CipherError)
        }
    }
}

/// Sanitized boundary error: no path, SQL, key or database error string escapes.
#[derive(Debug)]
pub struct CipherError;
impl fmt::Display for CipherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("encrypted storage unavailable")
    }
}
impl std::error::Error for CipherError {}

/// Nonsecret connection options. Key pragmas never enter this Debug-able value.
/// Default SQLx initialization only enables FK; it does not read database schema.
pub fn connection_options(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .disable_statement_logging()
        .statement_cache_capacity(0)
}

type RetainedKey = Arc<Mutex<Option<DatasetKey>>>;
/// Trusted adapter plumbing only. Never a client, importer or wire API.
/// Close consumes the owner, awaits every shared connection, then clears the key.
pub struct CipherPool {
    pool: SqlitePool,
    key: RetainedKey,
}
impl CipherPool {
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
    pub async fn close(self) {
        self.pool.close().await;
        if let Ok(mut key) = self.key.lock() {
            *key = None;
        }
    }
}

async fn initialize(
    connection: &mut SqliteConnection,
    key: &RetainedKey,
    profile: CipherProfileV1,
) -> Result<(), CipherError> {
    let version: String = sqlx::query_scalar("PRAGMA cipher_version")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| CipherError)?;
    if version.split_whitespace().next() != Some("4.20.0") {
        return Err(CipherError);
    }
    let mut sql = Zeroizing::new(String::from("PRAGMA key = \"x'"));
    {
        use std::fmt::Write;
        let retained = key.lock().map_err(|_| CipherError)?;
        let secret = retained.as_ref().ok_or(CipherError)?;
        for byte in secret.0.iter() {
            write!(&mut *sql, "{byte:02x}").map_err(|_| CipherError)?;
        }
    }
    sql.push_str("'\"");
    // No persistent statement cache or SQL logs can retain the key statement.
    sqlx::query(AssertSqlSafe(sql.as_str()))
        .persistent(false)
        .execute(&mut *connection)
        .await
        .map_err(|_| CipherError)?;
    drop(sql);
    let provider: String = sqlx::query_scalar("PRAGMA cipher_provider")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| CipherError)?;
    if provider.to_ascii_lowercase() != profile.crypto_provider()? {
        return Err(CipherError);
    }

    sqlx::raw_sql("PRAGMA cipher_compatibility=4; PRAGMA cipher_page_size=4096; PRAGMA cipher_use_hmac=ON; PRAGMA cipher_hmac_algorithm=HMAC_SHA512; PRAGMA cipher_plaintext_header_size=0; PRAGMA cipher_memory_security=ON; PRAGMA temp_store=MEMORY;")
        .execute(&mut *connection).await.map_err(|_| CipherError)?;
    // First schema access authenticates an existing file before mutable pragmas.
    let _: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_schema")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| CipherError)?;
    sqlx::raw_sql("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON;")
        .execute(&mut *connection).await.map_err(|_| CipherError)?;
    let flags: (i64, i64, i64, i64) = sqlx::query_as("SELECT foreign_keys, recursive_triggers, synchronous, temp_store FROM pragma_foreign_keys, pragma_recursive_triggers, pragma_synchronous, pragma_temp_store")
        .fetch_one(&mut *connection).await.map_err(|_| CipherError)?;
    let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| CipherError)?;
    if flags != (1, 1, 2, 2) || journal != "wal" {
        return Err(CipherError);
    }
    Ok(())
}

/// Every initial, lazy, expired or replacement connection passes initialization
/// before pool acquisition can return it or native code can inspect schema.
pub async fn open_pool(
    path: &Path,
    key: DatasetKey,
    profile: CipherProfileV1,
    max_connections: u32,
) -> Result<CipherPool, CipherError> {
    profile.crypto_provider()?;
    if !(1..=16).contains(&max_connections) {
        return Err(CipherError);
    }
    let key = Arc::new(Mutex::new(Some(key)));
    let retained = key.clone();
    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(2))
        .after_connect(move |connection, _| {
            let key = retained.clone();
            Box::pin(async move {
                initialize(connection, &key, profile)
                    .await
                    .map_err(|_| sqlx::Error::Protocol("encrypted initialization rejected".into()))
            })
        })
        .connect_lazy_with(connection_options(path));
    match pool.acquire().await {
        Ok(connection) => drop(connection),
        Err(_) => {
            pool.close().await;
            if let Ok(mut retained) = key.lock() {
                *retained = None;
            }
            return Err(CipherError);
        }
    }
    Ok(CipherPool { pool, key })
}
