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
    pub evidence: Vec<EvidenceContentV1>,
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
    /// Read-only retained history from this exact authority writer.
    pub fn retained_history(&self) -> crate::HistoryReader {
        crate::HistoryReader::new(self.pool.clone())
    }

    /// C05 pre-write binding for a C08 plan re-admitted by the service.
    /// Freshly freeze trusted intent; a well-formed but different batch cannot
    /// substitute another native interpretation of the same accepted prefix.
    pub fn verify_correction_freeze(
        prior: &DomainState,
        plan: &DecisionPlan,
        supplied: &AccountingBatchV1,
    ) -> Result<(), EngineError> {
        if !matches!(&plan.accounting, AccountingIntent::Correction(_))
            || &currents_accounting::AccountingAdapter::freeze(&plan.accounting, prior)? != supplied
        {
            return Err(EngineError::invalid());
        }
        Ok(())
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
        // This private path accepts only exactly referenced, bounded content.
        let mut required = std::collections::BTreeMap::new();
        for record in &prepared.decision_plan.records {
            for content in crate::references::describe(record)
                .map_err(CommitFailure::DefinitelyRolledBack)?
                .contents
            {
                required.entry(content.digest).or_insert(content.size);
            }
        }
        if !prepared.evidence.is_empty() {
            use sha2::{Digest as _, Sha256};
            let mut supplied = std::collections::BTreeSet::new();
            for content in &prepared.evidence {
                if content.bytes.len() > crate::staging::MAX_FILE
                    || Digest(Sha256::digest(&content.bytes).into()) != content.digest
                    || !supplied.insert(content.digest)
                    || !required.contains_key(&content.digest)
                    || required[&content.digest]
                        .is_some_and(|n| n < 0 || n as usize != content.bytes.len())
                {
                    return Err(CommitFailure::DefinitelyRolledBack(EngineError::invalid()));
                }
            }
            if supplied.len() != required.len() {
                return Err(CommitFailure::DefinitelyRolledBack(EngineError::invalid()));
            }
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
        let mut staged = Vec::new();
        let mut tokens = Vec::new();
        for content in &prepared.evidence {
            if let Err(error) = prepared.control.check_cancelled() {
                crate::staging::discard(&self.pool, &tokens)
                    .await
                    .map_err(CommitFailure::DefinitelyRolledBack)?;
                return Err(CommitFailure::DefinitelyRolledBack(error));
            }
            match crate::staging::stage(&self.pool, &content.bytes).await {
                Ok(handle) => {
                    tokens.push(handle.token);
                    staged.push(handle);
                }
                Err(error) => {
                    crate::staging::discard(&self.pool, &tokens)
                        .await
                        .map_err(CommitFailure::DefinitelyRolledBack)?;
                    return Err(CommitFailure::DefinitelyRolledBack(error));
                }
            }
        }
        let command_id = prepared.result.command_id;
        let outcome = self
            .append_closure(
                crate::HistoryAppend {
                    prior_head: prepared.prior_head,
                    records,
                    decisions: prepared.decisions,
                    result: prepared.result,
                    staged,
                    native: None,
                },
                batch,
                &prepared.control,
            )
            .await;
        // Published immutable blobs are distinct from temporary staging tokens.
        // Failure to clean after a submitted commit must never imply rollback.
        if crate::staging::discard(&self.pool, &tokens).await.is_err() {
            return Err(CommitFailure::OutcomeUnknown(command_id));
        }
        let commit = outcome?;
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
