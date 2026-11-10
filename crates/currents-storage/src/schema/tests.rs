//! Synthetic opened-schema evidence and the exact history-reader query plans.
use super::*;
use crate::{EncryptedStore, history};
use currents_accounting::AtomicStore;
use currents_cipher::{CipherPool, CipherProfileV1, DatasetKey, open_pool};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct SchemaInventory {
    schema_sha256: String,
    objects: Vec<[String; 4]>,
}

async fn initialized_pool() -> (tempfile::TempDir, CipherPool) {
    let directory = tempfile::tempdir().unwrap();
    let pool = open_pool(
        &directory.path().join("schema.db"),
        DatasetKey::new([0x68; 32]),
        CipherProfileV1,
        1,
    )
    .await
    .unwrap();
    AtomicStore::from_initialized_pool(pool.pool().clone(), true)
        .await
        .unwrap();
    initialize_authority(pool.pool()).await.unwrap();
    (directory, pool)
}

#[tokio::test]
async fn reviewed_inventory_matches_opened_schema() {
    let (directory, pool) = initialized_pool().await;
    let objects: Vec<[String; 4]> = sqlx::query(
        "SELECT type, name, tbl_name, coalesce(sql,'') AS definition FROM sqlite_schema ORDER BY type, name",
    )
    .fetch_all(pool.pool())
    .await
    .unwrap()
    .into_iter()
    .map(|row| {
        ["type", "name", "tbl_name", "definition"]
            .map(|field| row.get::<String, _>(field))
    })
    .collect();
    let mut digest = Sha256::new();
    for object in &objects {
        for field in object {
            digest.update((field.len() as u64).to_be_bytes());
            digest.update(field.as_bytes());
        }
    }
    let actual = SchemaInventory {
        schema_sha256: hex(&digest.finalize()),
        objects,
    };
    // An explicit output path regenerates evidence for review. It never skips
    // comparison with the compiled-in fixture or the runtime attestation.
    if let Some(path) = std::env::var_os("CURRENTS_SCHEMA_INVENTORY_OUTPUT") {
        std::fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()),
        )
        .unwrap();
    }
    let reviewed: SchemaInventory =
        serde_json::from_str(include_str!("../../../../fixtures/storage/schema-v1.json")).unwrap();
    assert_eq!(
        actual.schema_sha256, reviewed.schema_sha256,
        "reviewed schema fingerprint drifted"
    );
    assert_eq!(
        actual.objects, reviewed.objects,
        "reviewed schema objects drifted"
    );
    let attestation = attest(pool.pool()).await.unwrap();
    assert_eq!(actual.schema_sha256, attestation.schema_sha256);
    pool.close().await;
    let reopened = EncryptedStore::open(
        &directory.path().join("schema.db"),
        DatasetKey::new([0x68; 32]),
        CipherProfileV1,
    )
    .await
    .unwrap();
    assert_eq!(actual.schema_sha256, reopened.attestation().schema_sha256);
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn history_item_queries_use_covering_indexes() {
    let (_directory, pool) = initialized_pool().await;
    let queries = [
        (
            history::ITEM_REFERENCE_COUNT,
            "currents_references",
            "currents_references_by_item",
        ),
        (
            history::ITEM_REFERENCES,
            "currents_references",
            "currents_references_by_item",
        ),
        (
            history::ITEM_DEFINITIONS,
            "currents_definitions",
            "currents_definitions_by_item",
        ),
        (
            history::ITEM_CONTENT_COUNT,
            "currents_content_refs",
            "currents_content_refs_by_item",
        ),
        (
            history::ITEM_CONTENTS,
            "currents_content_refs",
            "currents_content_refs_by_item",
        ),
    ];
    let mut failures = Vec::new();
    for (sql, table, index) in queries {
        let explain = format!("EXPLAIN QUERY PLAN {sql}");
        let mut query = sqlx::query(sqlx::AssertSqlSafe(explain.as_str()));
        // EXPLAIN checks the actual statement without reading financial values.
        for _ in 0..sql.matches('?').count() {
            query = query.bind(1_i64);
        }
        let plan = query.fetch_all(pool.pool()).await.unwrap();
        let details: Vec<String> = plan.iter().map(|row| row.get("detail")).collect();
        if details.len() != 1
            || !details[0].starts_with(&format!("SEARCH {table} USING COVERING INDEX {index} "))
            || !details[0].contains("(seq=? AND position=?")
        {
            failures.push(format!("{sql}: {details:?}"));
        }
    }
    pool.close().await;
    assert!(failures.is_empty(), "history lookup scans: {failures:#?}");
}
