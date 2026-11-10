//! Ordered immutable history, with bounded prefix iteration and independent checkpoints.
use crate::{CommitFailure, StagedBatchHandle, Writer, schema, staging};
use currents_accounting::{AtomicFailure, PreparedNativeOperation, verify_native_checkpoint};
use currents_types::*;
use sha2::{Digest as _, Sha256};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

const MAX_COMMIT: usize = 64 * 1_048_576;
const MAX_ITEMS: usize = 20_000;
// Shared with the private schema query-plan regression so it explains the reader SQL.
pub(crate) const ITEM_REFERENCE_COUNT: &str =
    "SELECT count(*) FROM currents_references WHERE seq=? AND position=?";
pub(crate) const ITEM_REFERENCE_MEMBERSHIP: &str = "SELECT count(*) FROM currents_references WHERE seq=? AND position=? AND target_kind=? AND target_id=? AND target_version=?";
pub(crate) const ITEM_REFERENCES: &str = "SELECT target_kind,target_id,target_version FROM currents_references WHERE seq=? AND position=?";
pub(crate) const ITEM_DEFINITIONS: &str =
    "SELECT kind,id,version FROM currents_definitions WHERE seq=? AND position=?";
pub(crate) const ITEM_CONTENT_COUNT: &str =
    "SELECT count(*) FROM currents_content_refs WHERE seq=? AND position=?";
pub(crate) const ITEM_CONTENTS: &str =
    "SELECT digest FROM currents_content_refs WHERE seq=? AND position=?";
pub(crate) const ITEM_CONTENT_MEMBERSHIP: &str =
    "SELECT count(*) FROM currents_content_refs WHERE seq=? AND position=? AND digest=?";
