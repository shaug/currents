# S4: Partial history and backfill (bounded CUR-016 witness)

`date-floor.json` is synthetic input for
`cur_016_3_2024_date_floor_preserves_old_basis_and_repairs_current_as_of`.
The test asserts an unknown 2024 balance under the archived basis and -96
under the corrected basis. The 2024 movement keeps distinct effective and
booking dates. The cross-account tests check that re-emitted counterparts
keep their economics, and the domain suite checks that a residual blocks the
replacement without a balancing posting. This does not qualify holdings or
the general S4 reporting journey.
