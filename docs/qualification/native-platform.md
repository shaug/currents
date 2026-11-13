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

- `cur_004_1`: current-host portion measured; actual macOS 15.0 arm64 build/launch
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

The minimum macOS role must execute on exactly macOS 15.0 arm64. The current role
uses the current macOS version recorded in the inventory. Different host roles
may use different full Xcode/SDK versions; each records its own exact versions
and requires Swift 6 or newer. In particular, an Xcode release requiring a newer
host cannot be imposed on the macOS 15.0 runner. iOS 18.0 is the deployment/API
availability floor. Qualification accepts actual iOS 18.x minor and patch runtime
results, including 18.5 and 18.6, at the exact destination from the checked-in
inventory. Each result records its actual runtime; a patched-runtime pass does
not claim execution on iOS 18.0. A null
`native_platform.qualified_ios_destination` remains a blocker; no identifier is
generated from a convenient available simulator. Linux evidence requires the
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

When an iOS destination has actually been qualified, its inventory value is a
string with the exact form `platform=iOS Simulator,id=<qualified identifier>` or
`platform=iOS,id=<qualified identifier>`. The evaluator checks both that ID and
platform against xcresult's executed device and requires a numeric iOS 18.x
runtime version with two or three components. It rejects versions below the
18.0 floor, malformed versions and other major versions. SDK or deployment
target values do not replace actual runtime evidence. Debug and Release must
still pass every required assertion on arm64 at the locked destination.

The macOS minimum criterion remains actual macOS **15.0** arm64, not a newer
15.x runtime. This iOS correction neither changes that requirement nor claims
coverage of an untested OS. The positive 18.5/18.6 evaluator fixtures are synthetic
unit tests only; they do not qualify a runtime or populate the null destination.
