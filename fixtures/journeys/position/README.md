# Position: source precedence

`source-precedence.json` is synthetic input for the `cur_018_1` tests in
`crates/currents-domain/tests/cur_018.rs`.

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
