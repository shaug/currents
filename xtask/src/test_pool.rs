//! Runs every workspace test in its own process on a bounded pool.
//!
//! `cargo test` runs test binaries one after another. Running one test per
//! process instead lets independent tests use every core. It also keeps
//! process-global state apart: SQLCipher memory security, tracing subscribers
//! and exit-time native cleanup never cross from one test to another. In CI,
//! `CURRENTS_TEST_SHARD` splits the tests between several runners, and each
//! test must finish within its time limit from `docs/slow-tests.json`.
use crate::checks::{checked, output, recorded, root, sha};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;

struct Binary {
    executable: PathBuf,
    package_dir: PathBuf,
    /// Package directory and target name, stable across machines and builds.
    label: String,
}

struct Case<'a> {
    binary: &'a Binary,
    name: String,
}

/// One unit of pool work: the workspace doctests, or the test at an index.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Job {
    Doctests,
    Test(usize),
}

/// Test binaries that exercise only vendored code. Their result changes only
/// when the vendored sources, their patches, the toolchain or the test itself
/// change, so the CI `doubleentry` job runs them (`cargo xtask ticket CUR-005`)
/// only for inputs that have no recorded pass, instead of every core run.
const SEPARATE: &[&str] = &["currents-storage/cur_005"];

/// The reviewed list of tests that may take longer than the default CI time
/// limit, with their own limits, longest first. The pool starts these tests
/// first, in list order, so that none of them is left running alone at the end
/// and the shard split spreads them evenly.
const SLOW_TESTS: &str = "docs/slow-tests.json";

/// The doctests job's name in timing lines and in the slow-test list.
const DOCTESTS: &str = "doctests";

/// What a CI run says when a test runs past its limit.
const OVER_LIMIT: &str = "Make the test faster. Each service reopen replays the whole \
history, so reopen only when reopening is what the test checks, and do not start cargo \
from a test. If the test needs the time, add or raise its entry in docs/slow-tests.json \
with a reason; the maintainer reviews every change to that file. See AGENTS.md.";

/// CI time limits from `docs/slow-tests.json`.
#[derive(Debug, PartialEq)]
struct Limits {
    default: Duration,
    /// Listed tests (`package/target test_name`, or `doctests`) and their
    /// limits, in list order.
    listed: Vec<(String, Duration)>,
}

impl Limits {
    fn load() -> Result<Self> {
        let path = root().join(SLOW_TESTS);
        let text =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        Self::parse(&text)
    }

    fn parse(text: &str) -> Result<Self> {
        let bad = |why: &str| format!("{SLOW_TESTS}: {why}");
        let value: Value = serde_json::from_str(text).map_err(|error| bad(&error.to_string()))?;
        let seconds = |value: &Value| {
            value
                .as_u64()
                .filter(|seconds| *seconds > 0)
                .map(Duration::from_secs)
        };
        let default = seconds(&value["default_limit_seconds"])
            .ok_or_else(|| bad("default_limit_seconds must be a positive integer"))?;
        let entries = value["tests"]
            .as_array()
            .ok_or_else(|| bad("tests must be a list"))?;
        let mut listed: Vec<(String, Duration)> = Vec::new();
        for entry in entries {
            let test = entry["test"]
                .as_str()
                .filter(|test| !test.is_empty())
                .ok_or_else(|| bad("every entry needs a test"))?;
            let limit = seconds(&entry["limit_seconds"])
                .ok_or_else(|| bad(&format!("{test}: limit_seconds must be a positive integer")))?;
            if entry["reason"]
                .as_str()
                .is_none_or(|reason| reason.trim().is_empty())
            {
                return Err(bad(&format!("{test}: every entry needs a reason")));
            }
            if listed.iter().any(|(seen, _)| seen == test) {
                return Err(bad(&format!("{test} is listed twice")));
            }
            listed.push((test.to_owned(), limit));
        }
        Ok(Self { default, listed })
    }

    /// The test's position in the list, if it is listed.
    fn rank(&self, test: &str) -> Option<usize> {
        self.listed.iter().position(|(listed, _)| listed == test)
    }

