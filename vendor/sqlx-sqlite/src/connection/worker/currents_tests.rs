//! Lifecycle probes exist only in this crate's unit-test build. They observe real
//! workers, SQLCipher authentication and SQLite locking, not fake connections.
use super::*;
use crate::{Sqlite, SqliteConnectOptions};
use futures_util::{FutureExt, StreamExt};
use sqlx_core::{
    connection::{ConnectOptions, Connection},
    executor::Executor,
    pool::PoolOptions,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Condvar, Mutex as StdMutex, OnceLock, Weak},
    time::{Duration, Instant},
};

pub(super) const AUTH_SQL: &str = "SELECT count(*) FROM sqlite_schema";
const KEY_SQL: &str =
    "PRAGMA key = \"x'1111111111111111111111111111111111111111111111111111111111111111'\"";
const WRONG_KEY_SQL: &str =
    "PRAGMA key = \"x'2222222222222222222222222222222222222222222222222222222222222222'\"";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Event {
    Started,
    BeforePublish,
    BeforeShutdown,
    AuthExecute,
    AtExit,
    Joining,
    Joined,
    Returned,
}
#[derive(Default)]
struct State {
    events: Vec<Event>,
    gate: Option<Event>,
    released: bool,
}
#[derive(Default)]
pub(super) struct Probe {
    state: StdMutex<State>,
    changed: Condvar,
}
impl Probe {
    fn record(&self, event: Event) {
        let mut state = self.state.lock().unwrap();
        state.events.push(event);
        self.changed.notify_all();
        while state.gate == Some(event) && !state.released {
            state = self.changed.wait(state).unwrap();
        }
    }
    fn wait(&self, predicate: impl Fn(&[Event]) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut state = self.state.lock().unwrap();
        while !predicate(&state.events) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "worker event watchdog: {:?}",
                state.events
            );
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }
    fn release(&self) {
        self.state.lock().unwrap().released = true;
        self.changed.notify_all();
    }
    fn count(&self, event: Event) -> usize {
        self.state
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|&&e| e == event)
            .count()
    }
    fn drained(&self) {
        let events = self.state.lock().unwrap().events.clone();
        let count = |event| events.iter().filter(|&&e| e == event).count();
        assert!(count(Event::Started) > 0, "test must create real workers");
        assert_eq!(
            count(Event::Joined),
            count(Event::Started),
            "worker survived failure/close boundary: {:?}",
            events
        );
        assert_eq!(count(Event::AtExit), count(Event::Started));
    }
}
fn registry() -> &'static StdMutex<HashMap<String, Weak<Probe>>> {
    static REGISTRY: OnceLock<StdMutex<HashMap<String, Weak<Probe>>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
