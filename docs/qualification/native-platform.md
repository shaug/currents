# Native platform qualification (CUR-004)

**Blocked. No native application or shipping target is qualified.**

This partial fixture separates a working Swift/C command-line toolchain from
native app qualification. It does not implement the production C14 ABI, owner
service, signed installation, FinanceKit acquisition, or financial semantics.
The private `currents_platform_probe_roundtrip` symbol is a synthetic toolchain
probe, not a substitute for any `currents_*_v1` API.

## Observed host (2026-11-09)

- macOS 27.0.1, build 26A434, arm64.
- Selected developer directory: `/Library/Developer/CommandLineTools`.
- Swift 6.4 (`swiftlang-6.4.0.34.1`); selected SDK 27.0.
- No full Xcode found by a separate search of `/Applications`,
  `~/Applications`, `/Library/Developer`, and Spotlight's Xcode bundle index.
  CLT selection alone was **not** used as evidence of Xcode absence. Discovery
  is bounded and does not prove an installation cannot exist elsewhere.
- No actual macOS 15.0 arm64 runner, qualified iOS simulator/device destination,
  or signed production fixture was supplied. Intel Mac is unsupported.

No developer-directory setting, signing enrollment, entitlement request,
credential, or account access was changed.

## Repeatable local diagnostics

`cargo xtask qualify native-platform` records the observed prerequisite inputs
in a C16 `QualificationReportV1` and exits nonzero with `Blocked`. Discovery
checks conventional application directories, the selected developer directory,
and Spotlight independently. It never treats CLT compilation as app acceptance.
This partial implementation deliberately cannot return `Passed`: the Xcode
project/schemes and actual minimum/current-OS evidence are still missing.

`cargo test --locked -p currents-ffi --test cur_004` exercises `cur_004_3`:
a child process selected to CLT must return `Blocked` with exact full-Xcode and
macOS 15.0 prerequisites. This is one counterexample, not the entire CUR-004 suite.

The Swift package uses language mode 6 and declares macOS 15/iOS 18 minimums.
A framework-independent diagnostic can run under CLT:

```sh
swift run --build-system native --package-path native/CurrentsPlatform \
  --scratch-path target/swift-native-build currents-platform-probe
```

It exercises async actor-to-C roundtrips for zero, one and `UInt64.max`, and
verifies automatic acquisition remains disabled. This was built and launched
on the observed current OS. It is **not** an app bundle launch on macOS 15.
The CLT's default `swift test` build failed because TestingMacros was missing;
the documented native build-system retry failed because module Testing was
missing. The Swift test suite therefore remains unexecuted, even though the
standalone diagnostic passes. Native build-system deprecation is an additional
reason not to adopt this as a shipping qualification route.

## Acceptance ledger and continuation

- `cur_004_1`: unavailable. Requires full Xcode, actual macOS 15.0 arm64 and
  current-OS app bundle build/launch, exact Xcode build and SDK records.
- `cur_004_2`: pending actual Linux core build from a frozen candidate with
  `native/` absent. A Mac core build is not evidence of Linux independence.
- `cur_004_3`: public CLT counterexample implemented. Its raw output and
  head-bound qualification report must accompany the candidate.
- `cur_004_4`: only the synthetic disabled-acquisition policy probe ran. iOS 18
  guarded FinanceKit compilation and actual entitlement behavior require a
  qualified Apple SDK/destination; no acquisition API is called by this fixture.

On a capable host, implement and validate `native/Currents.xcodeproj` with schemes
CurrentsApp, CurrentsOwner, CurrentsCapture and CurrentsPlatformTests, and plans
CurrentsSynthetic and CurrentsProductionQualification. Preserve the distinction
between synthetic fixtures and signed installed production targets. The owning
native runner must execute:

```sh
xcodebuild test -project native/Currents.xcodeproj \
  -scheme CurrentsPlatformTests -testPlan CurrentsSynthetic \
  -destination 'platform=macOS,arch=arm64'
```

Record the exact iOS simulator/device identifier in `tools/versions.lock.json`
only after qualification; none is guessed here. Keep production blocked until
its separately authorized signed-host and entitlement gates pass. Register all
four exact CUR-004 assertions and reject absent/zero-match tests before allowing
this gate to pass. Do not close issue #13 on the partial diagnostics above.
