//! Isolated read-only evidence inspection, never a financial command capability.
use currents_cipher::{CipherPool, CipherProfileV1, DatasetKey};
use currents_types::EngineError;
use std::path::Path;
pub struct ForensicStore {
    pool: CipherPool,
    failure: EngineError,
}
impl ForensicStore {
    pub async fn open(
        path: &Path,
        key: DatasetKey,
        profile: CipherProfileV1,
    ) -> Result<Self, EngineError> {
        let pool = currents_cipher::open_pool_read_only(path, key, profile, 4)
            .await
            .map_err(|_| crate::schema::failure())?;
        let result = crate::HistoryReader::new(pool.pool().clone())
            .audit_report()
            .await;
        match result {
            Err(failure) => Ok(Self { pool, failure }),
            Ok(_) => {
                pool.close().await;
                Err(EngineError::invalid())
            }
        }
    }
    pub fn failure(&self) -> &EngineError {
        &self.failure
    }
    pub fn history(&self) -> crate::HistoryReader {
        crate::HistoryReader::new(self.pool.pool().clone())
    }
    pub async fn close(self) {
        self.pool.close().await;
    }
}

/// Existing-source capability for offline verification. It cannot stage content,
/// return a writer, initialize a dataset, or certify full C06 semantics by itself.
pub struct ReadOnlyStore {
    pool: CipherPool,
}
impl ReadOnlyStore {
    pub async fn open(
        path: &Path,
        key: DatasetKey,
        profile: CipherProfileV1,
    ) -> Result<Self, EngineError> {
        let pool = currents_cipher::open_pool_read_only(path, key, profile, 4)
            .await
            .map_err(|_| crate::schema::failure())?;
        Ok(Self { pool })
    }
    pub fn history(&self) -> crate::HistoryReader {
        crate::HistoryReader::new(self.pool.pool().clone())
    }
    /// Preserve a failed full-replay result on the same read-only source handle.
    /// This is a trust-failure label, never a grant of native mutation authority.
    pub fn into_forensic(self, failure: EngineError) -> ForensicStore {
        ForensicStore {
            pool: self.pool,
            failure,
        }
    }
    pub async fn close(self) {
        self.pool.close().await;
    }
}