pub(super) fn lookup(name: &str) -> Option<Arc<Probe>> {
    registry().lock().unwrap().get(name).and_then(Weak::upgrade)
}
pub(super) struct ExitProbe(Option<Arc<Probe>>);
impl Drop for ExitProbe {
    fn drop(&mut self) {
        if let Some(probe) = &self.0 {
            probe.record(Event::AtExit);
        }
    }
}
pub(super) fn started(name: &str) -> ExitProbe {
    let probe = lookup(name);
    if let Some(probe) = &probe {
        probe.record(Event::Started);
    }
    ExitProbe(probe)
}
pub(super) fn event(event: Event) {
    if let Some(probe) = thread::current().name().and_then(lookup) {
        probe.record(event);
    }
}
pub(super) fn joining(probe: &Option<Arc<Probe>>) {
    if let Some(probe) = probe {
        probe.record(Event::Joining);
    }
}
pub(super) fn joined(probe: &Option<Arc<Probe>>) {
    if let Some(probe) = probe {
        probe.record(Event::Joined);
    }
}
fn options(path: &Path, probe: &Arc<Probe>) -> SqliteConnectOptions {
    let probe = Arc::downgrade(probe);
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .disable_statement_logging()
        .statement_cache_capacity(0)
        .join_worker_on_drop(true)
        .busy_timeout(Duration::from_secs(5))
        .thread_name(move |id| {
            let name = format!("currents-lifecycle-{}-{id}", std::process::id());
            registry()
                .lock()
                .unwrap()
                .insert(name.clone(), probe.clone());
            name
        })
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}
fn fixture_path(scenario: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "currents-worker-{}-{scenario}.db",
        std::process::id()
    ))
}
async fn authenticate(connection: &mut crate::SqliteConnection, wrong: bool) -> Result<(), Error> {
    connection
        .execute(if wrong { WRONG_KEY_SQL } else { KEY_SQL })
        .await?;
    let version: String = sqlx_core::query_scalar::query_scalar("PRAGMA cipher_version")
        .fetch_one(&mut *connection)
        .await?;
    assert!(
        version.starts_with("4.20.0"),
        "tests must use pinned SQLCipher"
    );
    connection.execute(AUTH_SQL).await?;
    Ok(())
}
async fn fixture(path: &Path) {
    let mut conn = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .connect()
        .await
        .unwrap();
    authenticate(&mut conn, false).await.unwrap();
    conn.execute("PRAGMA journal_mode=DELETE; CREATE TABLE payload(value INTEGER); INSERT INTO payload VALUES(7)").await.unwrap();
    conn.close().await.unwrap();
    assert_ne!(&std::fs::read(path).unwrap()[..16], b"SQLite format 3\0");
}
async fn reopen(path: &Path) {
    let mut conn = SqliteConnectOptions::new()
        .filename(path)
        .connect()
        .await
        .unwrap();
    authenticate(&mut conn, false).await.unwrap();
    let value: i64 = sqlx_core::query_scalar::query_scalar("SELECT value FROM payload")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(value, 7);
    conn.close().await.unwrap();
}
async fn wait_event(probe: &Probe, event: Event) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while probe.count(event) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("worker reached expected real operation");
}
fn release_on_join(probe: &Arc<Probe>) -> thread::JoinHandle<()> {
    let probe = probe.clone();
    thread::spawn(move || {
        probe.wait(|events| events.contains(&Event::Joining) || events.contains(&Event::Returned));
        probe.release();
    })
}
fn lock_until_join(path: &Path, probe: &Arc<Probe>) -> thread::JoinHandle<()> {
    let path = path.to_owned();
    let probe = probe.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let controller = thread::spawn(move || {
        runtime().block_on(async {
            let mut conn = SqliteConnectOptions::new()
                .filename(path)
                .connect()
                .await
                .unwrap();
            authenticate(&mut conn, false).await.unwrap();
            let mode: String = sqlx_core::query_scalar::query_scalar("PRAGMA journal_mode")
                .fetch_one(&mut conn)
                .await
                .unwrap();
            assert_eq!(mode, "delete", "WAL does not block this schema reader");
            conn.execute("BEGIN EXCLUSIVE").await.unwrap();
            ready_tx.send(()).unwrap();
            probe.wait(|events| {
                events.contains(&Event::Joining) || events.contains(&Event::Returned)
            });
            conn.execute("ROLLBACK").await.unwrap();
            conn.close().await.unwrap();
            probe.release();
        });
    });
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    controller
}
async fn failure(scenario: &str, path: &Path, probe: &Arc<Probe>) {
    fixture(path).await;
    let original = std::fs::read(path).unwrap();
    let wrong = scenario.contains("rejected");
    let pool = PoolOptions::<Sqlite>::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(Duration::from_millis(150))
        .after_connect(move |conn, _| Box::pin(authenticate(conn, wrong)))
        .connect_lazy_with(options(path, probe));
    if scenario.contains("replacement") {
        // Force the next acquisition to establish a replacement, not reuse an idle slot.
        pool.acquire().await.unwrap().close().await.unwrap();
        probe.wait(|events| events.contains(&Event::AtExit));
        probe.state.lock().unwrap().events.clear();
    }
    probe.state.lock().unwrap().gate = Some(Event::AtExit);
    let controller = if wrong {
        release_on_join(probe)
    } else {
        lock_until_join(path, probe)
    };
    let start = Instant::now();
    if scenario.contains("cancel") {
        let mut acquire = Box::pin(pool.acquire());
        tokio::select! {
            result = &mut acquire => panic!("acquire unexpectedly completed: {result:?}"),
            _ = wait_event(probe, Event::AuthExecute) => {}
        }
        drop(acquire);
    } else {
        assert!(matches!(pool.acquire().await, Err(Error::PoolTimedOut)));
        assert!(start.elapsed() >= Duration::from_millis(150));
    }
    probe.record(Event::Returned);
    // Before pool.close, runtime teardown, waiting for the controller, or reopening.
    probe.drained();
    pool.close().await;
    controller.join().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), original);
    reopen(path).await;
}
async fn startup(probe: &Arc<Probe>) {
    probe.state.lock().unwrap().gate = Some(Event::BeforePublish);
    let controller = release_on_join(probe);
    let opts = options(Path::new(":memory:"), probe);
    let mut connect = Box::pin(opts.connect());
    tokio::select! {
        result = &mut connect => panic!("connect unexpectedly completed: {result:?}"),
        _ = wait_event(probe, Event::BeforePublish) => {}
    }
    drop(connect);
    probe.record(Event::Returned);
    probe.drained();
    controller.join().unwrap();
}
async fn backpressure(probe: &Arc<Probe>) {
    let mut conn = options(Path::new(":memory:"), probe)
        .row_buffer_size(1)
        .connect()
        .await
        .unwrap();
    let mut stream = conn.fetch("WITH RECURSIVE seq(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM seq WHERE x<10000) SELECT x FROM seq");
    stream.next().await.unwrap().unwrap();
    drop(stream);
    drop(conn);
    probe.record(Event::Returned);
    probe.drained();
}
async fn close_cancel(probe: &Arc<Probe>) {
    let conn = options(Path::new(":memory:"), probe)
        .connect()
        .await
        .unwrap();
    probe.state.lock().unwrap().gate = Some(Event::BeforeShutdown);
    let controller = release_on_join(probe);
    let mut close = Box::pin(conn.close());
    // Force close to suspend after enqueueing Shutdown but before its native
    // acknowledgement. Dropping this pending future must retain the JoinHandle.
    assert!((&mut close).now_or_never().is_none());
    wait_event(probe, Event::BeforeShutdown).await;
    drop(close);
    probe.record(Event::Returned);
    probe.drained();
    controller.join().unwrap();
}