    fn limit(&self, test: &str) -> Duration {
        self.listed
            .iter()
            .find(|(listed, _)| listed == test)
            .map_or(self.default, |(_, limit)| *limit)
    }

    /// Listed names that match no test in this run.
    fn missing<'a>(&'a self, tests: &BTreeSet<String>) -> Vec<&'a str> {
        self.listed
            .iter()
            .map(|(test, _)| test.as_str())
            .filter(|test| !tests.contains(*test))
            .collect()
    }
}

/// The part of the run that one CI runner takes: `index` of `count`, set by
/// `CURRENTS_TEST_SHARD=index/count`. Without it, one runner takes everything.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shard {
    index: usize,
    count: usize,
}

impl Shard {
    pub(crate) const ALL: Self = Self { index: 1, count: 1 };

    pub(crate) fn from_env() -> Result<Self> {
        match env::var("CURRENTS_TEST_SHARD") {
            Ok(value) => Self::parse(&value),
            Err(_) => Ok(Self::ALL),
        }
    }

    fn parse(value: &str) -> Result<Self> {
        value
            .split_once('/')
            .and_then(|(index, count)| {
                Some((index.parse::<usize>().ok()?, count.parse::<usize>().ok()?))
            })
            .filter(|(index, count)| (1..=*count).contains(index))
            .map(|(index, count)| Self { index, count })
            .ok_or_else(|| {
                format!("CURRENTS_TEST_SHARD must be INDEX/COUNT, 1 <= INDEX <= COUNT: {value}")
            })
    }

    /// The first shard also runs the checks that are not split.
    pub(crate) fn is_first(self) -> bool {
        self.index == 1
    }

    /// This shard's items. The first `head` items, the longest, are dealt back
    /// and forth (1, 2, 2, 1, ...) so that every shard gets a fair share of
    /// them. The rest are dealt in turn (1, 2, 1, 2, ...), so neighboring tests,
    /// which often take about as long as each other, go to different shards.
    fn take<T>(self, items: Vec<T>, head: usize) -> Vec<T> {
        let lanes = 2 * self.count;
        items
            .into_iter()
            .enumerate()
            .filter(|(position, _)| {
                let lane = if *position < head {
                    let turn = position % lanes;
                    if turn < self.count {
                        turn
                    } else {
                        lanes - 1 - turn
                    }
                } else {
                    (position - head) % self.count
                };
                lane + 1 == self.index
            })
            .map(|(_, item)| item)
            .collect()
    }
}

/// The name a test goes by in timing lines and in the slow-test list.
fn key(case: &Case) -> String {
    format!("{} {}", case.binary.label, case.name)
}

/// The run order, and how many listed jobs lead it: the listed tests and
/// doctests in list order, then the doctests if unlisted, then every other
/// test in listing order.
fn schedule(cases: &[Case], limits: &Limits) -> (Vec<Job>, usize) {
    let unlisted = limits.listed.len();
    let rank = |job: &Job| {
        let rank = match *job {
            Job::Doctests => limits.rank(DOCTESTS),
            Job::Test(index) => cases.get(index).and_then(|case| limits.rank(&key(case))),
        };
        rank.unwrap_or(unlisted)
    };
    let mut jobs: Vec<Job> = std::iter::once(Job::Doctests)
        .chain((0..cases.len()).map(Job::Test))
        .collect();
    // A stable sort keeps unlisted jobs in their original order.
    jobs.sort_by_key(rank);
    let head = jobs.iter().take_while(|job| rank(job) < unlisted).count();
    (jobs, head)
}

/// Worker count: `CURRENTS_TEST_JOBS` if set, else every available core.
fn jobs() -> Result<usize> {
    match env::var("CURRENTS_TEST_JOBS") {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .filter(|jobs| *jobs > 0)
            .ok_or_else(|| format!("CURRENTS_TEST_JOBS must be a positive integer: {value}")),
        Err(_) => Ok(thread::available_parallelism().map_or(1, usize::from)),
    }
}

