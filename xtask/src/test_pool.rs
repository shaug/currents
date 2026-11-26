//! Runs every workspace test in its own process on a bounded pool.
//!
//! `cargo test` runs test binaries one after another. Running one test per
//! process instead lets independent tests use every core. It also keeps
//! process-global state apart: SQLCipher memory security, tracing subscribers
//! and exit-time native cleanup never cross from one test to another. In CI,
//! `CURRENTS_TEST_SHARD` splits the tests between several runners.
use crate::checks::{checked, output, recorded, root, sha};
use serde_json::{Value, json};
use std::{
    env,
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

/// The tests that take longest in CI, longest first. They start first so that
/// none of them is left running alone at the end, and the shard split spreads
/// them evenly. The list only changes the order; a name that no longer exists
/// changes nothing.
const START_FIRST: &[&str] = &[
    "prefix_visibility",
    "cur_025_service_durable_revisions_and_no_recognition",
    "cur_015_opening_boundary_encrypted_initial_and_remap",
    "cur_015_1_separate_arrival_permutations_rebuild_same_economic_postings",
    "history::audit_parity_tests::combined_decisions_count_boundary_and_rehashed_overflow",
    "cur_007_1",
    "cur_010_1",
    "cur_014_2_slice_fault",
    "cur_007_3",
    "cur_015_3_real_parser_remap_preserves_raw_cancellation_retry_and_reopen",
];

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

    /// This shard's items, dealt back and forth (1, 2, 2, 1, 1, 2, ...) so that
    /// every shard gets a fair share of the long tests at the front.
    fn take<T>(self, items: Vec<T>) -> Vec<T> {
        let lanes = 2 * self.count;
        items
            .into_iter()
            .enumerate()
            .filter(|(position, _)| {
                let turn = position % lanes;
                let lane = if turn < self.count {
                    turn
                } else {
                    lanes - 1 - turn
                };
                lane + 1 == self.index
            })
            .map(|(_, item)| item)
            .collect()
    }
}

/// The run order: the longest tests, then the doctests, then every other test
/// in listing order.
fn schedule(cases: &[Case]) -> Vec<Job> {
    let rank = |index: &usize| {
        cases
            .get(*index)
            .and_then(|case| START_FIRST.iter().position(|name| *name == case.name))
            .unwrap_or(START_FIRST.len())
    };
    let mut tests: Vec<usize> = (0..cases.len()).collect();
    tests.sort_by_key(rank);
    let first = tests
        .iter()
        .take_while(|index| rank(index) < START_FIRST.len())
        .count();
    let mut jobs: Vec<Job> = tests.iter().copied().map(Job::Test).collect();
    jobs.insert(first, Job::Doctests);
    jobs
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
/// own process. `SEPARATE` binaries are left out. With `timings`, it prints
/// when each test started and how long it took to stderr, which stays out of
/// the evidence.
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
    let work = shard.take(schedule(&cases));

    let stop = AtomicBool::new(false);
    let mut outputs = vec![String::new(); cases.len()];
    let mut doctests = None;
    let mut failures = Vec::new();
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
                        timing("doctests", started.duration_since(began), took);
                    }
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
            if timings {
                timing(
                    &format!("{} {}", case.binary.label, case.name),
                    started.duration_since(began),
                    took,
                );
            }
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
    let doctests = match doctests {
        Some(stdout) => Some(json!({"command": doc, "stdout_sha256": sha(stdout.as_bytes())})),
        None if work.contains(&Job::Doctests) => return Err("doctests did not run".into()),
        // Another shard runs them.
        None => None,
    };

    let mut names: Vec<_> = cases
        .iter()
        .map(|case| format!("{} {}", case.binary.label, case.name))
        .collect();
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
    use super::{Binary, Case, Job, START_FIRST, Shard, binaries, pool, schedule};
    use std::{
        path::{Path, PathBuf},
        sync::atomic::{AtomicBool, Ordering},
    };

    #[test]
    fn schedule_starts_long_tests_then_doctests() {
        let binary = Binary {
            executable: PathBuf::new(),
            package_dir: PathBuf::new(),
            label: "pkg/target".into(),
        };
        let case = |name: &str| Case {
            binary: &binary,
            name: name.into(),
        };
        let (Some(first), Some(second)) = (START_FIRST.first(), START_FIRST.get(1)) else {
            panic!("START_FIRST lists at least two tests");
        };
        let cases = [case("a"), case(second), case("b"), case(first)];
        assert_eq!(
            schedule(&cases),
            [
                Job::Test(3),
                Job::Test(1),
                Job::Doctests,
                Job::Test(0),
                Job::Test(2)
            ]
        );
        assert_eq!(schedule(&cases[..1]), [Job::Doctests, Job::Test(0)]);
    }

    #[test]
    fn shards_split_every_job_exactly_once() {
        let items: Vec<usize> = (0..11).collect();
        for count in 1..=3 {
            let mut seen: Vec<usize> = (1..=count)
                .flat_map(|index| Shard { index, count }.take(items.clone()))
                .collect();
            seen.sort_unstable();
            assert_eq!(seen, items, "{count} shards");
        }
        // Dealt back and forth, so the two longest tests go to different shards.
        assert_eq!(
            Shard { index: 1, count: 2 }.take(items.clone()),
            [0, 3, 4, 7, 8]
        );
        assert_eq!(
            Shard { index: 2, count: 2 }.take(items.clone()),
            [1, 2, 5, 6, 9, 10]
        );
        assert_eq!(Shard::ALL.take(items.clone()), items);
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
