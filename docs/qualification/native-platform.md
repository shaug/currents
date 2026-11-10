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
`cur_004_1()` and `cur_004_4()`. Legacy XCTest's zero-test summary is not evidence
for the separately executed Swift Testing suite. Raw command output is retained
in `target/qualification/commands.jsonl`; fresh result bundles and a head/dirty
bound current-host observation live under `target/qualification/native-*`.
The overall command remains nonzero until the remaining cross-host acceptance
and its evidence aggregation are completed; it cannot certify shipping today.

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
- `cur_004_2`: earlier candidate 907d043 passed a real Linux core build with
  native unavailable; it is stale for the resumed candidate and must be rerun.
- `cur_004_3`: public CLT-only counterexample and ticket-dispatch test pass;
  both require a nonzero blocked result, not an app-pass claim.
- `cur_004_4`: disabled-acquisition test passes on current macOS and iOS SDK
  compilation succeeds; actual qualified iOS 18 destination remains unavailable.

No final acceptance, issue closure, production qualification or native shipping
claim follows from this partial matrix. Preserve raw outputs, fixture hashes,
exact candidate identity and independent review before any readiness decision.
