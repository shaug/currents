# S4: Partial history and backfill (date floor)

`date-floor.json` is synthetic input for
`cur_016_3_2024_backfill_after_2026_declaration` in
`crates/currents-service/tests/cur_016.rs`.

Account B is declared on 2026-11-20 with a 50 USD opening at 2026-11-01. A
segment replacement then opens B at 146 USD on 2024-10-30 and backfills a
-96 USD movement, effective 2024-10-31 and posted 2024-11-02. The test checks
that history replayed through the commit before the replacement still shows
the old records, with nothing on B in 2024. History replayed through the
replacement shows B's new segment and a 50 USD balance at the end of
2024-10-31. The backfill keeps its own value and booking dates.

This covers the native date floor for one account. It does not cover holdings,
cross-account closures or the full S4 reporting journey.
