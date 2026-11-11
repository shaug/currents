//! Narrow C04 authority transaction; C05 operation freezing is not implemented here.
use crate::{AtomicFailure, AtomicStore, PreparedNativeOperation};
use currents_types::{CommandId, Digest, EngineError, NativeHead};
use doubleentry::{
    LedgerId,
    store::CurrentsAuthorityTransaction,
    transparency::{Checkpoint, KeyName},
};
use sha2::{Digest as _, Sha256};

/// Checkpoint codec v1 binds the pinned native unsigned checkpoint text exactly.
pub fn verify_native_checkpoint(head: &NativeHead, text: &str) -> Result<(), EngineError> {
    if head.semantics_version != 1 {
        return Err(EngineError::version());
    }
    let checkpoint = Checkpoint::parse_text(text).map_err(|_| EngineError::invalid())?;
    let id = LedgerId::new("currents/dataset-v1").map_err(|_| EngineError::invalid())?;
    if head.semantics_version != 1
        || checkpoint.origin() != &KeyName::from(&id)
        || !checkpoint.extensions().is_empty()
        || checkpoint.text() != text
        || head.checkpoint.0 != <[u8; 32]>::from(Sha256::digest(text.as_bytes()))
    {
        return Err(EngineError::invalid());
    }
    Ok(())
}
/// Private trusted storage component. It exposes neither native commands nor SQL.
pub struct AuthorityTransaction<'a> {
    inner: CurrentsAuthorityTransaction<'a>,
    command: CommandId,
}
impl AtomicStore {
    pub async fn begin_authority(
        &self,
        command: CommandId,
        operation: Option<&PreparedNativeOperation>,
    ) -> Result<AuthorityTransaction<'_>, AtomicFailure> {
        self.store
            .currents_begin_authority(operation.map(|v| &v.operation))
            .await
            .map(|inner| AuthorityTransaction { inner, command })
            .map_err(|e| {
                if e.may_have_applied() {
                    AtomicFailure::Unknown(command)
                } else {
                    AtomicFailure::RolledBack(EngineError::invalid())
                }
            })
    }
}
impl AuthorityTransaction<'_> {
    pub async fn command(&mut self, command: CommandId) -> Result<Option<Vec<u8>>, EngineError> {
        self.inner
            .command(command.0.as_bytes())
            .await
            .map_err(|_| EngineError::invalid())
    }
    pub async fn apply_native(
        &mut self,
        operation: &PreparedNativeOperation,
    ) -> Result<(), EngineError> {
        self.inner
            .apply_native(&operation.operation)
            .await
            .map_err(|_| EngineError::invalid())
    }
    pub async fn apply_closure(
        &mut self,
        batch: &crate::PreparedNativeOperations,
        control: &crate::ClosureControl,
    ) -> Result<(), EngineError> {
        batch.validate_limits()?;
        for operation in &batch.operations {
            control.check_cancelled()?;
            self.inner
                .apply_native(&operation.operation)
                .await
                .map_err(|error| {
                    #[cfg(feature = "synthetic-test-fixtures")]
                    {
                        use std::error::Error;
                        if error
                            .source()
                            .and_then(|source| source.downcast_ref::<sqlx::Error>())
                            .and_then(sqlx::Error::as_database_error)
                            .and_then(|error| error.code())
                            .is_some_and(|code| code == "13")
                        {
                            control
                                .observed_full
                                .store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    let _ = error;
                    EngineError::invalid()
                })?;
            control.advance(match &operation.operation {
                doubleentry::store::Op::Batch(items) => items.len(),
                _ => 1,
            });
            #[cfg(feature = "synthetic-test-fixtures")]
            {
                let fault = control.test_fault.load(std::sync::atomic::Ordering::SeqCst);
                let count = control.completed_operations();
                if fault == 1 && count == 600 {
                    self.inner
                        .test_exhaust_pages()
                        .await
                        .map_err(|_| EngineError::invalid())?;
                }
                if (fault == 2 && count == 600) || (fault == 3 && count == 1200) {
                    control.test_pause().await;
                }
            }
        }
        control.check_cancelled()
    }
    pub fn native_head(&self) -> NativeHead {
        NativeHead {
            semantics_version: 1,
            checkpoint: Digest(Sha256::digest(self.inner.checkpoint().as_bytes()).into()),
        }
    }
    pub fn checkpoint(&self) -> &str {
        self.inner.checkpoint()
    }
    pub async fn tip(&mut self) -> Result<Option<Vec<u8>>, EngineError> {
        self.inner.tip().await.map_err(|_| EngineError::invalid())
    }
    pub async fn insert_commit(
        &mut self,
        seq: i64,
        previous: &[u8],
        digest: &[u8],
        canonical: &[u8],
        command: &[u8],
    ) -> Result<(), EngineError> {
        self.inner
            .insert_commit(seq, previous, digest, canonical, command)
            .await
            .map_err(|_| EngineError::invalid())
    }
    pub async fn insert_item(
        &mut self,
        seq: i64,
        position: i64,
        kind: &str,
        digest: &[u8],
        canonical: &[u8],
    ) -> Result<(), EngineError> {
        self.inner
            .insert_item(seq, position, kind, digest, canonical)
            .await
            .map_err(|_| EngineError::invalid())
    }
    pub async fn insert_definition(
        &mut self,
        kind: &str,
        id: &str,
        version: i64,
        seq: i64,
        position: i64,
    ) -> Result<(), EngineError> {
        self.inner
            .insert_definition(kind, id, version, seq, position)
            .await
            .map_err(|_| EngineError::invalid())
    }
    pub async fn definition(
        &mut self,
        kind: &str,
        id: &str,
        version: Option<i64>,
    ) -> Result<Option<i64>, EngineError> {
        self.inner
            .definition(kind, id, version)
            .await
            .map_err(|_| EngineError::invalid())
    }
    pub async fn dataset_record(&mut self, id: &str) -> Result<Option<Vec<u8>>, EngineError> {
        self.inner
            .dataset_record(id)
            .await
            .map_err(|_| EngineError::invalid())
    }
    pub async fn insert_reference(
        &mut self,
        seq: i64,
        position: i64,
        kind: &str,
        id: &str,
        version: i64,
    ) -> Result<(), EngineError> {
        self.inner
            .insert_reference(seq, position, kind, id, version)
            .await
            .map_err(|_| EngineError::invalid())
    }
    pub async fn promote_content(
        &mut self,
        token: &[u8],
        digest: &[u8],
        size: i64,
        seq: i64,
    ) -> Result<(), EngineError> {
        self.inner
            .promote_content(token, digest, size, seq)
            .await
            .map_err(|_| EngineError::invalid())
    }
    pub async fn content_size(&mut self, digest: &[u8]) -> Result<Option<i64>, EngineError> {
        self.inner
            .content_size(digest)
            .await
            .map_err(|_| EngineError::invalid())
    }
    pub async fn insert_content_reference(
        &mut self,
        seq: i64,
        position: i64,
        digest: &[u8],
    ) -> Result<(), EngineError> {
        self.inner
            .insert_content_reference(seq, position, digest)
            .await
            .map_err(|_| EngineError::invalid())
    }
    pub async fn commit_controlled(
        self,
        control: &crate::ClosureControl,
    ) -> Result<(), AtomicFailure> {
        #[cfg(feature = "synthetic-test-fixtures")]
        if control.test_fault.load(std::sync::atomic::Ordering::SeqCst) == 4 {
            control.test_pause().await;
        }
        let command = self.command;
        self.commit().await?;
        #[cfg(feature = "synthetic-test-fixtures")]
        if control.test_fault.load(std::sync::atomic::Ordering::SeqCst) == 4 {
            return Err(AtomicFailure::Unknown(command));
        }
        let _ = (control, command);
        Ok(())
    }
    pub async fn commit(self) -> Result<(), AtomicFailure> {
        self.inner
            .commit()
            .await
            .map_err(|_| AtomicFailure::Unknown(self.command))
    }
    pub async fn rollback(self) -> Result<(), AtomicFailure> {
        self.inner
            .rollback()
            .await
            .map_err(|_| AtomicFailure::Unknown(self.command))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use currents_cipher::{CipherProfileV1, DatasetKey, open_pool};
    use doubleentry::{AccountKind, Amount, Entry, Ledger, currency::USD, date, store::Op};
    #[tokio::test]
    async fn native_head_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_pool(
            &dir.path().join("synthetic.db"),
            DatasetKey::new([0x47; 32]),
            CipherProfileV1,
            4,
        )
        .await
        .unwrap();
        let adapter = AtomicStore::from_initialized_pool(pool.pool().clone(), true)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(
            "../../currents-storage/migrations/0001_authority.sql"
        ))
        .execute(pool.pool())
        .await
        .unwrap();
        let books = Ledger::<_, 18>::with_precision(
            adapter.store.clone(),
            LedgerId::new("currents/dataset-v1").unwrap(),
        )
        .await
        .unwrap();
        let command = CommandId("00000000-0000-0000-0000-000000000001".parse().unwrap());
        let genesis = adapter.begin_authority(command, None).await.unwrap();
        let empty = genesis.checkpoint().to_owned();
        verify_native_checkpoint(&genesis.native_head(), &empty).unwrap();
        genesis.rollback().await.unwrap();
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
        let before = books.head().await.unwrap();
        let operation = PreparedNativeOperation {
            operation: Op::Record(Box::new(
                Entry::new("synthetic.purchase", date!("2026-01-01"))
                    .debit(
                        "Expenses:Purchase",
                        "96.00".parse::<Amount<18>>().unwrap(),
                        USD,
                    )
                    .credit(
                        "Assets:Checking",
                        "96.00".parse::<Amount<18>>().unwrap(),
                        USD,
                    ),
            )),
        };
        let mut tx = adapter
            .begin_authority(command, Some(&operation))
            .await
            .unwrap();
        let text = tx.checkpoint().to_owned();
        let head = tx.native_head();
        assert_ne!(Checkpoint::parse_text(&text).unwrap().head(), before);
        assert_eq!(books.head().await.unwrap(), before);
        // The named publication and money share the same native-owned transaction.
        tx.insert_commit(
            1,
            &[0; 32],
            &[1; 32],
            b"synthetic authority",
            command.0.as_bytes(),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            books.head().await.unwrap(),
            Checkpoint::parse_text(&text).unwrap().head()
        );
        let no_effect = adapter.begin_authority(command, None).await.unwrap();
        assert_eq!(no_effect.native_head(), head);
        no_effect.rollback().await.unwrap();
        let saved: String =
            sqlx::query_scalar("SELECT checkpoint FROM currents_commits WHERE seq=1")
                .fetch_one(pool.pool())
                .await
                .unwrap();
        assert_eq!(saved, text);
        verify_native_checkpoint(&head, &saved).unwrap();
        let mut unsupported = head.clone();
        unsupported.semantics_version = 2;
        assert!(verify_native_checkpoint(&unsupported, &saved).is_err());
        let golden = serde_json::json!({"empty":empty,"monetary":text,"digest":head.checkpoint});
        assert_eq!(
            golden,
            serde_json::from_str::<serde_json::Value>(include_str!(
                "../../../fixtures/contracts/v1/native-checkpoint.json"
            ))
            .unwrap()
        );
        books.audit().await.unwrap();
        pool.close().await;
    }
}
