//! Read-only native correspondence evidence; no native handle escapes this boundary.
use crate::AtomicStore;
use currents_types::{Digest, EngineError, ErrorClass, NativeHead, SafeNextAction};
use doubleentry::store::Store;
use doubleentry::{
    Ledger, LedgerId,
    transparency::{Checkpoint, KeyName},
};
use sha2::{Digest as _, Sha256};

fn failure() -> EngineError {
    EngineError {
        class: ErrorClass::IntegrityFailure,
        affected_scope: vec![],
        next_action: SafeNextAction::Recover,
    }
}
impl AtomicStore {
    /// Reads canonical native leaves independently of derived account/balance
    /// tables, so intact authority can repair a damaged disposable projection.
    pub async fn read_history_leaves(&self) -> Result<(NativeHead, Digest), EngineError> {
        let info = self.store.open().await.map_err(|_| failure())?;
        let id = LedgerId::new("currents/dataset-v1").map_err(|_| failure())?;
        let mut replay = doubleentry::store::Replayer::<18>::new(&id);
        let mut hash = Sha256::new();
        hash.update(b"currents/native-leaves/v1\0");
        let mut position = 0u64;
        while position < info.size {
            let page = self
                .store
                .read_leaves(&doubleentry::store::LeafRead {
                    from: position,
                    through: Some(info.size),
                    kinds: None,
                    limit: 1000,
                })
                .await
                .map_err(|_| failure())?;
            if page.items.is_empty() {
                return Err(failure());
            }
            for leaf in page.items {
                if leaf.position != position {
                    return Err(failure());
                }
                leaf.record::<18>().map_err(|_| failure())?;
                replay
                    .feed(doubleentry::log::ReplayLeaf {
                        position,
                        bytes: &leaf.bytes,
                        memo: None,
                        fingerprint: None,
                    })
                    .map_err(|_| failure())?;
                hash.update(position.to_be_bytes());
                hash.update((leaf.bytes.len() as u64).to_be_bytes());
                hash.update(&leaf.bytes);
                position = position.checked_add(1).ok_or_else(failure)?;
            }
        }
        let text = Checkpoint::new(KeyName::from(&id), replay.head()).text();
        Ok((
            NativeHead {
                semantics_version: 1,
                checkpoint: Digest(Sha256::digest(text.as_bytes()).into()),
            },
            Digest(hash.finalize().into()),
        ))
    }

    /// The pinned native audit checks every stored projection against canonical
    /// leaves. The separate digest includes every ordered leaf's exact bytes.
    pub async fn audit_history(&self) -> Result<(NativeHead, Digest), EngineError> {
        self.store.open().await.map_err(|_| failure())?;
        let id = LedgerId::new("currents/dataset-v1").map_err(|_| failure())?;
        let ledger = Ledger::<_, 18>::with_precision(self.store.clone(), id.clone())
            .await
            .map_err(|_| failure())?;
        let journal = ledger.to_journal().await.map_err(|_| failure())?;
        let mut leaves = Sha256::new();
        leaves.update(b"currents/native-leaves/v1\0");
        for leaf in journal.leaves() {
            leaves.update(leaf.position.to_be_bytes());
            leaves.update((leaf.bytes.len() as u64).to_be_bytes());
            leaves.update(leaf.bytes);
        }
        let text = Checkpoint::new(KeyName::from(&id), journal.head()).text();
        Ok((
            NativeHead {
                semantics_version: 1,
                checkpoint: Digest(Sha256::digest(text.as_bytes()).into()),
            },
            Digest(leaves.finalize().into()),
        ))
    }
}
