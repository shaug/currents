# Workspace and dependency qualification

CUR-001 establishes a synthetic-only Rust workspace. All 15 members are private.
It adds no financial behavior, native owner, signing, release workflow or credential
access. The upstream accounting kernel is resolved at the design's immutable
commit; its private transaction/key-initialization patches remain the owning
accounting/storage tickets' work. A successful build does not qualify encryption.

## Reproduce the local checks

Use the exact Rust 1.99.0 toolchain with rustfmt and Clippy. Install the official
Rust 1.94.0 rustc, cargo and standard library components into a separate prefix,
verifying each archive against the distribution manifest recorded in
`tools/versions.lock.json`. Set `CURRENTS_MSRV_BIN` to that prefix's `bin` directory.
This explicit path does not change a system developer directory or default Rust.

Run from the repository root:

```sh
cargo generate-lockfile # only when deliberately updating the reviewed lock
cargo xtask ticket CUR-001
cargo xtask check quality
cargo xtask check msrv
cargo xtask qualify workspace
cargo xtask qualify dependency-direction
cargo tree --locked -i libsqlite3-sys
```

The checked-in alias always runs xtask with `--locked`. `check core` builds all
workspace targets and runs the workspace tests; `check msrv` does the same with
the explicitly verified 1.94.0 compiler. The probe used by `cur_001_3` verifies
compiler identity only; the separate MSRV lane is mandatory acceptance evidence.
The same `qualify msrv --manifest-path <fixture/Cargo.toml>` build/report path
accepts a synthetic fixture manifest. `cur_001_3` generates an offline lockfile
for a local dependency requiring Rust 1.99, then requires Rust 1.94 qualification
to exit nonzero with `Blocked` and the exact package/compiler diagnostic. This
counterexample does not replace the positive full selected-feature MSRV lane.
No `rust-version` promise is made merely because dependency resolution succeeds.
No indiscriminate `--all-features` invocation enables PostgreSQL/MySQL.

`ticket CUR-001` first verifies the exact four assertion names, then requires
four passed, zero failed and zero ignored tests. An unknown ticket or missing
suite fails. Native/security/deep/release/all and production/private-activation
gates remain explicitly BLOCKED until their owning tickets register the required
suites. They never count an absent suite or report as passed.

Every gate emits C16 `QualificationReportV1`, checked against the required fields
in `docs/qualification-report.schema.json`. `gate`, `inputs`, `platform`,
`versions`, `tests`, `evidence`, `status` and `limitations` are always present.
Inputs bind command, Git head, dirty state, generation time and source/lock/pin
file digests. Version evidence names executed compiler/Cargo/C tool probes;
commands retain exact arguments, selected compiler, exit and output digests.
Executed test functions and outcomes are enumerated from test-run output, not
inferred from a registry. An empty test list means that command ran no tests.
Reports and full subprocess stdout/stderr are retained in `target/qualification/`.
C16 statuses are preserved: successful checks use `Passed`; failed assertions
and unavailable prerequisites both use `Blocked`, distinguished by
`evidence.error`. Malformed options also produce a blocked report.
Dirty-tree results are preliminary and must be rerun at the final committed
candidate. A pass certifies only the named gate on the recorded host, never
another platform or the complete product. Compiler probes and source hashing
are not build qualification. Provider overrides, including `OPENSSL_NO_VENDOR`
and its target-prefixed forms, are rejected before core/MSRV builds.

## Source pins and cache verification

`tools/versions.lock.json` records official registry archive checksums, source
URLs, licenses, Rust requirements and enabled features; Cargo.lock locks the
complete resolved graph. Jiff 0.2.41/jiff-tzdb 0.1.9 contains timezone data 2026e.
Two manifest corrections were verified directly against published packages:
sha2 0.11.1 has no `std` feature, and secrecy 0.10.3 has no `std`/`alloc` feature.
Both therefore use their supported default-free configuration. Reqwest 0.13.5
uses only `rustls,stream`; Jiff disables ambient system timezone discovery.

Obtain every `artifacts` entry only from its exact `source`, save it as `path` in
one local cache, and verify SHA-256 against `sha256`. Then run:

```sh
cargo xtask qualify pin-inventory --artifact-root /absolute/path/to/source-cache
```