fn canonical<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, EngineError> {
    serde_json_canonicalizer::to_vec(v).map_err(|_| EngineError::invalid())
}
fn hash(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}
fn seq(v: CommitSeq) -> Result<i64, EngineError> {
    i64::try_from(v.0).map_err(|_| EngineError::invalid())
}
fn atomic(e: AtomicFailure) -> CommitFailure {
    match e {
        AtomicFailure::RolledBack(e) => CommitFailure::DefinitelyRolledBack(e),
        AtomicFailure::Unknown(id) => CommitFailure::OutcomeUnknown(id),
    }
}
fn failed(e: EngineError) -> CommitFailure {
    CommitFailure::DefinitelyRolledBack(e)
}
/// Storage-ready typed inputs only. C05 freezing and C10 command lifecycle are separate.
pub struct HistoryAppend {
    pub prior_head: CommitSeq,
    pub records: Vec<AuthoritativeRecordV1>,
    pub decisions: Vec<DecisionEnvelopeV1>,
    pub result: CommandResultV1,
    pub staged: Vec<StagedBatchHandle>,
    pub native: Option<PreparedNativeOperation>,
}
impl Writer {
    pub async fn append(&self, input: HistoryAppend) -> Result<DomainCommitV1, CommitFailure> {
        if input.records.len().saturating_add(input.decisions.len()) > MAX_ITEMS
            || input
                .records
                .iter()
                .filter(|r| matches!(r, AuthoritativeRecordV1::ObservationVersion(_)))
                .count()
                > 10_000
        {
            return Err(failed(EngineError::invalid()));
        }
        for staged in &input.staged {
            staging::verify(&self.pool, staged).await.map_err(failed)?;
        }
        let mut tx = self
            .accounting
            .begin_authority(input.result.command_id, input.native.as_ref())
            .await
            .map_err(atomic)?;
        let work = async {
            let previous = tx.tip().await?;
            let previous: Option<DomainCommitV1> = previous
                .map(|b| serde_json::from_slice(&b).map_err(|_| schema::failure()))
                .transpose()?;
            let prior = previous.as_ref().map_or(CommitSeq(0), |c| c.seq);
            if prior != input.prior_head {
                return Err(EngineError::invalid());
            }
            let next = CommitSeq(prior.0.checked_add(1).ok_or_else(EngineError::invalid)?);
            let number = seq(next)?;
            let mut result = input.result.clone();
            if result.state != CommandState::Committed {
                return Err(EngineError::invalid());
            }
            result.commit = Some(next);
            let mut commit = DomainCommitV1 {
                seq: next,
                previous_digest: previous.as_ref().map_or(Digest::default(), |c| c.digest),
                records: input.records,
                decisions: input.decisions,
                native_head_after: tx.native_head(),
                command_result: result,
                digest: Digest::default(),
            };
            for (offset, d) in commit.decisions.iter().enumerate() {
                if d.commit != next
                    || d.item_position as usize != commit.records.len().saturating_add(offset)
                    || d.command_id != commit.command_result.command_id
                    || d.command_digest != commit.command_result.digest
                {
                    return Err(EngineError::invalid());
                }
            }
            let bytes = canonical(&commit)?;
            if bytes.len() > MAX_COMMIT {
                return Err(EngineError::invalid());
            }
            commit.digest = hash(&bytes);
            let bytes = canonical(&commit)?;
            tx.insert_commit(
                number,
                &commit.previous_digest.0,
                &commit.digest.0,
                &bytes,
                commit.command_result.command_id.0.as_bytes(),
            )
            .await?;
            let mut items = commit.records.clone();
            items.extend(
                commit
                    .decisions
                    .iter()
                    .cloned()
                    .map(AuthoritativeRecordV1::Decision),
            );
            let mut metadata = Vec::with_capacity(items.len());
            for (position, item) in items.iter().enumerate() {
                let bytes = canonical(item)?;
                if bytes.len() > staging::CHUNK {
                    return Err(EngineError::invalid());
                }
                let meta = crate::references::describe(item)?;
                if meta.prefixes.iter().any(|p| *p > next.0) {
                    return Err(EngineError::invalid());
                }
                if let AuthoritativeRecordV1::Decision(d) = item
                    && (d.commit != next
                        || d.item_position as usize != position
                        || d.command_id != commit.command_result.command_id
                        || d.command_digest != commit.command_result.digest)
                {
                    return Err(EngineError::invalid());
                }
                tx.insert_item(number, position as i64, &meta.kind, &hash(&bytes).0, &bytes)
                    .await?;
                for definition in &meta.definitions {
                    if definition.family
                        && tx
                            .definition(&definition.kind, &definition.id, Some(definition.version))
                            .await?
                            .is_some()
                    {
                        continue;
                    }
                    tx.insert_definition(
                        &definition.kind,
                        &definition.id,
                        definition.version,
                        number,
                        position as i64,
                    )
                    .await?;
                }
                metadata.push(meta);
            }
            for staged in &input.staged {
                tx.promote_content(
                    staged.token.as_bytes(),
                    &staged.digest.0,
                    staged.size as i64,
                    number,
                )
                .await?;
            }
            for (position, meta) in metadata.iter().enumerate() {
                for reference in &meta.references {
                    let reference = if reference.kinds == ["DatasetCatalog"] {
                        catalog_reference(reference, tx.dataset_record(&reference.id).await?)?
                    } else {
                        reference.clone()
                    };
                    let mut found = None;
                    for kind in &reference.kinds {
                        if let Some(version) = tx
                            .definition(kind, &reference.id, reference.version)
                            .await?
                        {
                            if found.is_some() {
                                return Err(EngineError::invalid());
                            }
                            found = Some((kind, version));
                        }
                    }
                    let (kind, version) = found.ok_or_else(EngineError::invalid)?;
                    tx.insert_reference(number, position as i64, kind, &reference.id, version)
                        .await?;
                }
                for content in &meta.contents {
                    let size = tx
                        .content_size(&content.digest.0)
                        .await?
                        .ok_or_else(EngineError::invalid)?;
                    if content.size.is_some_and(|v| v != size) {
                        return Err(EngineError::invalid());
                    }
                    tx.insert_content_reference(number, position as i64, &content.digest.0)
                        .await?;
                }
            }
            Ok::<_, EngineError>(commit)
        }
        .await;
        match work {
            Ok(commit) => {
                tx.commit().await.map_err(atomic)?;
                Ok(commit)
            }
            Err(e) => {
                tx.rollback().await.map_err(atomic)?;
                Err(failed(e))
            }
        }
    }
}
#[derive(Clone)]
pub struct HistoryReader {
    pool: SqlitePool,
}
impl HistoryReader {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub(crate) async fn audit(&self) -> Result<(), EngineError> {
        let head: i64 = sqlx::query_scalar("SELECT coalesce(max(seq),0) FROM currents_commits")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| schema::failure())?;
        let mut stream = self
            .stream_prefix(CommitSeq(
                u64::try_from(head).map_err(|_| schema::failure())?,
            ))
            .await?;
        while stream.next().await?.is_some() {}
        Ok(())
    }
    /// A fixed database snapshot. Each next() holds at most one bounded commit.
    pub async fn stream_prefix(&self, through: CommitSeq) -> Result<HistoryStream, EngineError> {
        let mut tx = self.pool.begin().await.map_err(|_| schema::failure())?;
        let head: i64 = sqlx::query_scalar("SELECT coalesce(max(seq),0) FROM currents_commits")
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| schema::failure())?;
        if seq(through)? > head {
            return Err(schema::failure());
        }
        Ok(HistoryStream {
            tx,
            next: 1,
            through: seq(through)?,
            previous: Digest::default(),
        })
    }
    /// Convenience reader bounded to 64 MiB total; archives use stream_prefix.
    pub async fn read_prefix(
        &self,
        through: CommitSeq,
    ) -> Result<Vec<DomainCommitV1>, EngineError> {
        let mut stream = self.stream_prefix(through).await?;
        let mut result = Vec::new();
        let mut bytes = 0usize;
        while let Some(c) = stream.next().await? {
            bytes = bytes
                .checked_add(canonical(&c)?.len())
                .ok_or_else(schema::failure)?;
            if bytes > MAX_COMMIT {
                return Err(EngineError::invalid());
            }
            result.push(c);
        }
        Ok(result)
    }
    pub async fn verify_checkpoint(
        &self,
        through: CommitSeq,
        digest: Digest,
    ) -> Result<(), EngineError> {
        let mut stream = self.stream_prefix(through).await?;
        while stream.next().await?.is_some() {}
        if stream.previous != digest {
            return Err(schema::failure());
        }
        Ok(())
    }
    /// Verify content in the pinned snapshot before yielding bounded chunks.
    pub async fn stream_content(
        &self,
        through: CommitSeq,
        digest: Digest,
    ) -> Result<ContentStream, EngineError> {
        let prefix = self.stream_prefix(through).await?;
        let mut tx = prefix.tx;
        verify_blob(&mut tx, digest, seq(through)?, None).await?;
        Ok(ContentStream {
            tx,
            digest,
            next: 0,
        })
    }
    pub async fn content_count(&self, through: CommitSeq) -> Result<u64, EngineError> {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM currents_blobs WHERE first_seq<=?")
            .bind(seq(through)?)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| schema::failure())?;
        u64::try_from(n).map_err(|_| schema::failure())
    }
}
pub struct HistoryStream {
    tx: Transaction<'static, Sqlite>,
    next: i64,
    through: i64,
    previous: Digest,
}
impl HistoryStream {
    pub async fn next(&mut self) -> Result<Option<DomainCommitV1>, EngineError> {
        if self.next > self.through {
            return Ok(None);
        }
        let row=sqlx::query("SELECT canonical,checkpoint,digest,previous_digest,command_id FROM currents_commits WHERE seq=?").bind(self.next).fetch_optional(&mut *self.tx).await.map_err(|_|schema::failure())?.ok_or_else(schema::failure)?;
        let bytes: Vec<u8> = row.try_get("canonical").map_err(|_| schema::failure())?;
        if bytes.len() > MAX_COMMIT {
            return Err(schema::failure());
        }
        let mut c: DomainCommitV1 =
            serde_json::from_slice(&bytes).map_err(|_| schema::failure())?;
        if canonical(&c)? != bytes
            || seq(c.seq)? != self.next
            || c.previous_digest != self.previous
            || c.command_result.commit != Some(c.seq)
            || c.command_result.state != CommandState::Committed
        {
            return Err(schema::failure());
        }
        let stored = c.digest;
        c.digest = Digest::default();
        if hash(&canonical(&c)?) != stored {
            return Err(schema::failure());
        }
        c.digest = stored;
        let digest: Vec<u8> = row.try_get("digest").map_err(|_| schema::failure())?;
        let prev: Vec<u8> = row
            .try_get("previous_digest")
            .map_err(|_| schema::failure())?;
        let command: Vec<u8> = row.try_get("command_id").map_err(|_| schema::failure())?;
        if digest != stored.0
            || prev != c.previous_digest.0
            || command != c.command_result.command_id.0.as_bytes()
        {
            return Err(schema::failure());
        }
        let checkpoint: String = row.try_get("checkpoint").map_err(|_| schema::failure())?;
        verify_native_checkpoint(&c.native_head_after, &checkpoint)
            .map_err(|_| schema::failure())?;
        let mut items = c.records.clone();
        items.extend(
            c.decisions
                .iter()
                .cloned()
                .map(AuthoritativeRecordV1::Decision),
        );
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM currents_items WHERE seq=?")
            .bind(self.next)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(|_| schema::failure())?;
        if count != items.len() as i64 {
            return Err(schema::failure());
        }
        for (position, item) in items.iter().enumerate() {
            let bytes = canonical(item)?;
            let row = sqlx::query(
                "SELECT canonical,digest,kind FROM currents_items WHERE seq=? AND position=?",
            )
            .bind(self.next)
            .bind(position as i64)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(|_| schema::failure())?
            .ok_or_else(schema::failure)?;
            let actual: Vec<u8> = row.try_get("canonical").map_err(|_| schema::failure())?;
            let digest: Vec<u8> = row.try_get("digest").map_err(|_| schema::failure())?;
            let kind: String = row.try_get("kind").map_err(|_| schema::failure())?;
            let meta = crate::references::describe(item)?;
            if meta.prefixes.iter().any(|p| *p > c.seq.0) {
                return Err(schema::failure());
            }
            if actual != bytes || digest != hash(&bytes).0 || kind != meta.kind {
                return Err(schema::failure());
            }
            let reference_count: i64 = sqlx::query_scalar(ITEM_REFERENCE_COUNT)
                .bind(self.next)
                .bind(position as i64)
                .fetch_one(&mut *self.tx)
                .await
                .map_err(|_| schema::failure())?;
            if reference_count != meta.references.len() as i64 {
                return Err(schema::failure());
            }
            let mut expected_references = Vec::new();
            for reference in &meta.references {
                let reference = if reference.kinds == ["DatasetCatalog"] {
                    let bytes=sqlx::query_scalar("SELECT i.canonical FROM currents_definitions d JOIN currents_items i ON i.seq=d.seq AND i.position=d.position WHERE d.kind='Dataset' AND d.id=? AND d.version=0 AND d.seq<=?").bind(&reference.id).bind(self.next).fetch_optional(&mut *self.tx).await.map_err(|_|schema::failure())?;
                    catalog_reference(reference, bytes)?
                } else {
                    reference.clone()
                };
                let mut matches = 0usize;
                for kind in &reference.kinds {
                    let found:Option<i64>=sqlx::query_scalar("SELECT version FROM currents_definitions WHERE kind=? AND id=? AND seq<=? AND (? IS NULL OR version=?) ORDER BY version LIMIT 1").bind(kind).bind(&reference.id).bind(self.next).bind(reference.version).bind(reference.version).fetch_optional(&mut *self.tx).await.map_err(|_|schema::failure())?;
                    if let Some(version) = found {
                        let count: i64 = sqlx::query_scalar(ITEM_REFERENCE_MEMBERSHIP)
                            .bind(self.next)
                            .bind(position as i64)
                            .bind(kind)
                            .bind(&reference.id)
                            .bind(version)
                            .fetch_one(&mut *self.tx)
                            .await
                            .map_err(|_| schema::failure())?;
                        if count == 0 {
                            return Err(schema::failure());
                        }
                        expected_references.push((kind.clone(), reference.id.clone(), version));
                        matches = matches.saturating_add(1);
                    }
                }
                if matches != 1 {
                    return Err(schema::failure());
                }
            }
            let rows = sqlx::query(ITEM_REFERENCES)
                .bind(self.next)
                .bind(position as i64)
                .fetch_all(&mut *self.tx)
                .await
                .map_err(|_| schema::failure())?;
            let mut actual_references = rows
                .iter()
                .map(|r| {
                    Ok((
                        r.try_get::<String, _>("target_kind")?,
                        r.try_get::<String, _>("target_id")?,
                        r.try_get::<i64, _>("target_version")?,
                    ))
                })
                .collect::<Result<Vec<_>, sqlx::Error>>()
                .map_err(|_| schema::failure())?;
            expected_references.sort();
            actual_references.sort();
            if expected_references != actual_references {
                return Err(schema::failure());
            }
            let mut expected_definitions = Vec::new();
            for definition in &meta.definitions {
                let row=sqlx::query("SELECT seq,position FROM currents_definitions WHERE kind=? AND id=? AND version=?").bind(&definition.kind).bind(&definition.id).bind(definition.version).fetch_optional(&mut *self.tx).await.map_err(|_|schema::failure())?.ok_or_else(schema::failure)?;
                let at: i64 = row.try_get("seq").map_err(|_| schema::failure())?;
                let pos: i64 = row.try_get("position").map_err(|_| schema::failure())?;
                if at == self.next && pos == position as i64 {
                    expected_definitions.push((
                        definition.kind.clone(),
                        definition.id.clone(),
                        definition.version,
                    ));
                }
                if at > self.next
                    || (!definition.family && (at != self.next || pos != position as i64))
                {
                    return Err(schema::failure());
                }
            }
            let rows = sqlx::query(ITEM_DEFINITIONS)
                .bind(self.next)
                .bind(position as i64)
                .fetch_all(&mut *self.tx)
                .await
                .map_err(|_| schema::failure())?;
            let mut actual_definitions = rows
                .iter()
                .map(|r| {
                    Ok((
                        r.try_get::<String, _>("kind")?,
                        r.try_get::<String, _>("id")?,
                        r.try_get::<i64, _>("version")?,
                    ))
                })
                .collect::<Result<Vec<_>, sqlx::Error>>()
                .map_err(|_| schema::failure())?;
            expected_definitions.sort();
            expected_definitions.dedup();
            actual_definitions.sort();
            if expected_definitions != actual_definitions {
                return Err(schema::failure());
            }
            let content_count: i64 = sqlx::query_scalar(ITEM_CONTENT_COUNT)
                .bind(self.next)
                .bind(position as i64)
                .fetch_one(&mut *self.tx)
                .await
                .map_err(|_| schema::failure())?;
            if content_count != meta.contents.len() as i64 {
                return Err(schema::failure());
            }
            let mut expected_content: Vec<Vec<u8>> =
                meta.contents.iter().map(|c| c.digest.0.to_vec()).collect();
            let mut actual_content: Vec<Vec<u8>> = sqlx::query_scalar(ITEM_CONTENTS)
                .bind(self.next)
                .bind(position as i64)
                .fetch_all(&mut *self.tx)
                .await
                .map_err(|_| schema::failure())?;
            expected_content.sort();
            actual_content.sort();
            if expected_content != actual_content {
                return Err(schema::failure());
            }
            for content in meta.contents {
                let count: i64 = sqlx::query_scalar(ITEM_CONTENT_MEMBERSHIP)
                    .bind(self.next)
                    .bind(position as i64)
                    .bind(content.digest.0.as_slice())
                    .fetch_one(&mut *self.tx)
                    .await
                    .map_err(|_| schema::failure())?;
                if count == 0 {
                    return Err(schema::failure());
                }
                verify_blob(&mut self.tx, content.digest, self.next, content.size).await?;
            }
        }
        self.previous = stored;
        self.next = self.next.checked_add(1).ok_or_else(schema::failure)?;
        Ok(Some(c))
    }
}
async fn verify_blob(
    tx: &mut Transaction<'static, Sqlite>,
    digest: Digest,
    through: i64,
    expected: Option<i64>,
) -> Result<(), EngineError> {
    let size: Option<i64> =
        sqlx::query_scalar("SELECT byte_count FROM currents_blobs WHERE digest=? AND first_seq<=?")
            .bind(digest.0.as_slice())
            .bind(through)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| schema::failure())?;
    let size = size.ok_or_else(schema::failure)?;
    if expected.is_some_and(|v| v != size) {
        return Err(schema::failure());
    }
    let mut total = 0usize;
    let mut index = 0i64;
    let mut hash = Sha256::new();
    loop {
        let row = sqlx::query(
            "SELECT bytes,chunk_digest FROM currents_chunks WHERE digest=? AND position=?",
        )
        .bind(digest.0.as_slice())
        .bind(index)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| schema::failure())?;
        let Some(row) = row else { break };
        let bytes: Vec<u8> = row.try_get("bytes").map_err(|_| schema::failure())?;
        let chunk: Vec<u8> = row.try_get("chunk_digest").map_err(|_| schema::failure())?;
        if bytes.len() > staging::CHUNK || Sha256::digest(&bytes).as_slice() != chunk {
            return Err(schema::failure());
        }
        total = total.saturating_add(bytes.len());
        if total > staging::MAX_FILE {
            return Err(schema::failure());
        }
        hash.update(bytes);
        index = index.saturating_add(1);
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM currents_chunks WHERE digest=?")
        .bind(digest.0.as_slice())
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| schema::failure())?;
    if count != index || total as i64 != size || <[u8; 32]>::from(hash.finalize()) != digest.0 {
        return Err(schema::failure());
    }
    Ok(())
}

