//! Storage-only frozen replay I/O. No policy evaluation or service dependency.
use crate::{EncryptedStore, HistoryAppend, HistoryReader};
use currents_accounting::{AccountingAdapter, ClosureControl};
use currents_types::*;
use std::collections::BTreeSet;

impl HistoryReader {
    /// Replay exactly one already accepted commit, including every retained blob.
    /// Returns the accounting index only after the destination commit is identical.
    pub async fn rebuild_commit(
        &self,
        destination: &EncryptedStore,
        state: &DomainState,
        commit: &DomainCommitV1,
    ) -> Result<AccountingIndexV1, EngineError> {
        let mut seen = BTreeSet::new();
        let mut staged = Vec::new();
        let mut instructions = Vec::new();
        let mut batch = None;
        for record in &commit.records {
            let metadata = crate::references::describe(record)?;
            for content in metadata.contents {
                if seen.insert(content.digest.0) {
                    let bytes = self.content_bytes(commit.seq, content.digest).await?;
                    staged.push(destination.stage(&bytes).await?);
                }
            }
            if let AuthoritativeRecordV1::FrozenAccountingManifest(manifest) = record {
                if batch.is_some() {
                    return Err(crate::schema::failure());
                }
                instructions = self
                    .content_bytes(commit.seq, manifest.instructions.digest)
                    .await?;
                batch = Some(AccountingAdapter::from_manifest(manifest, &instructions)?);
            }
        }
        // Independently validate frozen semantics/ownership before writing any authority.
        let mut next = state.clone();
        AccountingAdapter::advance_index(&mut next, commit, &instructions)?;
        AccountingAdapter::verify_correspondence(
            commit,
            batch.as_ref(),
            next.accounting
                .as_ref()
                .ok_or_else(crate::schema::failure)?,
        )?;
        let input = HistoryAppend {
            prior_head: state.through,
            records: commit.records.clone(),
            decisions: commit.decisions.clone(),
            result: commit.command_result.clone(),
            staged,
            native: None,
        };
        let written = if let Some(batch) = batch {
            destination
                .writer()
                .append_closure(
                    input,
                    AccountingAdapter::prepare_closure(&batch)?,
                    &ClosureControl::default(),
                )
                .await
        } else {
            destination.writer().append(input).await
        };
        let written = written.map_err(|_| crate::schema::failure())?;
        if written != *commit {
            return Err(crate::schema::failure());
        }
        next.accounting.ok_or_else(crate::schema::failure)
    }
}
