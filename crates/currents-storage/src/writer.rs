//! Private C04 ordered authority writer.
//! Full PreparedCommit/frozen_batch construction remains with C05/C03.
use currents_accounting::AtomicStore;
use currents_types::{CommandId, EngineError};

/// Commit uncertainty is not proof of rollback. Resolve it only by reopening/auditing.
#[derive(Debug)]
pub enum CommitFailure {
    DefinitelyRolledBack(EngineError),
    OutcomeUnknown(CommandId),
}

/// Private host/storage component. No public raw native handle, SQL or callback.
/// CUR-006 owns encrypted construction; this type exports no plaintext opener.
pub struct Writer {
    pub(crate) accounting: AtomicStore,
    pub(crate) pool: sqlx::SqlitePool,
}
impl Writer {
    pub(crate) fn new(accounting: AtomicStore, pool: sqlx::SqlitePool) -> Self {
        Self { accounting, pool }
    }
}
