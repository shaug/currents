//! Real encrypted-store regressions for commit-local content verification.
use super::*;
use crate::EncryptedStore;
use currents_cipher::{CipherPool, CipherProfileV1, DatasetKey};
use std::{cell::Cell, path::Path};

// Only the actual chunk-hash path calls this hook. Each current-thread Tokio
// test owns its counter, so parallel tests cannot affect each other's evidence.
thread_local! {
    static CHUNK_VERIFICATIONS: Cell<usize> = const { Cell::new(0) };
}
pub(super) fn record_chunk_verification() {
    CHUNK_VERIFICATIONS.with(|count| count.set(count.get() + 1));
}
fn take_chunk_verifications() -> usize {
    CHUNK_VERIFICATIONS.with(|count| count.replace(0))
}
fn id(n: u128) -> RecordId {
    RecordId(uuid::Uuid::from_u128(n))
}
async fn open(path: &Path) -> EncryptedStore {
    EncryptedStore::open(path, DatasetKey::new([0x47; 32]), CipherProfileV1)
        .await
        .unwrap()
}
async fn tamper_pool(path: &Path) -> CipherPool {
    currents_cipher::open_pool(path, DatasetKey::new([0x47; 32]), CipherProfileV1, 1)
        .await
        .unwrap()
}
async fn append_shared(store: &EncryptedStore, prior: u64) -> DomainCommitV1 {
    let source = SourceIdentityV1 {
        source: "synthetic".into(),
        connection_id: id(999),
        device_id: None,
    };
    // Three chunks shared by 64 observations and a later receipt, plus one
    // distinct single-chunk blob. Expected work is four chunk hashes, not 196.
    let bytes = vec![0x61; 2 * staging::CHUNK + 37];
    let first = store.stage(&bytes).await.unwrap();
    let second = store.stage(b"distinct").await.unwrap();
    let base = u128::from(prior) * 1000;
    let mut records: Vec<_> = (0..64)
        .map(|n| {
            AuthoritativeRecordV1::ObservationVersion(ObservationVersionV1 {
                id: id(base + n + 1),
                source: source.clone(),
                original_ids: vec![],
                sanitized_representation: "shared synthetic content".into(),
                reported_fields: vec![],
                temporal: TemporalValue::Unknown {
                    original_text: String::new(),
                },
                semantic_kind: ObservationKind::Movement,
                prior_version: None,
                tombstone_of: None,
                content_digest: first.digest(),
            })
        })
        .collect();
    // The first use has no expected byte count. The receipt's later, stronger
    // size assertion must still be checked when the digest has been verified.
    records.push(AuthoritativeRecordV1::ImportReceipt(ImportReceiptV1 {
        id: id(base + 100),
        source,
        adapter_version: "1".into(),
        parser_version: "1".into(),
        requested_coverage: vec![],
        reported_coverage: vec![],
        received_time: serde_json::from_str("\"2026-01-01T00:00:00Z\"").unwrap(),
        result: ImportResult::Complete,
        cursor: SourceValue::Known(format!("cursor-{prior}")),
        truncated: false,
        complete: true,
        contents: vec![
            ContentReferenceV1 {
                digest: first.digest(),
                byte_count: CommitSeq(bytes.len() as u64),
                media_type: "text/plain".into(),
            },
            ContentReferenceV1 {
                digest: second.digest(),
                byte_count: CommitSeq(8),
                media_type: "text/plain".into(),
            },
        ],
    }));
    store
        .writer()
        .append(HistoryAppend {
            prior_head: CommitSeq(prior),
            records,
            decisions: vec![],
            result: CommandResultV1 {
                command_id: CommandId(id(base + 200).0),
                digest: hash(&prior.to_be_bytes()),
                state: CommandState::Committed,
                commit: None,
                effects: EffectsV1 {
                    records: vec![],
                    accounts: vec![],
                    amounts: vec![],
                    invalidations: vec![],
                },
                health: vec![],
            },
            staged: vec![first, second],
            native: None,
        })
        .await
        .unwrap()
}
fn shared_digest(commit: &DomainCommitV1) -> Digest {
    let AuthoritativeRecordV1::ObservationVersion(observation) = &commit.records[0] else {
        panic!("observation missing")
    };
    observation.content_digest
}

