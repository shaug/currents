//! C06 typed full-correspondence report.
use currents_accounting::AtomicStore;
use currents_types::*;
use sha2::{Digest as _, Sha256};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayReport {
    pub through: CommitSeq,
    pub domain_hash: Digest,
    pub posting_hash: Digest,
    pub native_head: NativeHead,
    pub native_leaf_hash: Digest,
    pub identity_map_hash: Digest,
    pub problems: Vec<EngineError>,
}
/// Source evidence distinguishes an unavailable native-cache observation from a
/// verified leaf digest. A successful reconstructed report always has real hashes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayEvidence {
    pub through: CommitSeq,
    pub domain_hash: Digest,
    pub posting_hash: Digest,
    pub native_head: NativeHead,
    pub native_leaf_hash: Option<Digest>,
    pub identity_map_hash: Digest,
    pub problems: Vec<EngineError>,
}
impl ReplayEvidence {
    pub fn matches(&self, report: &ReplayReport) -> bool {
        self.through == report.through
            && self.domain_hash == report.domain_hash
            && self.posting_hash == report.posting_hash
            && self.native_head == report.native_head
            && self.identity_map_hash == report.identity_map_hash
            && self
                .native_leaf_hash
                .is_none_or(|hash| hash == report.native_leaf_hash)
    }
}
fn framed<T: serde::Serialize>(hash: &mut Sha256, value: &T) -> Result<(), EngineError> {
    let bytes = serde_json_canonicalizer::to_vec(value).map_err(|_| crate::schema::failure())?;
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    Ok(())
}
impl crate::HistoryReader {
    pub async fn audit_report(&self) -> Result<ReplayReport, EngineError> {
        let report = self.replay_evidence().await?;
        if let Some(error) = report.problems.first() {
            return Err(error.clone());
        }
        Ok(ReplayReport {
            through: report.through,
            domain_hash: report.domain_hash,
            posting_hash: report.posting_hash,
            native_head: report.native_head,
            native_leaf_hash: report.native_leaf_hash.ok_or_else(crate::schema::failure)?,
            identity_map_hash: report.identity_map_hash,
            problems: report.problems,
        })
    }
    /// Authority remains mandatory; disposable native projection errors are labeled.
    pub async fn replay_evidence(&self) -> Result<ReplayEvidence, EngineError> {
        crate::schema::attest(&self.pool).await?;
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| crate::schema::failure())?;
        if integrity != "ok" {
            return Err(crate::schema::failure());
        }
        // One verified pass over the history yields both the hashes and the
        // tip; the closing check re-audits the tip.
        let head: i64 = sqlx::query_scalar("SELECT coalesce(max(seq),0) FROM currents_commits")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| crate::schema::failure())?;
        let through = CommitSeq(u64::try_from(head).map_err(|_| crate::schema::failure())?);
        let mut domain = Sha256::new();
        domain.update(b"currents/domain/v1\0");
        let mut postings = Sha256::new();
        postings.update(b"currents/postings/v1\0");
        let mut identities = Sha256::new();
        identities.update(b"currents/identities/v1\0");
        let mut tip = None;
        let mut stream = self.stream_prefix(through).await?;
        while let Some(commit) = stream.next().await? {
            framed(&mut domain, &commit)?;
            framed(
                &mut identities,
                &(&commit.command_result, &commit.native_head_after),
            )?;
            for record in &commit.records {
                match record {
                    AuthoritativeRecordV1::Recognition(recognition) => {
                        framed(&mut postings, recognition)?
                    }
                    AuthoritativeRecordV1::FrozenAccountingManifest(manifest) => {
                        let bytes = self
                            .content_bytes(through, manifest.instructions.digest)
                            .await?;
                        let batch = currents_accounting::AccountingAdapter::from_manifest(
                            manifest, &bytes,
                        )?;
                        for operation in batch.operations {
                            if let FrozenNativeOperationV1::Entry(entry) = operation {
                                framed(&mut postings, &entry)?;
                            }
                        }
                    }
                    _ => {}
                }
            }
            tip = Some(commit);
        }
        drop(stream);
        let accounting = AtomicStore::from_existing_pool(self.pool.clone()).await;
        let observed = match &accounting {
            Ok(accounting) => accounting.read_history_leaves().await.ok(),
            Err(_) => None,
        };
        let native_head = tip
            .as_ref()
            .map(|c| c.native_head_after.clone())
            .or_else(|| observed.as_ref().map(|(head, _)| head.clone()))
            .ok_or_else(crate::schema::failure)?;
        let native_leaf_hash = observed
            .as_ref()
            .filter(|(head, _)| *head == native_head)
            .map(|(_, hash)| *hash);
        let native_audit = match &accounting {
            Ok(accounting) => accounting.audit_history().await,
            Err(error) => Err(error.clone()),
        };
        let mut problems = vec![];
        match native_audit {
            Ok((head, leaves))
                if head == native_head
                    && Some(leaves) == native_leaf_hash
                    && observed
                        .as_ref()
                        .is_some_and(|(observed, _)| *observed == native_head) => {}
            _ => problems.push(crate::schema::failure()),
        }
        if self.audited_tip().await? != tip {
            return Err(crate::schema::failure());
        }
        Ok(ReplayEvidence {
            through,
            domain_hash: Digest(domain.finalize().into()),
            posting_hash: Digest(postings.finalize().into()),
            native_head,
            native_leaf_hash,
            identity_map_hash: Digest(identities.finalize().into()),
            problems,
        })
    }
    /// Bounded full content for frozen instructions and one evidence object.
    pub async fn content_bytes(
        &self,
        through: CommitSeq,
        digest: Digest,
    ) -> Result<Vec<u8>, EngineError> {
        let mut stream = self.stream_content(through, digest).await?;
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await? {
            if bytes.len().saturating_add(chunk.len()) > crate::staging::MAX_FILE {
                return Err(EngineError::invalid());
            }
            bytes.extend(chunk);
        }
        Ok(bytes)
    }
}
