//! Private C04 ordered authority writer.
//! Full PreparedCommit/frozen_batch construction remains with C05/C03.
use currents_accounting::AtomicStore;
pub use currents_accounting::ClosureControl as CommitControl;
use currents_types::*;
use std::sync::Arc;

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

/// Private service-to-storage input. Stable command identity is in `result`;
/// live authorization is deliberately absent. C05 owns the frozen instructions.
pub struct PreparedCommit {
    pub decision_plan: DecisionPlan,
    pub frozen_batch: AccountingBatchV1,
    pub prior_head: CommitSeq,
    pub result: CommandResultV1,
    pub decisions: Vec<DecisionEnvelopeV1>,
    pub manifest_id: RecordId,
    pub control: Arc<CommitControl>,
}
pub struct CommitReceipt {
    pub commit: CommitSeq,
    pub native_head: NativeHead,
    pub domain_digest: Digest,
    pub result: CommandResultV1,
}
impl Writer {
    pub(crate) fn new(accounting: AtomicStore, pool: sqlx::SqlitePool) -> Self {
        Self { accounting, pool }
    }
    pub async fn commit(&self, prepared: PreparedCommit) -> Result<CommitReceipt, CommitFailure> {
        if prepared.decision_plan.invalidations != prepared.result.effects.invalidations
            || prepared.frozen_batch.preconditions.through != prepared.prior_head
            || prepared
                .decision_plan
                .records
                .iter()
                .any(|r| matches!(r, AuthoritativeRecordV1::FrozenAccountingManifest(_)))
        {
            return Err(CommitFailure::DefinitelyRolledBack(EngineError::invalid()));
        }
        let mut records = prepared.decision_plan.records;
        records.push(AuthoritativeRecordV1::FrozenAccountingManifest(
            currents_accounting::AccountingAdapter::manifest(
                prepared.manifest_id,
                &prepared.frozen_batch,
            )
            .map_err(CommitFailure::DefinitelyRolledBack)?,
        ));
        let batch = currents_accounting::AccountingAdapter::prepare_closure(&prepared.frozen_batch)
            .map_err(CommitFailure::DefinitelyRolledBack)?;
        let commit = self
            .append_closure(
                crate::HistoryAppend {
                    prior_head: prepared.prior_head,
                    records,
                    decisions: prepared.decisions,
                    result: prepared.result,
                    staged: vec![],
                    native: None,
                },
                batch,
                &prepared.control,
            )
            .await?;
        Ok(CommitReceipt {
            commit: commit.seq,
            native_head: commit.native_head_after,
            domain_digest: commit.digest,
            result: commit.command_result,
        })
    }
    /// Complete structural authority audit followed by a locked exact-tip/native
    /// comparison. This is not C06 financial replay or a production-ready claim.
    pub async fn audited_prefix(&self) -> Result<AccountingPrefixV1, EngineError> {
        let audited = crate::HistoryReader::new(self.pool.clone())
            .audited_tip()
            .await?;
        let mut tx = self
            .accounting
            .begin_authority(CommandId(uuid::Uuid::nil()), None)
            .await
            .map_err(|_| crate::schema::failure())?;
        let result = async {
            let tip = tx
                .tip()
                .await?
                .map(|b| {
                    serde_json::from_slice::<DomainCommitV1>(&b)
                        .map_err(|_| crate::schema::failure())
                })
                .transpose()?;
            if tip != audited
                || tip
                    .as_ref()
                    .is_some_and(|c| c.native_head_after != tx.native_head())
            {
                return Err(crate::schema::failure());
            }
            Ok(AccountingPrefixV1 {
                through: tip.as_ref().map_or(CommitSeq(0), |c| c.seq),
                domain_digest: tip.as_ref().map_or(Digest::default(), |c| c.digest),
                native_head: tx.native_head(),
            })
        }
        .await;
        tx.rollback().await.map_err(|_| crate::schema::failure())?;
        result
    }
}
