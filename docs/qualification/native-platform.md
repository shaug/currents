# Native platform qualification

CUR-004 remains **Blocked**. Current-host synthetic results do not establish
minimum-OS, signed installation, FinanceKit entitlement, or production readiness.
Intel Mac remains unsupported.

## Reproducible current-host checks

`cargo xtask qualify native-platform` (also `cargo xtask ticket CUR-004`) first
records the selected developer directory, separately discovered full Xcode
installations, OS/architecture, Swift and SDK versions. A CLT-only selection
fails with the full-Xcode prerequisite even if Xcode exists elsewhere. It never
changes developer settings or accepts agreements.

On an arm64 host with selected full Xcode it runs Debug and Release:

```sh
xcodebuild test -project native/Currents.xcodeproj \
  -scheme CurrentsPlatformTests -testPlan CurrentsSynthetic \
  -destination 'platform=macOS,arch=arm64'
```

The runner supplies configuration, isolated result-bundle paths, owned derived
data and `CODE_SIGNING_ALLOWED=NO`. It reads actual `xcresulttool get test-results
tests` JSON, rejects missing/skipped/failed or duplicate test cases, and requires
`cur_004_1()`, `cur_004_4()` and `cur_004_actor_c_roundtrip()`. Legacy XCTest's zero-test summary is not evidence
for the separately executed Swift Testing suite. Raw command output is retained
in each fresh `evidence.json`; result bundles and before/after candidate-bound
observations live under `target/qualification/native-*`.
The overall command remains nonzero until the remaining cross-host acceptance evidence
is supplied to the evidence aggregator; it cannot certify shipping today.

## Fixtures and boundaries

The checked-in project exposes CurrentsApp, CurrentsOwner, CurrentsCapture and
CurrentsPlatformTests schemes. Minimal app/owner fixtures are synthetic `.app`
bundles whose executables exercise the Swift actor and private C-header
roundtrip. They do not install/register an owner or implement C14 financial ABI.
The Swift package uses Swift 6 with strict concurrency, macOS 15.0 and iOS 18.0
minimums. CurrentsCapture uses the separate capture synthetic plan; no signed
production or financial acquisition operation is invoked.

`CurrentsProductionQualification` is a distinct plan with an explicit failing
prerequisite assertion. It is not an empty successful plan and does not substitute
synthetic applications for signed installed targets. Later production owners
must replace the blocker with actual authorized installed-target evidence.

CaptureAvailability conditionally imports FinanceKit on iOS and is guarded at
iOS 18.0. Automatic acquisition always refuses while entitlement/account
qualification is absent. This proves the disabled policy, not real entitlement
availability or provider access.

## Observed tools and evidence

After separately approved installation/setup, observed Xcode 27.2 (27B52),
macOS/iOS SDK 27.2, host macOS 27.0.1 (26A434) arm64. Observations are recorded
under `native_platform` in `tools/versions.lock.json`; both minimum macOS runner
and qualified iOS destination remain null. No device identifier is invented.

Working-tree current-host tests executed all three Swift assertions successfully:
`cur_004_1()`, `cur_004_4()`, and `cur_004_actor_c_roundtrip()`. The first launches
both bundle executables and checks their actor/C output. Initial absent-project
red and the test-plan variable-expansion failure are preserved alongside green
logs. iOS arm64 Simulator SDK build-for-testing succeeded with deployment 18.0;
no iOS simulator runtime/device was available and no iOS test executed. The
production plan returned its explicit prerequisite failure. Exact candidate
qualification must be repeated after freezing the final reviewed candidate.

## Acceptance ledger

- `cur_004_1`: current-host portion measured; actual locked-baseline macOS arm64 build/launch
  remains missing. Setting deployment target 15.0 is not an actual OS run.
- `cur_004_2`: preserved candidate b712be2 passed a real Linux core build with
  native unavailable; it is stale for the resumed candidate and must be rerun.
- `cur_004_3`: public CLT-only counterexample and ticket-dispatch test pass;
  both require a nonzero blocked result, not an app-pass claim.
- `cur_004_4`: disabled-acquisition test passes on current macOS and iOS SDK
  compilation succeeds; actual qualified iOS 18 destination remains unavailable.

No final acceptance, issue closure, production qualification or native shipping
claim follows from this partial matrix. Preserve raw outputs, fixture hashes,
exact candidate identity and independent review before any readiness decision.

## Candidate-bound evidence aggregation

The local evidence format is versioned and contains command receipts, not manually
selected pass flags. Every receipt retains the exact program, argument vector,
exit status, stdout/stderr and SHA-256 of both raw outputs. Each matrix entry
captures HEAD, the committed tree identity and worktree status before and after
execution. The aggregate evaluator must reject stale candidates, dirty trees,
changed trees, missing or duplicate roles, failed commands, mismatched commands,
missing assertions, zero tests, skipped tests and wrong runtime environments.
The five roles are `macos-minimum`, `macos-current`, `ios`, `linux` and `clt`.
Debug and Release are required for each Apple runtime role.