/// The test executables `cargo test` would run, from Cargo's own build plan.
fn binaries(messages: &str) -> Result<Vec<Binary>> {
    let mut found = Vec::new();
    for line in messages.lines() {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if message["reason"] != "compiler-artifact" || message["profile"]["test"] != true {
            continue;
        }
        let Some(executable) = message["executable"].as_str() else {
            continue;
        };
        let manifest = message["manifest_path"]
            .as_str()
            .ok_or("test artifact without manifest path")?;
        let package_dir = Path::new(manifest)
            .parent()
            .ok_or("test manifest without parent")?;
        let package = package_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("test package directory without UTF-8 name")?;
        let target = message["target"]["name"]
            .as_str()
            .ok_or("test artifact without target name")?;
        found.push(Binary {
            executable: PathBuf::from(executable),
            package_dir: package_dir.to_path_buf(),
            label: format!("{package}/{target}"),
        });
    }
    if found.is_empty() {
        return Err("BLOCKED: cargo reported zero test executables".into());
    }
    Ok(found)
}

/// Best-effort diagnostics: a failed write must not change the result.
fn timing(label: &str, start: Duration, took: Duration) {
    let _ = writeln!(
        std::io::stderr().lock(),
        "core test start_ms={} duration_ms={} {label}",
        start.as_millis(),
        took.as_millis()
    );
}

