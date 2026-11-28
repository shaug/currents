# S6: Processor change and category exception

`processor-change.json` is synthetic input for the `cur_019_1_*`,
`cur_019_precedence_owner_provider_unknown`, `cur_019_alias_conflict_is_explicit`
and `cur_019_s6_journey_rebuilds_identical_meaning` tests in
`crates/currents-domain/tests/cur_019.rs`.

Account K holds two purchases. Observation O1 backs purchase L1 (96 USD on
2026-11-03) and reports the description "SQ *BLUE BOTTLE" and the category
"Restaurants". Observation O2 backs purchase L2 (40 USD on 2026-11-10) and
reports the description "BLUE BOTTLE COFFEE 0412" and no category.

The owner defines party Blue Bottle with alias A1 for O1's description. The
processor then changes, so the owner adds alias A2 for O2's description, and
both legs report Blue Bottle without any change to O1 or O2. The owner sets
L1's category to Coffee. A source revision of O1 then reports the category
"Dining": L1 keeps Coffee from the owner, and its provider proposal reads
"Dining". When the owner clears the value, L1 reports "Dining" from the
provider.

The journey test replays this history twice from the same base and checks
that both replays report the same value, tier and source records for every
leg and field.

This covers aliases, the owner and provider tiers, and source revisions on
stable legs. It does not cover rules, reporting splits or the full S6
reporting journey.
