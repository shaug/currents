//! Encrypted staging uses the same initialized database, never a plaintext file.
use currents_types::{Digest, EngineError};
use sha2::{Digest as _, Sha256};
use sqlx::{Row, SqlitePool};

pub(crate) const CHUNK: usize = 1_048_576;
pub(crate) const MAX_FILE: usize = 100 * CHUNK;
/// Opaque capability tied to one store; no path, SQL or caller-selected token.
pub struct StagedBatchHandle {
    pub(crate) token: uuid::Uuid,
    pub(crate) digest: Digest,
    pub(crate) size: usize,
}
impl StagedBatchHandle {
    pub fn digest(&self) -> Digest {
        self.digest
    }
}
pub(crate) async fn stage(
    pool: &SqlitePool,
    bytes: &[u8],
) -> Result<StagedBatchHandle, EngineError> {
    if bytes.len() > MAX_FILE {
        return Err(EngineError::invalid());
    }
    let token = uuid::Uuid::new_v4();
    let digest = Digest(Sha256::digest(bytes).into());
    let mut tx = pool.begin().await.map_err(|_| crate::schema::failure())?;
    sqlx::query("INSERT INTO currents_staging(token,digest,byte_count) VALUES(?,?,?)")
        .bind(token.as_bytes().as_slice())
        .bind(digest.0.as_slice())
        .bind(bytes.len() as i64)
        .execute(&mut *tx)
        .await
        .map_err(|_| crate::schema::failure())?;
    for (position, chunk) in bytes.chunks(CHUNK).enumerate() {
        let hash: [u8; 32] = Sha256::digest(chunk).into();
        sqlx::query("INSERT INTO currents_staging_chunks(token,position,bytes,chunk_digest) VALUES(?,?,?,?)").bind(token.as_bytes().as_slice()).bind(position as i64).bind(chunk).bind(hash.as_slice()).execute(&mut *tx).await.map_err(|_|crate::schema::failure())?;
    }
    tx.commit().await.map_err(|_| crate::schema::failure())?;
    Ok(StagedBatchHandle {
        token,
        digest,
        size: bytes.len(),
    })
}
/// Check bounded chunks before they can become authoritative, including resumed files.
pub(crate) async fn verify(
    pool: &SqlitePool,
    staged: &StagedBatchHandle,
) -> Result<(), EngineError> {
    let mut hash = Sha256::new();
    let mut length = 0usize;
    let mut index = 0i64;
    loop {
        let row = sqlx::query(
            "SELECT bytes,chunk_digest FROM currents_staging_chunks WHERE token=? AND position=?",
        )
        .bind(staged.token.as_bytes().as_slice())
        .bind(index)
        .fetch_optional(pool)
        .await
        .map_err(|_| crate::schema::failure())?;
        let Some(row) = row else { break };
        let bytes: Vec<u8> = row.try_get("bytes").map_err(|_| crate::schema::failure())?;
        let digest: Vec<u8> = row
            .try_get("chunk_digest")
            .map_err(|_| crate::schema::failure())?;
        if bytes.len() > CHUNK
            || Sha256::digest(&bytes).as_slice() != digest
            || length.saturating_add(bytes.len()) > MAX_FILE
        {
            return Err(crate::schema::failure());
        }
        hash.update(&bytes);
        length = length.saturating_add(bytes.len());
        index = index.saturating_add(1);
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM currents_staging_chunks WHERE token=?")
            .bind(staged.token.as_bytes().as_slice())
            .fetch_one(pool)
            .await
            .map_err(|_| crate::schema::failure())?;
    if count != index
        || length != staged.size
        || <[u8; 32]>::from(hash.finalize()) != staged.digest.0
    {
        return Err(crate::schema::failure());
    }
    Ok(())
}
