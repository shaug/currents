//! C04 trusted storage construction. Host key errors never enter a file opener.
use crate::{
    Writer,
    schema::{self, SchemaAttestation},
};
use currents_accounting::AtomicStore;
use currents_cipher::{CipherPool, CipherProfileV1, DatasetKey, open_pool};
use currents_types::EngineError;
use std::path::Path;

pub struct EncryptedStore {
    pool: CipherPool,
    writer: Writer,
    attestation: SchemaAttestation,
}
impl EncryptedStore {
    pub async fn open(
        path: &Path,
        key: DatasetKey,
        profile: CipherProfileV1,
    ) -> Result<Self, EngineError> {
        let pool = open_pool(path, key, profile, 4)
            .await
            .map_err(|_| schema::failure())?;
        let initialize = async {
            let objects: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_schema")
                .fetch_one(pool.pool())
                .await
                .map_err(|_| schema::failure())?;
            // Do not repair or add schema objects to an existing dataset before
            // checking it. The native foreign-schema guard still owns creation.
            if objects > 0 {
                schema::attest(pool.pool()).await?;
            }
            let accounting =
                AtomicStore::from_initialized_pool(pool.pool().clone(), objects == 0).await?;
            let attestation = schema::attest(pool.pool()).await?;
            Ok::<_, EngineError>((Writer::new(accounting), attestation))
        }
        .await;
        match initialize {
            Ok((writer, attestation)) => Ok(Self {
                pool,
                writer,
                attestation,
            }),
            Err(error) => {
                pool.close().await;
                Err(error)
            }
        }
    }
    /// The host may supply its Keychain result without a plaintext fallback.
    pub async fn open_with_key_result(
        path: &Path,
        key: Result<DatasetKey, EngineError>,
        profile: CipherProfileV1,
    ) -> Result<Self, EngineError> {
        Self::open(path, key?, profile).await
    }
    pub fn writer(&self) -> &Writer {
        &self.writer
    }
    pub fn attestation(&self) -> &SchemaAttestation {
        &self.attestation
    }
    pub async fn close(self) -> Result<(), EngineError> {
        self.pool.close().await;
        Ok(())
    }
}
