//! The public fixture key is not suitable for private data.
use currents_cipher::{CipherProfileV1, DatasetKey};
use currents_storage::EncryptedStore;
use std::process::Command;
#[tokio::test]
async fn synthetic_headless_verification_preserves_source_and_rejects_collisions() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let store = EncryptedStore::open(&source, DatasetKey::new([0x61; 32]), CipherProfileV1)
        .await
        .unwrap();
    store.close().await.unwrap();
    let before = std::fs::read(&source).unwrap();
    let destination = dir.path().join("destination.db");
    let run = |destination: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_currents-verify"))
            .arg("--synthetic")
            .arg(&source)
            .arg(destination)
            .output()
            .unwrap()
    };
    let result = run(&destination);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("verified synthetic replay"));
    assert_eq!(std::fs::read(&source).unwrap(), before);
    let destination_before = std::fs::read(&destination).unwrap();
    assert!(!run(&destination).status.success());
    assert_eq!(std::fs::read(&destination).unwrap(), destination_before);
    assert!(!run(&source).status.success());
    #[cfg(unix)]
    {
        let link = dir.path().join("destination-link.db");
        std::os::unix::fs::symlink(&source, &link).unwrap();
        assert!(!run(&link).status.success());
        assert!(
            !Command::new(env!("CARGO_BIN_EXE_currents-verify"))
                .arg("--synthetic")
                .arg(&link)
                .arg(dir.path().join("from-link.db"))
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(!dir.path().join("from-link.db").exists());
    }
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_currents-verify"))
            .output()
            .unwrap()
            .status
            .success()
    );
}
