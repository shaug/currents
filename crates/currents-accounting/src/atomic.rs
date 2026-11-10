//! Private native composition adapter. CUR-009 supplies frozen operation construction.
use currents_types::{AuthoritativeRecordV1, CommandId, EngineError, ErrorClass, SafeNextAction};
use doubleentry::store::{Done, Op, SqlStore, Sqlite};

/// Opaque prepared native input; no public raw native constructor or import decoder.
/// Frozen C05 construction belongs to CUR-009, not this storage seam.
pub struct PreparedNativeOperation {
    operation: Op<18>,
}

/// Native result remains in its upstream type, behind the private adapter boundary.
pub struct AtomicReceipt {
    _native: Done<18>,
}

/// The one native store used by the private writer. Encrypted construction belongs
/// to CUR-006; intentionally no plaintext application opener is exported here.
pub struct AtomicStore {
    store: SqlStore<Sqlite, 18>,
}

/// Sanitized failure classification, independent of SQL/native diagnostic strings.
#[derive(Debug)]
pub enum AtomicFailure {
    RolledBack(EngineError),
    Unknown(CommandId),
}
impl AtomicStore {
    /// Trusted storage construction only. Receives an already keyed private pool;
    /// no URL, key, SQL, native handle or callback is exposed to clients.
    pub async fn from_initialized_pool(
        pool: sqlx::SqlitePool,
        new_dataset: bool,
    ) -> Result<Self, EngineError> {
        let store = SqlStore::<Sqlite, 18>::currents_from_initialized_pool(pool);
        let initialize = async {
            let id = doubleentry::LedgerId::new("currents/dataset-v1")
                .map_err(|_| EngineError::invalid())?;
            let _ledger = doubleentry::Ledger::<_, 18>::with_precision(store.clone(), id)
                .await
                .map_err(|_| EngineError::invalid())?;
            if new_dataset {
                store
                    .currents_initialize()
                    .await
                    .map_err(|_| EngineError::invalid())?;
            }
            Ok::<(), EngineError>(())
        }
        .await;
        if let Err(error) = initialize {
            store.close().await;
            return Err(error);
        }
        Ok(Self { store })
    }

    /// Append a typed record and one native operation atomically. Not the aggregate
    /// C04 commit API: ordering, frozen batches and full history remain their owners' work.
    pub async fn write_record(
        &self,
        command_id: CommandId,
        record: &AuthoritativeRecordV1,
        prepared: PreparedNativeOperation,
    ) -> Result<AtomicReceipt, AtomicFailure> {
        let bytes = serde_json_canonicalizer::to_vec(record)
            .map_err(|_| AtomicFailure::RolledBack(EngineError::invalid()))?;
        if bytes.len() > 1_048_576 {
            return Err(AtomicFailure::RolledBack(EngineError::invalid()));
        }
        match self
            .store
            .currents_write(&prepared.operation, command_id.0.as_bytes(), &bytes)
            .await
        {
            Ok(Ok(done)) => {
                if matches!(done, Done::BatchRejected { .. }) {
                    return Err(AtomicFailure::RolledBack(EngineError::invalid()));
                }
                Ok(AtomicReceipt { _native: done })
            }
            Ok(Err(_)) => Err(AtomicFailure::RolledBack(EngineError::invalid())),
            Err(error) if error.may_have_applied() => Err(AtomicFailure::Unknown(command_id)),
            Err(_) => Err(AtomicFailure::RolledBack(EngineError {
                class: ErrorClass::StorageFailure,
                affected_scope: vec![],
                next_action: SafeNextAction::Recover,
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doubleentry::{Ledger, LedgerId, currency::USD, date};

    #[tokio::test]
    async fn typed_record_composes_and_retry_preserves_native_head() {
        let directory = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}",
            directory.path().join("synthetic.db").display()
        );
        let store = SqlStore::<Sqlite, 18>::sqlite(&url).await.unwrap();
        let ledger = Ledger::<_, 18>::with_precision(
            store.clone(),
            LedgerId::new("synthetic.currents/typed-record").unwrap(),
        )
        .await
        .unwrap();
        store.currents_initialize().await.unwrap();
        let adapter = AtomicStore { store };
        let records: Vec<AuthoritativeRecordV1> =
            serde_json::from_str(include_str!("../../../fixtures/contracts/v1/records.json"))
                .unwrap();
        let record = records.first().unwrap();
        let command = CommandId("00000000-0000-0000-0000-000000000001".parse().unwrap());
        let operation = || PreparedNativeOperation {
            operation: Op::Currency {
                code: USD,
                exponent: 2,
                effective: date!("0001-01-01"),
                actor: "synthetic".into(),
            },
        };
        assert!(
            adapter
                .write_record(command, record, operation())
                .await
                .is_ok()
        );
        let first = ledger.head().await.unwrap();
        assert!(
            adapter
                .write_record(command, record, operation())
                .await
                .is_ok()
        );
        assert_eq!(ledger.head().await.unwrap(), first);
        let different = records.get(1).unwrap();
        assert!(matches!(
            adapter.write_record(command, different, operation()).await,
            Err(AtomicFailure::RolledBack(_))
        ));
        assert_eq!(ledger.head().await.unwrap(), first);
        ledger.audit().await.unwrap();
        adapter.store.close().await;
    }
}