/// Streaming bytes from an already verified immutable content object and snapshot.
pub struct ContentStream {
    tx: Transaction<'static, Sqlite>,
    digest: Digest,
    next: i64,
}
impl ContentStream {
    pub async fn next(&mut self) -> Result<Option<Vec<u8>>, EngineError> {
        let bytes =
            sqlx::query_scalar("SELECT bytes FROM currents_chunks WHERE digest=? AND position=?")
                .bind(self.digest.0.as_slice())
                .bind(self.next)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(|_| schema::failure())?;
        self.next = self.next.checked_add(1).ok_or_else(schema::failure)?;
        Ok(bytes)
    }
}

fn catalog_reference(
    reference: &crate::references::Reference,
    bytes: Option<Vec<u8>>,
) -> Result<crate::references::Reference, EngineError> {
    let record: AuthoritativeRecordV1 =
        serde_json::from_slice(&bytes.ok_or_else(EngineError::invalid)?)
            .map_err(|_| schema::failure())?;
    let AuthoritativeRecordV1::Dataset(dataset) = record else {
        return Err(schema::failure());
    };
    Ok(crate::references::Reference {
        kinds: vec!["UnitCatalog".into()],
        id: dataset.unit_catalog.0.to_string(),
        version: reference.version,
    })
}
