//! The driver suite proves native joining. This independently proves the private
//! cipher boundary actually opts into it, using its real two-second deadline.
use super::*;
use sqlx::{Connection, Executor};
use std::{
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Instant,
};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

#[test]
fn failed_open_waits_for_locked_initializer() {
    const CHILD: &str = "CURRENTS_CIPHER_SHUTDOWN_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::failed_open_waits_for_locked_initializer",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "private cipher shutdown regression failed"
                );
                return;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("private cipher native shutdown watchdog expired");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    let path = std::env::temp_dir().join(format!(
        "currents-cipher-shutdown-{}.db",
        std::process::id()
    ));
    let release_started = Arc::new(AtomicBool::new(false));
    let controller_path = path.clone();
    let controller_released = release_started.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let controller = thread::spawn(move || {
        runtime().block_on(async {
            let key = Arc::new(Mutex::new(Some(DatasetKey::new([19; 32]))));
            let mut conn = connection_options(&controller_path).connect().await.unwrap();
            initialize(&mut conn, &key, CipherProfileV1).await.unwrap();
            conn.execute("CREATE TABLE payload(value INTEGER); INSERT INTO payload VALUES(7); PRAGMA journal_mode=DELETE;")
                .await.unwrap();
            let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
                .fetch_one(&mut conn).await.unwrap();
            assert_eq!(journal, "delete");
            conn.execute("BEGIN EXCLUSIVE").await.unwrap();
            ready_tx.send(()).unwrap();
            // Independent native controller: the dropping Tokio thread may be blocked.
            thread::sleep(Duration::from_secs(3));
            controller_released.store(true, Ordering::Release);
            conn.execute("ROLLBACK").await.unwrap();
            conn.close().await.unwrap();
        });
    });
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let rt = runtime();
    rt.block_on(async {
        let result = open_pool(&path, DatasetKey::new([19; 32]), CipherProfileV1, 1).await;
        assert!(
            result.is_err(),
            "fixture must exceed the real acquisition deadline"
        );
        assert!(
            release_started.load(Ordering::Acquire),
            "open_pool returned while native initialization was still locked"
        );
    });
    drop(rt);
    controller.join().unwrap();
    runtime().block_on(async {
        let pool = open_pool(&path, DatasetKey::new([19; 32]), CipherProfileV1, 1)
            .await
            .unwrap();
        let value: i64 = sqlx::query_scalar("SELECT value FROM payload")
            .fetch_one(pool.pool())
            .await
            .unwrap();
        assert_eq!(value, 7);
        pool.close().await;
    });
    std::fs::remove_file(path).unwrap();
}
