# Position: source precedence and transfer overlap

Both files are synthetic input for tests in
`crates/currents-domain/tests/cur_018.rs`.

## Source precedence

`source-precedence.json` drives the `cur_018_1` tests.

Checking account A holds USD and has an alias for connection S1 and another
for S2. Each case commits its reports in list order, each in its own import
receipt, and then runs its queries. A query names the requested date, the
reports the response lists, the reports it selects, and whether it reports a
`conflict`. A query with a `priority` first commits that source priority,
effective from its `from` date.

- `older_as_of_received_later`: S1 reports 1,200 as of 2026-11-05 and then
  1,000 as of 2026-11-03. On 2026-11-06 the 1,200 report is selected, because
  its as-of date is later, even though the 1,000 report arrived later.
- `requested_date_limits_candidates`: the same reports. On 2026-11-04 only the
  1,000 report is a candidate. On 2026-11-02 the query lists and selects
  neither.
- `overlapping_boundaries_conflict`: S1 reports 1,200 as of the date
  2026-11-05 and S2 reports 1,150 as of the instant 2026-11-05T18:00Z. The date
  spans every timezone from UTC+14 to UTC−12, so neither report is later. The
  amounts differ, so the query selects neither.
- `overlapping_boundaries_corroborate`: the same boundaries with equal amounts.
  Both reports are selected.
- `equal_boundary_needs_adopted_priority`: S1 reports 1,200 and S2 reports
  1,180, both as of 2026-11-05. Without a source priority they conflict. With
  the priority S2, S1 from 2026-11-01, S2's report is selected. The test also
  commits the reports in reverse order and gets the same responses.

These cases cover one component. They do not cover the journal balance,
freshness, coverage or the household aggregate.

## Transfer overlap

`transfer-overlap.json` drives `cur_018_2_balance_receipt_before_feed`.

Checking A (USD, connection S1) sends 1,000.00 to savings B (USD, connection
S2), each owned 1/1, by one transfer association with a sending leg dated
2026-11-04 and a receiving leg dated 2026-11-05. A reports 4,000.00 as of
2026-11-05, and A's outflow line is allocated to the sending leg. B reports
3,000.00 as of 2026-11-05 before B's feed arrives, so B's balance may or may
not include the receipt.

- Before the feed, the aggregate on 2026-11-06 is the range from 7,000.00
  (B includes the receipt) to 8,000.00 (the funds are in flight).
- B's feed then brings the receipt line, dated 2026-11-05 and allocated to
  the receiving leg, and complete posting-date coverage from 2026-11-01
  through 2026-11-05. B's balance includes the receipt, and the aggregate is
  exactly 7,000.00.
