# Contributing to Currents

Public development uses synthetic data only. Read docs/constitution.md and
engineering-guide.md. No household or institutional credentials are needed.

Install Rust 1.99.0 with clippy/rustfmt and the separate candidate MSRV 1.94.0.
Run `python3 tools/install-ci-tools.py`, then set CURRENTS_TOOLS_BIN to the printed
absolute bin directory. CLI archives, including cargo-deny 0.20.2's official
release archives, are checked against tools/versions.lock.json.
Set CURRENTS_MSRV_BIN to the 1.94.0 bin directory. Gate commands are:

- `cargo xtask check quality`
- `cargo xtask check core`
- `cargo xtask check msrv`
- `cargo xtask check docs`
- `cargo xtask check privacy`
- `cargo xtask ticket CUR-003`

Quality checks formatting, warning-free compilation, Clippy, generated contracts,
workflow safety, doctests/rustdoc and dependency policy. Production library/binary
targets deny unwrap/expect/panic and unchecked arithmetic. Test assertions and
build-only xtask are excluded from those production-only lints, but remain under
all-target warnings-as-errors. Unsafe is forbidden by inherited workspace policy,
including types/domain/planning. Do not weaken these settings to pass a check.
Build-only xtask uses exact-pinned syn 3.0.10 to parse Rust attributes rather than
splitting source text; conditional/macro-generated exceptions are rejected.
Function-scoped exceptions require an explicit reason and matching reviewed entry
in docs/lint-allowlist.json; `expect` also fails when no longer needed.
Release overflow checks are enabled and panic unwinds.

Schema drift compares the generator output byte-for-byte. CUR008's authority
migration is verified by exact opened-schema attestation and its registered
qualification assertion. Public headers and root migrations remain absent; new
ones stay BLOCKED until their owner registers actual generation/validation.
This is not a claim of production native qualification.

Core and MSRV run the same tests as `cargo test --workspace`, except the
vendored doubleentry suite in `crates/currents-storage/tests/cur_005.rs`. Each
test runs in its own process on a pool sized to the machine's cores. Set
`CURRENTS_TEST_JOBS` to change the pool size; `1` runs one test at a time.
The pool starts the longest tests first (`START_FIRST` in
`xtask/src/test_pool.rs`), and runs `cargo test --doc` as one of its jobs.
`CURRENTS_TEST_SHARD=K/N` runs part K of N of that schedule; CI runs core and
MSRV as two parts on separate runners. In CI the core gate prints each test's
start time and duration. Nextest is not used.

The vendored suite's result changes only with `vendor/`, `patches/`,
`rust-toolchain.toml`, `.cargo/` or its test file. The CI `doubleentry` job
hashes those files with git, adds the OS and compiler, and looks for a recorded
pass under that key in the GitHub Actions cache. Only when it finds none does it
run `cargo xtask ticket CUR-005` and record a pass. It covers Linux and macOS
with 1.99.0 and Linux with 1.94.0. To force a rerun, change `doubleentry-v1` in
the key or delete the cache entry. Run the command locally before changing
those files.
In CI only the quality-policy job installs cargo-deny; other jobs install the
tools with `--without-cargo-deny`. That job leaves out the docs gate and the
CUR-001 ticket (`check quality-ci`), because the docs and core jobs run them on
every push; `check quality` still runs both.

## Required checks and repository settings

Desired required status names: `quality`, `core (ubuntu-24.04, 1/2)`,
`core (ubuntu-24.04, 2/2)`, `core (macos-15, 1/2)`, `core (macos-15, 2/2)`,
`msrv (1/2)`, `msrv (2/2)`, `docs`, `privacy`, `doubleentry (ubuntu-24.04, 1.99.0)`,
`doubleentry (macos-15, 1.99.0)`, `doubleentry (ubuntu-24.04, 1.94.0)`.
A separately authorized administrator should add these exact names to the main
branch ruleset, require pull requests and up-to-date passing checks, and require
maintainer review for CODEOWNERS paths. Verify names on a real PR before applying.
This change only documents desired rules; it does not apply settings or claim
branch protection is enabled. Hosted checks must execute; zero checks is not green.

CI is read-only on pull_request, push to main and workflow_dispatch, checks out
without persisted credentials, and uses only commit-pinned checkout and cache
actions. The cache holds only the `doubleentry` job's pass markers. CI has no
artifact uploads and no release/signing/OIDC secrets. Workflow JSON is the YAML
subset accepted by GitHub/actionlint and parsed without YAML ambiguities by the
local policy gate. Do not add a privileged trigger or secret-bearing job here.

## Dependency and policy updates

Review weekly Dependabot PRs; do not auto-merge. Verify official source/release,
checksum, minimal features, license and advisories; update exact inventory and
Cargo.lock together. Review native transitive sources, not just Rust licenses.
Run cargo-deny and both Rust/platform lanes plus affected golden/compatibility
suites. Urgent advisories get a separate reviewed fix. No blanket ignore or
unowned/indefinite exception is allowed. License policy is dependency compatibility
review, not selection of the application's final license. No release is authorized.
