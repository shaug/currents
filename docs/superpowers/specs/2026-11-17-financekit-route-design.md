# FinanceKit route-only gate

## Intent and scope
CUR-044 records whether an Apple Card acquisition route is established, without
probing an account. The implementation is a safe, executable blocked gate with
synthetic counterexamples. It is not FinanceKit capture or production readiness.
The coordinator approved this bounded design on 2026-11-17.

## Approach
Use the existing xtask/C16 report boundary and acquisition test crate. A small
pure prerequisite evaluator in `xtask/src/financekit_gate.rs` distinguishes
organization eligibility, managed entitlement, matching bundle/binary,
distribution, usage description and explicit fallback acceptance. Real execution
uses unresolved facts; it accepts no CLI claims that could promote eligibility.
Unit fixtures exercise complete and incomplete prerequisite combinations but
never become live qualification receipts. The operational command stays Blocked.

A docs-only answer would leave no executable regression barrier. Implementing an
Apple adapter or entitlement submission would exceed this ticket and authority.
The selected gate-only approach preserves both useful progress and honest limits.

## Boundaries
- C15 transfer interfaces stay unchanged. Actual account coverage belongs to CUR-057.
- No account API, authorization UI, sign-in, enrollment, capability activation,
  signing, spending, developer-account discovery or network request in this gate.
- Public output contains fixed reason codes only, never private input values.
- Project iOS 18.0 API floor stays unchanged; public US framework availability at
  17.4 is not proof of entitlement or this application's runtime support.
- Synthetic statement import can work without qualifying automation. An absent
  owner-accepted limitation remains unknown; even accepted limitations cannot
  waive accounting/security invariants or activate FinanceKit.
- Tests pass for correct blocked behavior. `qualify financekit` remains nonzero
  Blocked while route prerequisites are unresolved. No skipped-green gate.

## Evidence and source obligations
Document the current official sources and date, distinguishing public rules
from actual private eligibility. No developer identity or bundle ID is invented.
Entitlement configuration is documentary only, not an enabled plist capability.
Future separately authorized proof needs revalidation; no arbitrary age allowance
or developer attestation is silently interpreted as Apple's approval.

## Review
Review the whole candidate through the installed canonical review-fix-loop,
fresh_subagent/local_commit, its normal three-cycle budget. No merge until
required synthetic gates and independent review pass. Operational status remains
explicitly Blocked even if the route-only implementation is delivered.
