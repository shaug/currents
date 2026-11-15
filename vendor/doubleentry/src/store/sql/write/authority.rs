//! Currents-only named authority composition. Never exposes SQL or a transaction.
use super::*;
use crate::store::Sqlite;
use crate::{
    log::{TreeHead, nodes::RootPlan},
    transparency::{Checkpoint, KeyName},
};

/// Private Currents authority transaction with fixed named persistence operations.
pub struct CurrentsAuthorityTransaction<'a> {
    session: Session<'a, Sqlite>,
    checkpoint: String,
    native_failed: bool,
    authority_started: bool,
}
impl SqlStore<Sqlite, 18> {
    /// Open one native-owned transaction; at most one prepared operation in CUR008.
    pub async fn currents_begin_authority(
        &self,
        op: Option<&Op<18>>,
    ) -> Result<CurrentsAuthorityTransaction<'_>, StoreError> {
        let mut session = self.session(Mode::Write, "currents_authority").await?;
        let prepared = async {
            let ledger = self.ledger(&mut session).await?;
            session
                .lock(ledger.key)
                .await
                .map_err(|e| e.unwrap_or_else(|| StoreError::unavailable("currents_authority")))?;
            let size = session.size().await?;
            let plan =
                RootPlan::new(size).map_err(|_| integrity("native root size", Some(size)))?;
            let hashes = nodes(&mut session, plan.ids(), &mut BTreeMap::new()).await?;
            let root = plan
                .assemble(&hashes)
                .map_err(|_| integrity("native root", Some(size)))?;
            let checkpoint =
                Checkpoint::new(KeyName::from(&ledger.id), TreeHead { size, root }).text();
            Ok::<_, StoreError>(checkpoint)
        }
        .await;
        match prepared {
            Ok(checkpoint) => {
                let mut tx = CurrentsAuthorityTransaction {
                    session,
                    checkpoint,
                    native_failed: false,
                    authority_started: false,
                };
                if let Some(op) = op {
                    if let Err(error) = tx.apply_native(op).await {
                        tx.rollback().await.map_err(StoreError::outcome_unknown)?;
                        return Err(error);
                    }
                }
                Ok(tx)
            }
            Err(error) => {
                session
                    .rollback()
                    .await
                    .map_err(StoreError::outcome_unknown)?;
                Err(error)
            }
        }
    }
}
impl CurrentsAuthorityTransaction<'_> {
    /// Apply one native operation to the existing authority transaction.
    /// Every invocation reloads all metadata and facts from preceding uncommitted writes.
    /// No transaction boundary or caller-provided SQL is accepted.
    ///
    /// # Errors
    /// Native rejection or persistence failure requires the owner to roll back.
    pub async fn apply_native(&mut self, op: &Op<18>) -> Result<(), StoreError> {
        if self.native_failed || self.authority_started {
            return Err(StoreError::backend_message(
                "currents_authority",
                "native execution is closed",
            ));
        }
        self.native_failed = true;
        let session = &mut self.session;
        let info = existing::<Sqlite, 18>(session).await?;
        let mut header = header::<Sqlite, 18>(session, info.ledger.clone()).await?;
        let (decision, _, _) = plan_loop::<Sqlite, 18>(session, header.clone(), op).await?;
        let mut known = Known::default();
        match decision {
            Ok(Outcome::Unchanged(done)) if !matches!(done, Done::BatchRejected { .. }) => {}
            Ok(Outcome::Append(write)) => persist(session, &header, &write, &mut known).await?,
            Ok(Outcome::Batch { writes, done }) if !matches!(done, Done::BatchRejected { .. }) => {
                for write in &writes {
                    persist(session, &header, write, &mut known).await?;
                    header.size = header.size.saturating_add(1);
                    if matches!(write.record, Record::Entry(_)) {
                        header.entries = header.entries.saturating_add(1);
                    }
                }
            }
            _ => {
                return Err(StoreError::backend_message(
                    "currents_authority",
                    "native operation rejected",
                ));
            }
        }
        let size = session.size().await?;
        let plan = RootPlan::new(size).map_err(|_| integrity("native root size", Some(size)))?;
        let hashes = nodes(session, plan.ids(), &mut BTreeMap::new()).await?;
        let root = plan
            .assemble(&hashes)
            .map_err(|_| integrity("native root", Some(size)))?;
        self.checkpoint =
            Checkpoint::new(KeyName::from(&info.ledger), TreeHead { size, root }).text();
        self.native_failed = false;
        Ok(())
    }
    /// Find a prior command in this writer transaction before any replay or stale-head check.
    ///
    /// # Errors
    /// Returns a database error without treating absence as proof of settled noncommit.
    pub async fn command(&mut self, command: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        sqlx::query_scalar("SELECT canonical FROM currents_commits WHERE command_id=?")
            .bind(command)
            .fetch_optional(&mut *self.session.tx)
            .await
            .map_err(|e| self.session.fail(e))
    }
    /// Fixed synthetic database-size exhaustion on this transaction's pager.
    /// Not compiled into production. Accepts neither SQL nor caller parameters.
    ///
    /// # Errors
    /// Returns the underlying SQLite fault.
    #[cfg(feature = "testing")]
    pub async fn test_exhaust_pages(&mut self) -> Result<(), StoreError> {
        let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
            .fetch_one(&mut *self.session.tx)
            .await
            .map_err(|e| self.session.fail(e))?;
        sqlx::query(sqlx::AssertSqlSafe(format!("PRAGMA max_page_count={pages}")))
            .execute(&mut *self.session.tx)
            .await
            .map_err(|e| self.session.fail(e))?;
        Ok(())
    }
    /// Checkpoint.
    ///
    /// # Errors
    /// Returns the underlying transaction failure without exposing arbitrary SQL.
    pub fn checkpoint(&self) -> &str {
        &self.checkpoint
    }
    /// Tip.
    ///
    /// # Errors
    /// Returns the underlying transaction failure without exposing arbitrary SQL.
    pub async fn tip(&mut self) -> Result<Option<Vec<u8>>, StoreError> {
        sqlx::query_scalar("SELECT canonical FROM currents_commits ORDER BY seq DESC LIMIT 1")
            .fetch_optional(&mut *self.session.tx)
            .await
            .map_err(|e| self.session.fail(e))
    }
    /// Insert commit.
    ///
    /// # Errors
    /// Returns the underlying transaction failure without exposing arbitrary SQL.
    pub async fn insert_commit(
        &mut self,
        seq: i64,
        previous: &[u8],
        digest: &[u8],
        canonical: &[u8],
        command: &[u8],
    ) -> Result<(), StoreError> {
        self.authority_started = true;
        sqlx::query("INSERT INTO currents_commits(seq,previous_digest,digest,canonical,checkpoint,command_id) VALUES(?,?,?,?,?,?)").bind(seq).bind(previous).bind(digest).bind(canonical).bind(&self.checkpoint).bind(command).execute(&mut *self.session.tx).await.map_err(|e|self.session.fail(e))?;
        Ok(())
    }
    /// Insert item.
    ///
    /// # Errors
    /// Returns the underlying transaction failure without exposing arbitrary SQL.
    pub async fn insert_item(
        &mut self,
        seq: i64,
        position: i64,
        kind: &str,
        digest: &[u8],
        canonical: &[u8],
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO currents_items(seq,position,kind,digest,canonical) VALUES(?,?,?,?,?)",
        )
        .bind(seq)
        .bind(position)
        .bind(kind)
        .bind(digest)
        .bind(canonical)
        .execute(&mut *self.session.tx)
        .await
        .map_err(|e| self.session.fail(e))?;
        Ok(())
    }
    /// Insert definition.
    ///
    /// # Errors
    /// Returns the underlying transaction failure without exposing arbitrary SQL.
    pub async fn insert_definition(
        &mut self,
        kind: &str,
        id: &str,
        version: i64,
        seq: i64,
        position: i64,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO currents_definitions(kind,id,version,seq,position) VALUES(?,?,?,?,?)",
        )
        .bind(kind)
        .bind(id)
        .bind(version)
        .bind(seq)
        .bind(position)
        .execute(&mut *self.session.tx)
        .await
        .map_err(|e| self.session.fail(e))?;
        Ok(())
    }
    /// Definition.
    ///
    /// # Errors
    /// Returns the underlying transaction failure without exposing arbitrary SQL.
    pub async fn definition(
        &mut self,
        kind: &str,
        id: &str,
        version: Option<i64>,
    ) -> Result<Option<i64>, StoreError> {
        sqlx::query_scalar("SELECT version FROM currents_definitions WHERE kind=? AND id=? AND (? IS NULL OR version=?) ORDER BY version LIMIT 1").bind(kind).bind(id).bind(version).bind(version).fetch_optional(&mut *self.session.tx).await.map_err(|e|self.session.fail(e))
    }
    /// Read the canonical dataset declaration inside this transaction.
    ///
    /// # Errors
    /// Returns a database failure without exposing arbitrary SQL.
    pub async fn dataset_record(&mut self, id: &str) -> Result<Option<Vec<u8>>, StoreError> {
        sqlx::query_scalar("SELECT i.canonical FROM currents_definitions d JOIN currents_items i ON i.seq=d.seq AND i.position=d.position WHERE d.kind='Dataset' AND d.id=? AND d.version=0").bind(id).fetch_optional(&mut *self.session.tx).await.map_err(|e|self.session.fail(e))
    }
    /// Insert reference.
    ///
    /// # Errors
    /// Returns the underlying transaction failure without exposing arbitrary SQL.
    pub async fn insert_reference(
        &mut self,
        seq: i64,
        position: i64,
        kind: &str,
        id: &str,
        version: i64,
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO currents_references(seq,position,target_kind,target_id,target_version) VALUES(?,?,?,?,?)").bind(seq).bind(position).bind(kind).bind(id).bind(version).execute(&mut *self.session.tx).await.map_err(|e|self.session.fail(e))?;
        Ok(())
    }
    /// Promote content.
    ///
    /// # Errors
    /// Returns the underlying transaction failure without exposing arbitrary SQL.
    pub async fn promote_content(
        &mut self,
        token: &[u8],
        digest: &[u8],
        size: i64,
        seq: i64,
    ) -> Result<(), StoreError> {
        let valid: Option<i64> = sqlx::query_scalar(
            "SELECT byte_count FROM currents_staging WHERE token=? AND digest=? AND byte_count=?",
        )
        .bind(token)
        .bind(digest)
        .bind(size)
        .fetch_optional(&mut *self.session.tx)
        .await
        .map_err(|e| self.session.fail(e))?;
        if valid.is_none() {
            return Err(integrity("staged content absent", None));
        }
        let inserted=sqlx::query("INSERT INTO currents_blobs(digest,byte_count,first_seq) VALUES(?,?,?) ON CONFLICT(digest) DO NOTHING").bind(digest).bind(size).bind(seq).execute(&mut *self.session.tx).await.map_err(|e|self.session.fail(e))?.rows_affected();
        if inserted > 0 {
            sqlx::query("INSERT INTO currents_chunks(digest,position,bytes,chunk_digest) SELECT ?,position,bytes,chunk_digest FROM currents_staging_chunks WHERE token=? ORDER BY position").bind(digest).bind(token).execute(&mut *self.session.tx).await.map_err(|e|self.session.fail(e))?;
        }
        Ok(())
    }
    /// Content size.
    ///
    /// # Errors
    /// Returns the underlying transaction failure without exposing arbitrary SQL.
    pub async fn content_size(&mut self, digest: &[u8]) -> Result<Option<i64>, StoreError> {
        sqlx::query_scalar("SELECT byte_count FROM currents_blobs WHERE digest=?")
            .bind(digest)
            .fetch_optional(&mut *self.session.tx)
            .await
            .map_err(|e| self.session.fail(e))
    }
    /// Insert content reference.
    ///
    /// # Errors
    /// Returns the underlying transaction failure without exposing arbitrary SQL.
    pub async fn insert_content_reference(
        &mut self,
        seq: i64,
        position: i64,
        digest: &[u8],
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO currents_content_refs(seq,position,digest) VALUES(?,?,?)")
            .bind(seq)
            .bind(position)
            .bind(digest)
            .execute(&mut *self.session.tx)
            .await
            .map_err(|e| self.session.fail(e))?;
        Ok(())
    }
    /// Commit.
    ///
    /// # Errors
    /// Returns the underlying transaction failure without exposing arbitrary SQL.
    pub async fn commit(self) -> Result<(), StoreError> {
        if self.native_failed {
            self.session
                .rollback()
                .await
                .map_err(StoreError::outcome_unknown)?;
            return Err(StoreError::backend_message(
                "currents_authority",
                "native operation rejected",
            ));
        }
        self.session
            .commit()
            .await
            .map_err(StoreError::outcome_unknown)
    }
    /// Rollback.
    ///
    /// # Errors
    /// Returns the underlying transaction failure without exposing arbitrary SQL.
    pub async fn rollback(self) -> Result<(), StoreError> {
        self.session
            .rollback()
            .await
            .map_err(StoreError::outcome_unknown)
    }
}