The verifier never downloads, repairs or substitutes a missing artifact and never
chooses `latest`. It rejects absent artifacts, corrupted bytes, duplicate names,
empty inventories, nonexact version selectors and malformed digests. The source
inventory includes tool source archives and the two official Rust distribution
manifests; it is not a claim that every optional tool was installed or executed.
Each Rust manifest pins the platform component archive checksums. Gitleaks and
actionlint archives are separately pinned for macOS arm64 and Linux x86_64.
No GitHub Actions are introduced here; CUR-003 owns workflow commits and runner
selection. Native Xcode/compiler qualification belongs to CUR-004. Final app
license selection remains an explicit release-owner decision.

`qualify dependency-direction --manifest-path /path/to/Cargo.toml` audits a
synthetic workspace with the same direct dependency rules. Tests add both a
network crate and Apple-framework crate to a domain fixture and require rejection.
Normal, build and target-specific direct dependencies are checked; dev dependencies
are excluded. The allowed pure-crate library set is explicit. Future library
additions require a reviewed rule update, not a broad network/platform exemption.

The SQLx family is pinned to 0.9.0 and libsqlite3-sys to 0.37.0. Mac builds use
bundled SQLCipher with system Apple crypto; Linux builds select bundled SQLCipher
with vendored OpenSSL. Build lanes reject known SQLite/OpenSSL provider environment
overrides instead of silently accepting a different native library. The selected
SQLCipher amalgamation hash is
`8cd2347d18886fee4651153fabeeeffb7cec008700987dab1c00261febc4220f`.
Final runtime link/key-profile qualification remains CUR-006.

## Qualification evidence, 2026-11-09

The four public command-contract assertions fail against documentation-only base
c34aa3d5ab463652dbf14d8864b8e60911213974 because xtask is absent. Actual macOS arm64
and Linux x86_64 current-toolchain/core and Rust 1.94 build/test executions are
recorded for the source-refreshed candidate. Reports bind each execution to its
exact Git head; earlier-head or dirty-tree reports do not certify later commits.
Re-run the required checks after every change, including the incompatible-package
counterexample. Source-cache, frozen/offline build, advisory/license inventory,
secret scan and unsigned reproducibility evidence are retained separately.
Independent review and exact final-head acceptance remain publication gates.

### Historical vulnerability and reviewed-source candidate

Initial snapshot 2ca9bec used SQLCipher with SQLite 3.50.4, affected by
RUSTSEC-2026-0459 (CVE-2026-119375). An attacker-controlled database and sibling
WAL with different page sizes could disclose uninitialized page-cache bytes.
An encrypted-path exploit was not reproduced; the source-level missing tail
clearing still blocked that snapshot. The advisory also identifies affected
SQLite 3.53.4 inside libsqlite3-sys 0.39.0/0.39.1, so changing only the Rust
package version would not establish remediation.

Evidence: [RustSec advisory at immutable database revision](https://github.com/RustSec/advisory-db/blob/d9cb059f3217db87aeccd190b81eaffe8cc29cb9/crates/libsqlite3-sys/RUSTSEC-2026-0459.md).

The current candidate uses official SQLCipher 4.20.0/SQLite 3.54.1, source commit
355d4db92d3404e71faa8088d071a71d767807f4, with the upstream WAL tail-clearing fix.
The private sys 0.37 build consumes exact registry sqlcipher-src 420.0.0 without
modifying its native implementation or changing the SQLx Rust API. See
`vendor/libsqlite3-sys/CURRENTS-PATCH.md` and complete file provenance.
`cargo xtask qualify native-source` verifies selected amalgamation/header bytes;
the synthetic runtime test verifies linked SQLite/SQLCipher versions on both
hosts/toolchains. This is not full encryption/key-profile qualification.

The observed cargo-deny advisory scan passes without an ignore rule, but does
not flag the local path sys package despite its unchanged version falling in
the advisory range. Scanner green alone is therefore insufficient: the exact
source hashes, inspected upstream fix, independent applicability review and
executed native composition evidence must accompany it. No advisory waiver or
suppression is included. License inventory identifies the selected licenses;
it does not choose a final application license or authorize distribution.
