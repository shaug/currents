//! Exact schema-object attestation, including native indexes and triggers.
use currents_types::{EngineError, ErrorClass, SafeNextAction};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

// Generated from the reviewed native/Currents schema, not from the opened file.
const EXPECTED_SCHEMA_SHA256: &str =
    "eb9cd2fbe4465a038147c959862cbda4c1728f74f94bd7c611bb2c878103a40f";

pub struct SchemaAttestation {
    pub schema_sha256: String,
    pub migration_sha256: String,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn failure() -> EngineError {
    EngineError {
        class: ErrorClass::IntegrityFailure,
        affected_scope: vec![],
        next_action: SafeNextAction::Recover,
    }
}
pub(crate) async fn attest(pool: &SqlitePool) -> Result<SchemaAttestation, EngineError> {
    let rows = sqlx::query("SELECT type, name, tbl_name, coalesce(sql,'') AS definition FROM sqlite_schema ORDER BY type, name")
        .fetch_all(pool).await.map_err(|_| failure())?;
    let mut schema = Sha256::new();
    for row in rows {
        for field in ["type", "name", "tbl_name", "definition"] {
            let value: String = row.try_get(field).map_err(|_| failure())?;
            schema.update((value.len() as u64).to_be_bytes());
            schema.update(value.as_bytes());
        }
    }
    let schema_sha256 = hex(&schema.finalize());
    if schema_sha256 != EXPECTED_SCHEMA_SHA256 {
        return Err(failure());
    }
    let mut migration = Sha256::new();
    for source in [
        include_bytes!("../../../vendor/doubleentry/src/store/sql/schema.sql").as_slice(),
        include_bytes!("../../../vendor/doubleentry/src/store/sql/schema.rs").as_slice(),
        include_bytes!("../../../vendor/doubleentry/src/store/sql/sqlite.rs").as_slice(),
        include_bytes!("../../../vendor/doubleentry/src/store/sql/triggers_sqlite.sql").as_slice(),
        include_bytes!("../../../vendor/doubleentry/src/store/sql/no_replace_sqlite.sql")
            .as_slice(),
        include_bytes!("../../../vendor/doubleentry/src/store/sql/write/currents.rs").as_slice(),
    ] {
        migration.update((source.len() as u64).to_be_bytes());
        migration.update(source);
    }
    Ok(SchemaAttestation {
        schema_sha256,
        migration_sha256: hex(&migration.finalize()),
    })
}