The minimum macOS role must execute on the exact
`native_platform.requested_macos_runtime_baseline` version (initially 15.8.2)
on arm64. The field is a requested qualification input, not observed proof.
The macOS 15.0 compilation/API floor remains unchanged and does not imply
runtime support for 15.0. Missing or malformed baseline values fail closed. The current role
uses the current macOS version recorded in the inventory. Different host roles
may use different full Xcode/SDK versions; each records its own exact versions
and requires Swift 6 or newer. In particular, an Xcode release requiring a newer
host cannot be imposed on the locked minimum-runtime runner. iOS 18.0 is the deployment/API
availability floor. Qualification accepts actual iOS 18.x minor and patch runtime
results, including 18.5 and 18.6. Fixed-destination mode uses the exact destination
from the checked-in inventory. Runtime-pinned simulator mode instead locks
`native_platform.requested_ios_simulator_runtime` (initially 18.6), captures
actual simulator inventories and deterministically selects an available UUID
from that exact runtime. These modes are mutually exclusive. A null fixed
destination is allowed only with a valid requested simulator runtime. Each result
records its actual runtime; no result claims execution on untested iOS 18.0. Linux evidence requires the
native directory to be absent before and after the build, with only unstaged
native-file deletions allowed. It records `uname -m` and `rustc -vV`, requires a
Linux compiler host triple matching that architecture, and passes that triple
explicitly as `cargo build --target`. Ambient `CARGO_BUILD_TARGET` or Cargo
configuration therefore cannot substitute a cross-target build. The collector
does not remove that directory.

These artifacts are local qualification records from trusted operator-controlled
runners, not signed remote attestations. Hashes detect corruption, not malicious
rewriting by someone who controls the artifact. Aggregation treats supplied
artifacts as read-only data and never executes their commands or paths. Synthetic
positive evaluator fixtures exist only inside unit tests and are not operational
qualification evidence. Production installed-target qualification remains
separate and blocked by its own test plan.

Collect each role on its actual runner, at the same clean committed candidate:

```sh
cargo xtask qualify native-platform --emit macos-minimum
cargo xtask qualify native-platform --emit macos-current
cargo xtask qualify native-platform --emit ios
cargo xtask qualify native-platform --emit linux
cargo xtask qualify native-platform --emit clt
```

Each invocation writes a fresh `target/qualification/native-*/evidence.json`.
Successful collection still exits nonzero with **Blocked**, because one role is
not the complete gate. Failed or incomplete runs retain their receipts and cannot
qualify. The Linux collector requires the operator's isolated checkout to have
`native/` already unavailable; it does not rename or delete files. Other roles
require a fully clean checkout. Native input bytes are checked against committed
sources before and after each non-Linux run, in addition to the tree/status checks.

Transfer the five complete artifacts to a clean checkout of that candidate and
run the read-only evaluator (paths can be in any order):

```sh
cargo xtask qualify native-platform --aggregate \
  /evidence/minimum/evidence.json /evidence/current/evidence.json \
  /evidence/ios/evidence.json /evidence/linux/evidence.json \
  /evidence/clt/evidence.json
```

The report hashes each imported artifact and labels inherited assertion results
`native-evidence-receipt`. Imported commands are never reported as executed by
the aggregate invocation. A complete valid matrix can pass the CUR-004 synthetic
platform gate; its result still explicitly records production as **Blocked**.
There is no command-line production override. Existing older `current-host.json`
observations lack this protocol and must not be converted by filling in claims.
Rerun collection after freezing the candidate instead.

In fixed-destination mode, the inventory value is a
string with the exact form `platform=iOS Simulator,id=<qualified identifier>` or
`platform=iOS,id=<qualified identifier>`. The evaluator checks both that ID and
platform against xcresult's executed device and requires a numeric iOS 18.x
runtime version with two or three components. It rejects versions below the
18.0 floor, malformed versions and other major versions. SDK or deployment
target values do not replace actual runtime evidence. Debug and Release must
still pass every required assertion on arm64 at the locked destination.

In runtime-pinned simulator mode, the canonical emitter captures raw command
receipts for `xcrun simctl list runtimes -j` and `xcrun simctl list devices -j`.
The evaluator independently selects the lexically first available UUID from the
exact pinned iOS runtime identifier/version and checks it against each build
destination and both actual xcresult UUID/runtime/platform observations. Missing,
malformed, duplicate or unavailable inventory, conflicting modes, and identity
mismatches fail closed. No device/runtime creation or installation is attempted.