fn run(scenario: &str) {
    // A native join blocks the Tokio thread. Only an OS-process watchdog can
    // catch wrong drop order or backpressure deadlocks on a current-thread runtime.
    if std::env::var("CURRENTS_WORKER_CHILD").ok().as_deref() != Some(scenario) {
        let name = format!("connection::worker::currents_tests::{scenario}");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &name, "--nocapture"])
            .env("CURRENTS_WORKER_CHILD", scenario)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "{scenario}: child failed");
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("{scenario}: native shutdown watchdog expired");
            }
            thread::sleep(Duration::from_millis(10));
        }
        return;
    }
    let probe = Arc::new(Probe::default());
    let path = fixture_path(scenario);
    let rt = runtime();
    rt.block_on(async {
        match scenario {
            "startup_cancel_joins" => startup(&probe).await,
            "row_backpressure_drop_joins" => backpressure(&probe).await,
            "close_cancel_joins" => close_cancel(&probe).await,
            _ => failure(scenario, &path, &probe).await,
        }
    });
    drop(rt); // Immediate runtime teardown must not own any stranded native worker.
    probe.drained();
    if path.exists() {
        runtime().block_on(reopen(&path));
        std::fs::remove_file(path).unwrap();
    }
}
#[test]
fn startup_cancel_joins() {
    run("startup_cancel_joins");
}
#[test]
fn rejected_key_joins() {
    run("rejected_key_joins");
}
#[test]
fn locked_initial_timeout_joins() {
    run("locked_initial_timeout_joins");
}
#[test]
fn locked_replacement_timeout_joins() {
    run("locked_replacement_timeout_joins");
}
#[test]
fn locked_caller_cancel_joins() {
    run("locked_caller_cancel_joins");
}
#[test]
fn row_backpressure_drop_joins() {
    run("row_backpressure_drop_joins");
}
#[test]
fn close_cancel_joins() {
    run("close_cancel_joins");
}
