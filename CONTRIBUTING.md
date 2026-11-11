# Contributing to Currents

Public development uses synthetic data only. Read docs/constitution.md and
engineering-guide.md. No household or institutional credentials are needed.

Install Rust 1.99.0 with clippy/rustfmt and the separate candidate MSRV 1.94.0.
Run `python3 tools/install-ci-tools.py`, then set CURRENTS_TOOLS_BIN to the printed
absolute bin directory. CLI archives are checked against tools/versions.lock.json;
cargo-deny 0.20.2 is installed locked from the official crates.io registry.
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
Nextest is not used; cargo test --doc remains explicit even if introduced later.

## Required checks and repository settings

Desired required status names: `quality`, `core (ubuntu-24.04)`,
`core (macos-15)`, `msrv`, `docs`, `privacy`.
A separately authorized administrator should add these exact names to the main
branch ruleset, require pull requests and up-to-date passing checks, and require
maintainer review for CODEOWNERS paths. Verify names on a real PR before applying.
This change only documents desired rules; it does not apply settings or claim
branch protection is enabled. Hosted checks must execute; zero checks is not green.

CI is read-only on pull_request, push to main and workflow_dispatch, checks out
without persisted credentials, uses one commit-pinned checkout action, has no
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
