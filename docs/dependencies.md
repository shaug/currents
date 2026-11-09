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
No `rust-version` promise is made merely because dependency resolution succeeds.
No indiscriminate `--all-features` invocation enables PostgreSQL/MySQL.

`ticket CUR-001` first verifies the exact four assertion names, then requires
four passed, zero failed and zero ignored tests. An unknown ticket or missing
suite fails. Native/security/deep/release/all and production/private-activation
gates remain explicitly BLOCKED until their owning tickets register the required
suites. They never count an absent suite or report as passed.

Every gate emits a versioned `QualificationReportV1`, with exact command, current
Git head, dirty state, host, evidence and limitations. Reports and full subprocess
stdout/stderr are retained in `target/qualification/`. Dirty-tree results are
preliminary and must be rerun at the final committed candidate. A pass certifies
only the named gate on the recorded host, never another platform or the complete
product. Compiler identity probes and source hashing are not build qualification.

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
`de78ef087cbc26e8fe584de05f1790f3e34897db8731e8154dc674d3f2df28cb`.
Final runtime link/key-profile qualification remains CUR-006.

## Qualification status, 2026-11-09

The initial public command-contract assertions were executed against documentation
base c34aa3d5ab463652dbf14d8864b8e60911213974: four failures because xtask did not
exist. These are separate from environment prerequisites. On the implementation
worktree, the four assertions, macOS arm64 quality checks and full Rust 1.94.0
workspace build/tests subsequently passed. These preliminary dirty-tree runs do
not substitute for final-candidate evidence or Linux execution.

Still required before claiming complete qualification: final-candidate Mac/Linux
runs, source-cache verification, independent review, cache-free and source-vendored
frozen/offline builds, and a measured unsigned reproducibility comparison. Security
and license review must retain exact findings rather than infer safety from a
successful resolver. No unresolved native, advisory, license, reproducibility or
external requirement is silently labeled PASS.

### Blocking security finding

**BLOCKED as of 2026-11-09.** RUSTSEC-2026-0459 (CVE-2026-119375)
reports uninitialized page-cache memory disclosure for SQLite before 3.54.0 when
opening an attacker-controlled database together with a sibling WAL whose page
size differs. The selected SQLCipher source embeds SQLite 3.50.4. This is a
source/version applicability assessment; an encrypted-path exploit has not been
reproduced here. No real data is used by this scaffold.

The advisory explicitly notes that even libsqlite3-sys 0.39.0/0.39.1's
bundled-sqlcipher path embeds affected SQLite 3.53.4. Upgrading only the sys
package therefore does not establish a fix; SQLx 0.9.0 also constrains sys below
0.38.0. Do not treat Cargo build success or a future package-range-only audit as
proof that the encrypted source is safe.

Evidence: [RustSec advisory at immutable database revision](https://github.com/RustSec/advisory-db/blob/d9cb059f3217db87aeccd190b81eaffe8cc29cb9/crates/libsqlite3-sys/RUSTSEC-2026-0459.md).

Qualification and publication are withheld pending a reviewed patched SQLCipher
source/dependency composition, or an explicitly owner-reviewed scoped and expiring
exception supported by concrete mitigation evidence under the dependency policy.
No exception, advisory suppression or design amendment is included in this change.
The lockfile is a reproducible candidate, not a production-qualified dependency set.

### Official-source refresh under review

The preceding blocked snapshot is preserved at commit2ca9bec. Subsequent official
source research found SQLCipher4.20.0/SQLite3.54.1, commit
355d4db92d3404e71faa8088d071a71d767807f4, containing the upstream fix. Current
candidate work adapts the private sys0.37 build to exact registry
sqlcipher-src420.0.0, without modifying its native implementation or changing the
SQLx Rust API. See vendor/libsqlite3-sys/CURRENTS-PATCH.md and its complete file
provenance. `cargo xtask qualify native-source` verifies the actual selected
amalgamation/header bytes; a separate synthetic runtime test checks both versions.
Mac current/MSRV builds and runtime smoke tests passed for this composition.

The raw advisory scanner still reports the unchanged package version. It has not
been suppressed. Final source-specific applicability review and Linux target
requalification remain required before publication. Source verification is not
an exception for unresolved vulnerable code. All other advisories still require
normal independent qualification.