#[tokio::test]
async fn shared_content_is_verified_once_per_commit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.db");
    let store = open(&path).await;
    let first = append_shared(&store, 0).await;
    let mut pinned = store.history().stream_prefix(first.seq).await.unwrap();
    let second = append_shared(&store, 1).await;
    take_chunk_verifications();
    assert_eq!(pinned.next().await.unwrap(), Some(first.clone()));
    assert_eq!(
        take_chunk_verifications(),
        4,
        "hash each distinct blob once"
    );
    assert!(pinned.next().await.unwrap().is_none());
    assert_eq!(take_chunk_verifications(), 0);
    drop(pinned);

    let mut both = store.history().stream_prefix(second.seq).await.unwrap();
    assert_eq!(both.next().await.unwrap(), Some(first.clone()));
    assert_eq!(take_chunk_verifications(), 4);
    assert_eq!(both.next().await.unwrap(), Some(second.clone()));
    assert_eq!(take_chunk_verifications(), 4, "reuse ends at each commit");
    assert!(both.next().await.unwrap().is_none());
    drop(both);

    assert_eq!(
        store.history().read_prefix(first.seq).await.unwrap(),
        vec![first]
    );
    assert_eq!(take_chunk_verifications(), 4, "fresh readers verify again");
    let mut content = store
        .history()
        .stream_content(CommitSeq(1), shared_digest(&second))
        .await
        .unwrap();
    assert_eq!(take_chunk_verifications(), 3);
    let mut sizes = Vec::new();
    while let Some(chunk) = content.next().await.unwrap() {
        sizes.push(chunk.len());
    }
    assert_eq!(sizes, vec![1_048_576, 1_048_576, 37]);
    drop(content);
    store.close().await.unwrap();
    let reopened = open(&path).await;
    assert_eq!(take_chunk_verifications(), 8, "startup audits both commits");
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn repeated_digest_still_checks_later_receipt_size() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("size.db");
    let store = open(&path).await;
    let mut commit = append_shared(&store, 0).await;
    let AuthoritativeRecordV1::ImportReceipt(receipt) = &mut commit.records[64] else {
        panic!("receipt missing")
    };
    receipt.contents[0].byte_count.0 += 1;
    // Recompute both canonical hashes so only the cached reference's expected
    // size is wrong; an unrelated commit/item digest failure cannot pass this.
    commit.digest = Digest::default();
    commit.digest = hash(&canonical(&commit).unwrap());
    let pool = tamper_pool(&path).await;
    sqlx::raw_sql(
        "DROP TRIGGER currents_commits_no_update; DROP TRIGGER currents_items_no_update;",
    )
    .execute(pool.pool())
    .await
    .unwrap();
    sqlx::query("UPDATE currents_commits SET canonical=?,digest=? WHERE seq=1")
        .bind(canonical(&commit).unwrap())
        .bind(commit.digest.0.as_slice())
        .execute(pool.pool())
        .await
        .unwrap();
    let item = canonical(&commit.records[64]).unwrap();
    sqlx::query("UPDATE currents_items SET canonical=?,digest=? WHERE seq=1 AND position=64")
        .bind(&item)
        .bind(hash(&item).0.as_slice())
        .execute(pool.pool())
        .await
        .unwrap();
    take_chunk_verifications();
    assert!(store.history().read_prefix(commit.seq).await.is_err());
    assert_eq!(
        take_chunk_verifications(),
        3,
        "size checked after cache hit"
    );
    pool.close().await;
    store.close().await.unwrap();
}

