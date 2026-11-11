//! Currents-only named record composition. No callback or caller-provided SQL.
use super::*;
use crate::store::Sqlite;

impl SqlStore<Sqlite, 18> {
    /// Currents-only trusted pool seam. The caller must initialize the codec,
    /// key and connection profile on every connection before supplying this pool.
    /// This does not read/create schema or bypass native create/open validation.
    /// The only application caller is the private accounting adapter.
    pub fn currents_from_initialized_pool(pool: sqlx::SqlitePool) -> Self {
        let retries = pool.options().get_max_connections();
        Self {
            inner: std::sync::Arc::new(super::super::Inner {
                pool,
                sql: super::super::Catalogue::new(String::new()),
                schema: super::super::SqlOptions::default().schema,
                ledger: std::sync::OnceLock::new(),
                retries,
            }),
        }
    }

    /// Initialize the private composition table after native metadata is established.
    /// This is not the Currents authoritative history schema or an encrypted opener.
    ///
    /// # Errors
    /// Refuses uninitialized or mismatched native metadata before any Currents DDL.
    pub async fn currents_initialize(&self) -> Result<(), StoreError> {
        let mut session = self.session(Mode::Write, "currents_initialize").await?;
        existing::<Sqlite, 18>(&mut session).await?;
        session.script(String::from(
            "CREATE TABLE IF NOT EXISTS currents_atomic_records (command_id BLOB PRIMARY KEY CHECK(length(command_id)=16), record BLOB NOT NULL CHECK(length(record)>0 AND length(record)<=1048576)) STRICT;"
        )).await?;
        session.commit().await
    }

    /// Compose one validated native operation with one already-canonical Currents record.
    /// Only the private accounting adapter may call this maintained seam. It grants no
    /// client/import authority and accepts neither SQL nor executable callbacks.
    /// Native validation, leaf encoding and result types are unchanged.
    ///
    /// # Errors
    /// Precommit errors roll back both writes. A COMMIT error remains outcome-unknown.
    pub async fn currents_write(
        &self,
        op: &Op<18>,
        command_id: &[u8; 16],
        record: &[u8],
    ) -> Result<Result<Done<18>, Rejection>, StoreError> {
        self.currents_write_inner(op, command_id, record, CommitMode::Normal)
            .await
    }

    async fn currents_write_inner(
        &self,
        op: &Op<18>,
        command_id: &[u8; 16],
        record: &[u8],
        mode: CommitMode,
    ) -> Result<Result<Done<18>, Rejection>, StoreError> {
        let mut session = self.session(Mode::Write, "currents_write").await?;
        let result = self
            .currents_prepare(&mut session, op, command_id, record)
            .await;
        match result {
            Ok(Ok(done)) if !matches!(done, Done::BatchRejected { .. }) => {
                commit(session, mode).await?;
                Ok(Ok(done))
            }
            other => {
                // Await rollback before asserting definite failure. A rollback error
                // is conservatively unknown, never a false definite-noncommit claim.
                session
                    .rollback()
                    .await
                    .map_err(StoreError::outcome_unknown)?;
                other
            }
        }
    }

    async fn currents_prepare(
        &self,
        session: &mut Session<'_, Sqlite>,
        op: &Op<18>,
        command_id: &[u8; 16],
        record: &[u8],
    ) -> Result<Result<Done<18>, Rejection>, StoreError> {
        let ledger = self.ledger(session).await?;
        session
            .lock(ledger.key)
            .await
            .map_err(|e| e.unwrap_or_else(|| StoreError::unavailable("currents_write")))?;
        let mut header = header::<Sqlite, 18>(session, ledger.id).await?;
        let initial_size = header.size;
        let (decision, _, _) = plan_loop::<Sqlite, 18>(session, header.clone(), op).await?;
        let mut known = Known::default();
        let done = match decision {
            Err(rejection) => return Ok(Err(rejection)),
            Ok(Outcome::Unchanged(done)) => done,
            Ok(Outcome::Append(write)) => {
                persist(session, &header, &write, &mut known).await?;
                write.done
            }
            Ok(Outcome::Batch { writes, done }) => {
                for write in &writes {
                    persist(session, &header, write, &mut known).await?;
                    header.size = header.size.saturating_add(1);
                    if matches!(write.record, Record::Entry(_)) {
                        header.entries = header.entries.saturating_add(1);
                    }
                }
                done
            }
        };
        if matches!(done, Done::BatchRejected { .. }) {
            return Ok(Ok(done));
        }
        let previous: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT record FROM currents_atomic_records WHERE command_id = ?")
                .bind(command_id.as_slice())
                .fetch_optional(&mut *session.tx)
                .await
                .map_err(|e| session.fail(e))?;
        if let Some(previous) = previous {
            if previous != record || session.size().await? != initial_size {
                return Err(StoreError::backend_message(
                    "currents_write",
                    "command identity conflict",
                ));
            }
        } else {
            sqlx::query("INSERT INTO currents_atomic_records(command_id, record) VALUES (?, ?)")
                .bind(command_id.as_slice())
                .bind(record)
                .execute(&mut *session.tx)
                .await
                .map_err(|e| session.fail(e))?;
        }
        Ok(Ok(done))
    }
}

#[derive(Clone, Copy)]
enum CommitMode {
    Normal,
    #[cfg(test)]
    IoBefore,
    #[cfg(test)]
    IoAfter,
}
async fn commit(session: Session<'_, Sqlite>, mode: CommitMode) -> Result<(), StoreError> {
    #[cfg(test)]
    if matches!(mode, CommitMode::IoBefore) {
        // COMMIT transport cannot establish whether the database applied it.
        drop(session);
        return Err(super::super::failure::<Sqlite>(
            "currents_write",
            &sqlx::Error::Io(std::io::Error::other("synthetic COMMIT I/O")),
            true,
        ));
    }
    session
        .commit()
        .await
        .map_err(StoreError::outcome_unknown)?;
    #[cfg(test)]
    if matches!(mode, CommitMode::IoAfter) {
        return Err(super::super::failure::<Sqlite>(
            "currents_write",
            &sqlx::Error::Io(std::io::Error::other(
                "synthetic COMMIT acknowledgement I/O",
            )),
            true,
        ));
    }
    let _ = mode;
    Ok(())
}

#[cfg(test)]
mod tests;