/// Runs `work` on each item with `jobs` threads until `stop` is set, and hands
/// each result to `each` on the calling thread as soon as it finishes.
fn pool<T: Sync, R: Send>(
    items: &[T],
    jobs: usize,
    stop: &AtomicBool,
    work: impl Fn(&T) -> R + Sync,
    mut each: impl FnMut(usize, R),
) {
    let next = AtomicUsize::new(0);
    thread::scope(|scope| {
        // Rendezvous: a worker cannot start more work until its result is taken,
        // so a stop overruns by at most one item per worker.
        let (sender, receiver) = mpsc::sync_channel(0);
        for _ in 0..jobs.max(1) {
            let sender = sender.clone();
            let (next, work) = (&next, &work);
            scope.spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(item) = items.get(index) else {
                        break;
                    };
                    if sender.send((index, work(item))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        for (index, result) in receiver {
            each(index, result);
        }
    });
}

/// Builds (or reuses) the workspace test binaries and runs this shard's part of
/// the tests and doctests that `cargo test --workspace` runs, each test in its
/// own process. `SEPARATE` binaries are left out. With `timings` (CI only), it
/// prints when each test started and how long it took to stderr, which stays
/// out of the evidence, and fails when a test runs past its time limit.
pub(crate) fn run(
    program: &Path,
    rustc: Option<&Path>,
    manifest: Option<&str>,
    timings: bool,
    shard: Shard,
) -> Result<Value> {
    let mut plan = vec![
        "test",
        "--locked",
        "--workspace",
        "--no-run",
        "--message-format=json",
    ];
    let mut doc = vec!["test", "--locked", "--workspace", "--doc"];
    if let Some(path) = manifest {
        plan.extend(["--manifest-path", path]);
        doc.extend(["--manifest-path", path]);
    }
    let (separate, binaries): (Vec<_>, Vec<_>) =
        binaries(&checked(program, &plan, &root(), rustc)?)?
            .into_iter()
            .partition(|binary| SEPARATE.contains(&binary.label.as_str()));
    let workers = jobs()?;
    let limits = Limits::load()?;
    let never = AtomicBool::new(false);

    // Listing is cheap, but a first launch can be slow on macOS, so list in parallel.
    let list = ["--list"];
    let mut listings = Vec::new();
    listings.resize_with(binaries.len(), || None);
    pool(
        &binaries,
        workers,
        &never,
        |binary| output(&binary.executable, &list, &binary.package_dir, rustc),
        |index, listing| {
            if let Some(slot) = listings.get_mut(index) {
                *slot = Some(listing);
            }
        },
    );
    let mut cases = Vec::new();
    for (binary, listing) in binaries.iter().zip(listings) {
        let listing = listing.ok_or("test listing did not finish")??;
        let listing = recorded(
            &binary.executable,
            &list,
            &binary.package_dir,
            rustc,
            &listing,
        )?;
        cases.extend(
            listing
                .lines()
                .filter_map(|line| line.strip_suffix(": test"))
                .map(|name| Case {
                    binary,
                    name: name.to_owned(),
                }),
        );
    }
    if cases.is_empty() {
        return Err("BLOCKED: workspace test binaries listed zero tests".into());
    }
    // Every runner sorts the same way, so the shards split one schedule.
    cases.sort_by(|a, b| {
        (&a.binary.label, &a.name, &a.binary.executable).cmp(&(
            &b.binary.label,
            &b.name,
            &b.binary.executable,
        ))
    });
    // A fixture run lists another workspace's tests, so only the workspace run
    // checks that every listed name still exists.
    if manifest.is_none() {
        let tests: BTreeSet<String> = cases.iter().map(key).chain([DOCTESTS.to_owned()]).collect();
        let missing = limits.missing(&tests);
        if !missing.is_empty() {
            return Err(format!(
                "{SLOW_TESTS} lists tests that no longer exist; remove or rename them:\n  {}",
                missing.join("\n  ")
            ));
        }
    }
    let (schedule, head) = schedule(&cases, &limits);
    let work = shard.take(schedule, head);

    let stop = AtomicBool::new(false);
    let mut outputs = vec![String::new(); cases.len()];
    let mut doctests = None;
    let mut failures = Vec::new();
    let mut over_limit = Vec::new();
    let mut check_limit = |test: &str, took: Duration| {
        let limit = limits.limit(test);
        if timings && took > limit {
            over_limit.push(format!(
                "{test}: {}s, limit {}s",
                took.as_secs(),
                limit.as_secs()
            ));
        }
    };
    let args = |case: &Case| {
        [
            "--exact".to_owned(),
            case.name.clone(),
            "--test-threads=1".to_owned(),
        ]
    };
    let began = Instant::now();
    pool(
        &work,
        workers,
        &stop,
        |job| {
            let started = Instant::now();
            let result = match *job {
                Job::Doctests => output(program, &doc, &root(), rustc),
                Job::Test(index) => cases
                    .get(index)
                    .ok_or_else(|| "unknown test".to_owned())
                    .and_then(|case| {
                        let args = args(case);
                        let args: Vec<_> = args.iter().map(String::as_str).collect();
                        output(
                            &case.binary.executable,
                            &args,
                            &case.binary.package_dir,
                            rustc,
                        )
                    }),
            };
            (*job, started, started.elapsed(), result)
        },
        // Evidence is recorded here, on the reporting thread, as results arrive.
        |_, (job, started, took, result)| {
            let index = match job {
                Job::Doctests => {
                    if timings {
                        timing(DOCTESTS, started.duration_since(began), took);
                    }
                    check_limit(DOCTESTS, took);
                    match result.and_then(|out| recorded(program, &doc, &root(), rustc, &out)) {
                        Ok(stdout) => doctests = Some(stdout),
                        Err(error) => {
                            stop.store(true, Ordering::SeqCst);
                            failures.push(error);
                        }
                    }
                    return;
                }
                Job::Test(index) => index,
            };
            let Some(case) = cases.get(index) else {
                return;
            };
            let test = key(case);
            if timings {
                timing(&test, started.duration_since(began), took);
            }
            check_limit(&test, took);
            let args = args(case);
            let args: Vec<_> = args.iter().map(String::as_str).collect();
            let recorded = result.and_then(|out| {
                recorded(
                    &case.binary.executable,
                    &args,
                    &case.binary.package_dir,
                    rustc,
                    &out,
                )
            });
            match recorded {
                // `cargo test` also reports an #[ignore] test without running it.
                Ok(stdout)
                    if stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored;")
                        || stdout.contains("test result: ok. 0 passed; 0 failed; 1 ignored;") =>
                {
                    if let Some(slot) = outputs.get_mut(index) {
                        *slot = stdout;
                    }
                }
                Ok(stdout) => {
                    // Exactly one named test must run; never pass a silent skip.
                    stop.store(true, Ordering::SeqCst);
                    failures.push(format!("{} did not run exactly once:\n{stdout}", case.name));
                }
                Err(error) => {
                    // Like `cargo test`, stop starting new work after a failure.
                    stop.store(true, Ordering::SeqCst);
                    failures.push(error);
                }
            }
        },
    );
    if !failures.is_empty() {
        return Err(failures.join("\n"));
    }
    if !over_limit.is_empty() {
        return Err(format!(
            "These tests ran past their CI time limit:\n  {}\n{OVER_LIMIT}",
            over_limit.join("\n  ")
        ));
    }
    let doctests = match doctests {
        Some(stdout) => Some(json!({"command": doc, "stdout_sha256": sha(stdout.as_bytes())})),
        None if work.contains(&Job::Doctests) => return Err("doctests did not run".into()),
        // Another shard runs them.
        None => None,
    };

    let mut names: Vec<_> = cases.iter().map(key).collect();
    names.sort();
    let separate: Vec<_> = separate.iter().map(|binary| &binary.label).collect();
    Ok(json!({
        "command": plan,
        "binaries": binaries.len(),
        "separate": separate,
        "tests": cases.len(),
        "shard": format!("{}/{}", shard.index, shard.count),
        "shard_jobs": work.len(),
        "jobs": workers,
        "per_test_process": true,
        "test_inventory_sha256": sha(names.join("\n").as_bytes()),
        "stdout_sha256": sha(outputs.concat().as_bytes()),
        "doctests": doctests,
    }))
}

#[cfg(test)]
mod tests {
    use super::{Binary, Case, Job, Limits, Shard, binaries, pool, schedule};
    use std::{
        collections::BTreeSet,
        path::{Path, PathBuf},
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };

    fn limits(text: &str) -> Limits {
        Limits::parse(text).unwrap()
    }

    #[test]
    fn schedule_starts_listed_jobs_in_list_order() {
        let binary = Binary {
            executable: PathBuf::new(),
            package_dir: PathBuf::new(),
            label: "pkg/target".into(),
        };
        let case = |name: &str| Case {
            binary: &binary,
            name: name.into(),
        };
        let cases = [case("a"), case("slow"), case("b"), case("slowest")];
        let listed = limits(
            r#"{"default_limit_seconds": 30, "tests": [
                {"test": "pkg/target slowest", "limit_seconds": 90, "reason": "r"},
                {"test": "doctests", "limit_seconds": 60, "reason": "r"},
                {"test": "pkg/target slow", "limit_seconds": 45, "reason": "r"}]}"#,
        );
        assert_eq!(
            schedule(&cases, &listed),
            (
                vec![
                    Job::Test(3),
                    Job::Doctests,
                    Job::Test(1),
                    Job::Test(0),
                    Job::Test(2)
                ],
                3
            )
        );
        // Unlisted doctests run right after the listed tests.
        let tests_only = limits(
            r#"{"default_limit_seconds": 30, "tests": [
                {"test": "pkg/target b", "limit_seconds": 45, "reason": "r"}]}"#,
        );
        assert_eq!(
            schedule(&cases, &tests_only),
            (
                vec![
                    Job::Test(2),
                    Job::Doctests,
                    Job::Test(0),
                    Job::Test(1),
                    Job::Test(3)
                ],
                1
            )
        );
        let none = limits(r#"{"default_limit_seconds": 30, "tests": []}"#);
        assert_eq!(
            schedule(&cases[..1], &none),
            (vec![Job::Doctests, Job::Test(0)], 0)
        );
    }

    #[test]
    fn limits_apply_to_listed_tests_and_default_to_the_rest() {
        let listed = limits(
            r#"{"default_limit_seconds": 30, "tests": [
                {"test": "pkg/target slow", "limit_seconds": 90, "reason": "r"}]}"#,
        );
        assert_eq!(listed.limit("pkg/target slow"), Duration::from_secs(90));
        assert_eq!(listed.limit("pkg/target other"), Duration::from_secs(30));
        let tests: BTreeSet<String> = ["pkg/target other".to_owned()].into();
        assert_eq!(listed.missing(&tests), ["pkg/target slow"]);
        let tests: BTreeSet<String> = ["pkg/target slow".to_owned()].into();
        assert!(listed.missing(&tests).is_empty());
    }

    #[test]
    fn slow_test_list_needs_limits_reasons_and_unique_names() {
        for bad in [
            "not json",
            r#"{"tests": []}"#,
            r#"{"default_limit_seconds": 0, "tests": []}"#,
            r#"{"default_limit_seconds": 30}"#,
            r#"{"default_limit_seconds": 30, "tests": [{"limit_seconds": 9, "reason": "r"}]}"#,
            r#"{"default_limit_seconds": 30, "tests": [{"test": "t", "reason": "r"}]}"#,
            r#"{"default_limit_seconds": 30, "tests": [{"test": "t", "limit_seconds": 9}]}"#,
            r#"{"default_limit_seconds": 30, "tests": [{"test": "t", "limit_seconds": 9, "reason": " "}]}"#,
            r#"{"default_limit_seconds": 30, "tests": [
                {"test": "t", "limit_seconds": 9, "reason": "r"},
                {"test": "t", "limit_seconds": 9, "reason": "r"}]}"#,
        ] {
            assert!(Limits::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn checked_in_slow_test_list_is_valid() {
        let text = std::fs::read_to_string(super::root().join(super::SLOW_TESTS)).unwrap();
        Limits::parse(&text).unwrap();
    }

    #[test]
    fn shards_split_every_job_exactly_once() {
        let items: Vec<usize> = (0..11).collect();
        for count in 1..=3 {
            for head in [0, 4, 11] {
                let mut seen: Vec<usize> = (1..=count)
                    .flat_map(|index| Shard { index, count }.take(items.clone(), head))
                    .collect();
                seen.sort_unstable();
                assert_eq!(seen, items, "{count} shards, head {head}");
            }
        }
        // The head goes back and forth, so the two longest go to different
        // shards; the rest alternate, so neighbors never share a shard.
        let first = Shard { index: 1, count: 2 };
        let second = Shard { index: 2, count: 2 };
        assert_eq!(first.take(items.clone(), 4), [0, 3, 4, 6, 8, 10]);
        assert_eq!(second.take(items.clone(), 4), [1, 2, 5, 7, 9]);
        assert_eq!(Shard::ALL.take(items.clone(), 4), items);
    }

    #[test]
    fn shard_must_name_an_index_within_its_count() {
        assert_eq!(Shard::parse("2/2"), Ok(Shard { index: 2, count: 2 }));
        assert_eq!(Shard::parse("1/1"), Ok(Shard::ALL));
        for bad in ["0/2", "3/2", "1/0", "2", "a/2", "1/2/3", ""] {
            assert!(Shard::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn build_plan_keeps_only_test_executables() {
        let messages = [
            r#"{"reason":"compiler-artifact","profile":{"test":true},"executable":"/t/deps/cur_1-ab","manifest_path":"/w/crates/a/Cargo.toml","target":{"name":"cur_1"}}"#,
            r#"{"reason":"compiler-artifact","profile":{"test":false},"executable":"/t/debug/a","manifest_path":"/w/crates/a/Cargo.toml","target":{"name":"a"}}"#,
            r#"{"reason":"compiler-artifact","profile":{"test":true},"executable":null,"manifest_path":"/w/crates/a/Cargo.toml","target":{"name":"a"}}"#,
            r#"{"reason":"build-finished","success":true}"#,
            "not json",
        ]
        .join("\n");
        let found = binaries(&messages).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "a/cur_1");
        assert_eq!(found[0].package_dir, Path::new("/w/crates/a"));
        assert!(binaries("").is_err(), "zero executables must not pass");
    }

    #[test]
    fn pool_reports_every_result_and_stops_after_a_failure() {
        let items: Vec<usize> = (0..20).collect();
        let never = AtomicBool::new(false);
        let mut seen = vec![false; items.len()];
        pool(
            &items,
            4,
            &never,
            |n| n * 2,
            |index, doubled| {
                assert_eq!(doubled, index * 2);
                seen[index] = true;
            },
        );
        assert!(seen.iter().all(|done| *done));

        let stop = AtomicBool::new(false);
        let mut ran = 0;
        pool(
            &items,
            1,
            &stop,
            |n| *n,
            |_, n| {
                ran += 1;
                if n == 2 {
                    stop.store(true, Ordering::SeqCst);
                }
            },
        );
        // One worker may finish the item it already took, never the rest.
        assert!((3..=4).contains(&ran), "ran {ran}");
    }
}