#[tokio::test]
async fn repeated_digest_still_checks_reference_membership() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("membership.db");
    let store = open(&path).await;
    let commit = append_shared(&store, 0).await;
    let pool = tamper_pool(&path).await;
    // Keep the same row count while substituting a different valid digest at a
    // later observation. Earlier references have already populated the cache.
    sqlx::raw_sql("DROP TRIGGER currents_content_refs_no_update; UPDATE currents_content_refs SET digest=(SELECT digest FROM currents_blobs WHERE byte_count=8) WHERE seq=1 AND position=63;")
        .execute(pool.pool())
        .await
        .unwrap();
    take_chunk_verifications();
    assert!(store.history().read_prefix(commit.seq).await.is_err());
    assert_eq!(take_chunk_verifications(), 3);
    pool.close().await;
    store.close().await.unwrap();
}

#[tokio::test]
async fn content_verification_rejects_missing_corrupt_and_future_blobs() {
    for mutation in [
        "DROP TRIGGER currents_chunks_no_delete; DELETE FROM currents_chunks WHERE position=1;",
        "DROP TRIGGER currents_chunks_no_update; UPDATE currents_chunks SET bytes=zeroblob(length(bytes)) WHERE position=1;",
        "DROP TRIGGER currents_chunks_no_update; UPDATE currents_chunks SET chunk_digest=zeroblob(32) WHERE position=1;",
        "DROP TRIGGER currents_blobs_no_update; UPDATE currents_blobs SET first_seq=2;",
        "PRAGMA foreign_keys=OFF; DROP TRIGGER currents_blobs_no_delete; DELETE FROM currents_blobs;",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("corrupt.db");
        let store = open(&path).await;
        let first = append_shared(&store, 0).await;
        let second = append_shared(&store, 1).await;
        // Warm a separate reader first: neither another prefix nor a content
        // reader may inherit verification from an earlier database snapshot.
        assert_eq!(
            store.history().read_prefix(first.seq).await.unwrap(),
            vec![first.clone()]
        );
        let pool = tamper_pool(&path).await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(mutation))
            .execute(pool.pool())
            .await
            .unwrap();
        assert!(
            store.history().read_prefix(first.seq).await.is_err(),
            "{mutation}"
        );
        assert!(
            store.history().read_prefix(second.seq).await.is_err(),
            "{mutation}"
        );
        assert!(
            store
                .history()
                .stream_content(first.seq, shared_digest(&first))
                .await
                .is_err(),
            "{mutation}"
        );
        pool.close().await;
        store.close().await.unwrap();
    }
}

#[tokio::test]
async fn pinned_verification_does_not_cross_snapshot_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapshot.db");
    let store = open(&path).await;
    let first = append_shared(&store, 0).await;
    let second = append_shared(&store, 1).await;
    let mut pinned = store.history().stream_prefix(second.seq).await.unwrap();
    take_chunk_verifications();
    assert_eq!(pinned.next().await.unwrap(), Some(first.clone()));
    assert_eq!(take_chunk_verifications(), 4);

    // A privileged writer changes a chunk and its matching chunk digest. Only
    // the whole-file digest now detects corruption. The already pinned reader
    // must still verify the original bytes, including in its next commit.
    let pool = tamper_pool(&path).await;
    sqlx::query("DROP TRIGGER currents_chunks_no_update")
        .execute(pool.pool())
        .await
        .unwrap();
    let replacement = vec![0; staging::CHUNK];
    sqlx::query("UPDATE currents_chunks SET bytes=?,chunk_digest=? WHERE position=1")
        .bind(&replacement)
        .bind(hash(&replacement).0.as_slice())
        .execute(pool.pool())
        .await
        .unwrap();
    assert_eq!(pinned.next().await.unwrap(), Some(second.clone()));
    assert_eq!(
        take_chunk_verifications(),
        4,
        "same pinned snapshot at next commit"
    );
    assert!(pinned.next().await.unwrap().is_none());
    drop(pinned);

    assert!(store.history().read_prefix(second.seq).await.is_err());
    assert_eq!(
        take_chunk_verifications(),
        3,
        "whole-file hash rejects fresh bytes"
    );
    assert!(
        store
            .history()
            .stream_content(first.seq, shared_digest(&first))
            .await
            .is_err()
    );
    assert_eq!(take_chunk_verifications(), 3);
    pool.close().await;
    store.close().await.unwrap();
}
