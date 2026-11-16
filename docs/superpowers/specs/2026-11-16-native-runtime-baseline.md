# Explicit native runtime baseline

CUR-004's exact macOS 15.0 runtime promise originated in the implementation plan,
not the constitution, technical design, shared contracts, or owner's requested
compatibility. Testing the unpatched release would require a separate VM, storage,
and setup approval. Adopt an explicit planned patched runtime baseline instead.

The initial requested macOS runtime baseline is **15.8.2 arm64**. The compilation
and API deployment floor remains **15.0**; that build setting does not promise
runtime support for untested releases. The baseline is a requested qualification
input, not a claim that this candidate passed. `minimum_macos_runner` stays null.

Alternatives considered: an exact 15.0 VM (unnecessary setup and capacity cost),
accepting any macOS 15.x result (ambiguous support claims), and an explicit locked
patched baseline (selected). Actual recorded OS must equal the lock, not merely
share its major version. Missing/malformed lock values fail closed. Current-host,
iOS 18.x, Linux, CLT, exact candidate, clean tree, receipts, Debug/Release, and
production/security gates remain unchanged. Old supplemental results remain old
supplemental results; no artifact conversion or relabeling is included.

The next stage is candidate-bound collection of all five roles and independent
review. Hosted integration is separate from this bounded contract correction.

## Hosted simulator identity

A requested simulator runtime (initially iOS 18.6) is a qualification input,
not an observed device. It is mutually exclusive with the existing fixed
qualified destination. Fixed physical-device and fixed simulator behavior stays
unchanged. Runtime-mode collection captures actual `simctl list runtimes -j`
and `simctl list devices -j` receipts, requires one available exact iOS runtime,
and selects the lexically first available UUID in that runtime. Duplicate runtime
or device identities, malformed inventory and unavailable selected resources
fail closed. No runtime/device is installed, created or guessed.

The evaluator independently derives selection from hashed command receipts.
Build destination and both Debug/Release xcresult UUID, platform and runtime must
match that selection and the requested runtime pin. Existing candidate/tree,
clean-worktree, native-file hashes, five-role aggregation and production blockers
remain. This does not convert historical supplemental results to qualification.
