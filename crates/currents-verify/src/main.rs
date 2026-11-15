//! Offline synthetic-fixture verifier. The fixed PUBLIC key below is not a
//! private-data encryption facility. No production credential route exists.
use currents_cipher::{CipherProfileV1, DatasetKey};
use currents_service::ReplayCoordinator;
use currents_storage::{EncryptedStore, ReadOnlyStore};
use std::path::{Path, PathBuf};

fn paths(source: &Path, destination: &Path) -> Result<(PathBuf, PathBuf), ()> {
    let metadata = std::fs::symlink_metadata(source).map_err(|_| ())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(());
    }
    let source = source.canonicalize().map_err(|_| ())?;
    let name = destination.file_name().ok_or(())?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let destination = parent.canonicalize().map_err(|_| ())?.join(name);
    if source == destination || std::fs::symlink_metadata(&destination).is_ok() {
        return Err(());
    }
    // Reserve only our new pathname; never truncate a pre-existing destination.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|_| ())?;
    Ok((source, destination))
}
async fn run() -> Result<(), ()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 || args[0] != "--synthetic" {
        return Err(());
    }
    let (source, destination) = paths(Path::new(&args[1]), Path::new(&args[2]))?;
    let source = ReadOnlyStore::open(&source, DatasetKey::new([0x61; 32]), CipherProfileV1)
        .await
        .map_err(|_| ())?;
    let destination = match EncryptedStore::open(
        &destination,
        DatasetKey::new([0x61; 32]),
        CipherProfileV1,
    )
    .await
    {
        Ok(value) => value,
        Err(_) => {
            source.close().await;
            return Err(());
        }
    };
    let report = ReplayCoordinator::rebuild(source.history(), destination).await;
    source.close().await;
    let report = report.map_err(|_| ())?;
    if !report.problems.is_empty() {
        eprintln!("reconstructed synthetic destination; source trust failed");
        return Err(());
    }
    println!("verified synthetic replay {report:?}");
    Ok(())
}
#[tokio::main]
async fn main() {
    if run().await.is_err() {
        eprintln!("synthetic replay verification failed; no trusted activation");
        std::process::exit(1);
    }
}
