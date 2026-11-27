# S9: False reconciliation (offsetting omissions)

`offsetting-omissions.json` is synthetic input for the `cur_017` activity
coverage tests in `crates/currents-domain/tests/cur_017.rs`.

Checking account A has an import receipt that reports complete coverage from
2026-11-01 to 2026-11-30 by posting date. A's statement shows a 1,000 USD
balance at the end of 2026-10-31, lines of -40, +100 and -100 USD, and a
960 USD balance at the end of 2026-11-30. Only the -40 line is recognized.

The opening plus the recognized movement equals the closing, but the +100 and
-100 lines have no recognized leg. Evaluating activity coverage for A over
November returns `PartiallySupported` and names both lines as missing
allocations. The other tests recognize both lines, exclude them, drop the
coverage report, add legs outside the claim's scope, and correct a
recognition's identities.

This covers activity coverage for one account. It does not cover balance
agreement, a statement-derived opening, or aggregate net worth.
