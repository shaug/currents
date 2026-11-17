# Apple Card acquisition route (CUR-044)

## Current disposition

The route is **Blocked**, not qualified or deployed. Run
`cargo xtask qualify financekit` for a nonzero C16 report containing public
prerequisite codes. It performs no account or entitlement probe and accepts no
arguments. Never provide account details or credentials on a command line:
shell history and Cargo's launcher diagnostics are outside this gate's boundary.
Rejected argument values are redacted from the gate's own report and error.

`cargo xtask ticket CUR-044` tests the synthetic fail-closed contract; its success
is not operational FinanceKit qualification. No private developer-account facts,
bundle identifier or binary eligibility have been established. Automatic capture
is disabled. Real account coverage and timeliness remain CUR-057 obligations.

## Public route requirements

Checked 2026-11-17 against [FinanceKit](https://developer.apple.com/financekit/)
and [Apple's framework documentation](https://developer.apple.com/documentation/financekit.md).
Apple documents organization-level Developer membership and an Account Holder
entitlement request. The managed entitlement is tied to an app's bundle ID and a
single binary. App Store eligibility includes the Finance category, financial
management functionality and iPhone distribution in the US or UK.
The app also needs a financial-data usage description.

US framework availability begins at iOS 17.4; UK support begins at iOS 18.4.
Currents retains its existing iOS 18.0 API floor. Public API availability does not
establish this project's approved distribution, runtime coverage or eligibility.
Apple Card Family participants are excluded from the documented US Card route.
A personal Developer account, signing or sideloading alone is not proof of eligibility.

## Evidence needed before changing the operational disposition

Recheck the public requirements, then obtain separately authorized evidence of:
organization eligibility; the actual granted entitlement; matching bundle and
binary; permitted distribution; and the installed usage description.
Do not substitute a fixture or an assertion supplied through CLI flags.
Authorization, account availability, coverage/history and timeliness must still
be qualified under their owning tickets. This gate neither requests consent nor
enables capture even when synthetic prerequisite fixtures are complete.

Statement import remains an independent, non-automatic route. CUR-044 exercises
the existing synthetic CSV parser without claiming Apple statement format parity.
No actual export, complete account history or owner-accepted limitation is proven.
Unknown acceptance remains unknown. A future explicit limitation cannot waive
accounting, transfer, privacy or security invariants. C15 and C16 remain unchanged.
