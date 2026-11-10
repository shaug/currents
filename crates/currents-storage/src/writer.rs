//! Staged C04 boundary: named single-record composition only.
//! Full PreparedCommit/frozen_batch construction is intentionally deferred to C05.
use currents_accounting::{AtomicFailure, AtomicReceipt, AtomicStore, PreparedNativeOperation};
use currents_types::{AuthoritativeRecordV1, CommandId, EngineError};

/// Commit uncertainty is not proof of rollback. Resolve it only by reopening/auditing.
#[derive(Debug)]
pub enum CommitFailure {
    DefinitelyRolledBack(EngineError),
    OutcomeUnknown(CommandId),
}

/// Private host/storage component. No public raw native handle, SQL or callback.
/// CUR-006 owns encrypted construction; this type exports no plaintext opener.
pub struct Writer {
    accounting: AtomicStore,
}
impl Writer {
    pub(crate) fn new(accounting: AtomicStore) -> Self {
        Self { accounting }
    }
    /// The single-record seam, not the future aggregate `commit(PreparedCommit)` API.
    pub async fn write_record(
        &self,
        command_id: CommandId,
        record: &AuthoritativeRecordV1,
        prepared: PreparedNativeOperation,
    ) -> Result<AtomicReceipt, CommitFailure> {
        self.accounting
            .write_record(command_id, record, prepared)
            .await
            .map_err(|error| match error {
                AtomicFailure::RolledBack(error) => CommitFailure::DefinitelyRolledBack(error),
                AtomicFailure::Unknown(command_id) => CommitFailure::OutcomeUnknown(command_id),
            })
    }
}