The macOS minimum criterion is the explicit locked patched-runtime baseline,
not an arbitrary 15.x runtime and not the 15.0 compilation target. No untested
OS is claimed. The positive 18.5/18.6 evaluator fixtures are synthetic
unit tests only; they do not qualify a runtime or populate the null destination.

## Hosted patched-OS observations (supplemental only)

The original `native-supplemental.yml` workflow supported manual dispatch and PRs
that change native inputs or its collector. It does not replace the existing CI
or the exact-minimum platform gate. Its `macos-15` hosted runner is required to
actually report arm64 and macOS 15.x; an Intel or different-major runner fails.
The job selects the preinstalled Xcode 16.4 through job-local `DEVELOPER_DIR`,
records the actual Xcode/Swift/SDK/OS build versions, and never changes the
machine's selected developer directory. Availability is checked during the run,
not inferred from a runner label or an older inventory.

`tools/native-supplemental.py` reads the existing simulator runtime and device
inventories. It selects the latest available numeric iOS 18.x version, breaking
ties by simulator identifier. It records that exact identifier and actual runtime
as a **supplemental destination**, never as a `tools/versions.lock.json` pin.
Missing Xcode or a preinstalled usable runtime/device blocks collection with no
installation, runtime download, device creation or settings-change fallback.
The retained script is restricted to a GitHub Actions environment to avoid accidentally
starting native builds on a local machine.

Both macOS and iOS execute the existing synthetic plans in Debug and Release,
with signing and parallel testing disabled. The collector requires the exact
nonzero assertion sets, all Passed, matching summary counts, no failed/skipped
or expected-failure tests, actual arm64, actual macOS version/build matching the
host, and actual iOS version/identifier matching the selected simulator.
It records the exact workflow/checkout candidate SHA, committed tree and clean
worktree before and after execution. A successful observation is not accepted
if the candidate or worktree changes.

The failure-safe artifact upload preserves raw command stdout/stderr, their
SHA-256 digests, parsed tests/summaries and xcresult bundles under
`target/native-supplemental/`. The report uses the distinct
`SupplementalNativeObservationV1` schema. A successful workflow means only that
four supplemental runs were observed. Its native and production qualification
statuses remain **Blocked**, with minimum-runtime qualification **Pending** (historical reports name the
then-requested exact 15.0 baseline).
This report cannot be supplied to the five-role native evidence aggregator.
A macOS 15.8.x or iOS 18.5/18.6 observation must retain its actual version.
Historical supplemental reports are not converted into full qualification
merely because the requested baseline is now 15.8.2. Publishing or dispatching the workflow is a separate step;
adding these files does not establish any hosted runtime result.

The workflow has a read-only contents token, no supplied secrets, no persisted
checkout credentials, immutable action references and a 30-minute job limit.
The collector has a shorter 25-minute budget and per-command timeouts. Offline
counterexamples can be run without Apple tools or network access:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools -p test_native_supplemental.py
```

## Runtime support scope correction

The original exact 15.0 runtime promise was an implementation-plan choice, not
an owner compatibility requirement. The locked patched baseline avoids a separate
unpatched-OS VM while making the intended support boundary explicit. Compile/API
floor, requested runtime baseline, observed runtime and full qualification are
separate claims. The baseline remains **unqualified** for this changed candidate;
`minimum_macos_runner` and the qualified iOS destination remain null. Complete
same-candidate evidence and independent review are still required. Production,
signing, FinanceKit, financial and dependency-security gates are unchanged.

## Hosted canonical partial collection

The updated workflow avoids rerunning the same native configurations through
both collectors. It retains the supplemental implementation, tests and historical
reports, but installs the pinned
Rust toolchain and runs only `tools/native-canonical.py` for new observations. It calls the existing
canonical emitter for `macos-minimum` and `ios`, preserving fresh raw artifacts
and logs. PR collection checks out the immutable PR head SHA, not the synthetic
merge ref, and verifies it against the workflow-provided expected candidate.
Each role has a ten-minute process timeout within the thirty-minute job budget.
On timeout the helper terminates its owned process group, force-kills remaining
members even if the leader exited, reaps its direct child and checks group
disappearance. Partial stdout/stderr and an explicit timeout receipt with actual
return code/signal are retained; timeout never counts as collected evidence.
A successful individual emit intentionally exits nonzero with the
exact collected-but-incomplete report; the helper accepts only that outcome,
a clean matching candidate/tree, successful captured commands, and a fresh
artifact in the owned qualification directory. Generic Blocked/error output,
missing/stale artifacts or path escapes fail the workflow. No blanket
continue-on-error or production override is used.

These two collected roles do not qualify the full matrix. The current-Mac,
Linux-with-native-absent and CLT evidence must still be collected at the exact
same clean candidate and all five artifacts must pass canonical aggregation.
Historical supplemental artifacts remain supplemental; neither this code nor
its synthetic tests establish any new native execution result.
