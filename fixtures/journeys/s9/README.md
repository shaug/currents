# S9: False reconciliation (offsetting omissions and circular anchors)

Both files are synthetic input for the `cur_017` tests in
`crates/currents-domain/tests/cur_017.rs`.

## Offsetting omissions

`offsetting-omissions.json` drives the activity coverage tests. Checking
account A has an import receipt that reports complete coverage from
2026-11-01 to 2026-11-30 by posting date. A's statement shows a 1,000 USD
balance at the end of 2026-10-31, lines of -40, +100 and -100 USD, and a
960 USD balance at the end of 2026-11-30. Only the -40 line is recognized.

The opening plus the recognized movement equals the closing, but the +100 and
-100 lines have no recognized leg. Evaluating activity coverage for A over
November returns `PartiallySupported` and names both lines as missing
allocations. The other tests recognize both lines, exclude them, drop the
coverage report, add legs outside the claim's scope, and correct a
recognition's identities.

## Circular anchor

`circular-anchor.json` drives the balance agreement tests. A's opening of
1,000 USD at the end of 2026-10-31 was adopted from statement balance S1, a
posted balance. S0 reports 1,000 USD posted at the end of 2026-09-30. The
only recognized November movement is the -40 debit on 2026-11-05. S2 reports
960 USD posted at the end of 2026-11-30.

- A balance claim at 2026-10-31 that uses S1 is `PartiallySupported` with
  `CircularAnchor`: S1 established the anchor, so it cannot reconcile it.
- An activity claim over October is `PartiallySupported` with
  `UnknownOpening`. October lies before the anchor, so S0 and S1 cannot
  corroborate it.
- An activity claim over November, which opens with S1 and closes with S2, is
  `Proposed`. November starts after the anchor.
- A balance claim at 2026-11-30 that uses S2 is `Proposed`: 1,000 - 40 = 960.

The other tests change S2 to 950 USD, to an available, statement or unknown
kind, to another unit, or ask at 2026-11-29. Each stays partial, and only the
950 case lists a difference (-10 USD).

These files cover one account. They do not cover aggregate net worth,
position selection or source quantization.
