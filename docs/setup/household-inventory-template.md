# Private household inventory and meaning questionnaire

This public template contains no real household data. Product owner is the
accountable decision role. The implementing product engineer owns this input
contract, not the owner's financial decisions. Do not put completed answers,
statements, institution account identifiers, balances, credentials or private
interpretations in this repository, issues, pull requests, CI or shared logs.
Keep any completed copy in an owner-selected private protected location. No
location, provider, consent, deadline, cost or commitment is chosen here.

## Format and local check

Start with `fixtures/setup/synthetic-household/inventory.json` for a **synthetic**
example only. The contract is `schemas/setup/inventory-v1.json`. For a private
copy, set `profile` to `actual_household`. Use locally assigned opaque component
IDs, never account numbers. Keep a separate `inventory_manifest` of every
component discovered during the inventory interview. Every manifest ID must occur
exactly once in `components` and once in `coverage`. The checker rejects omissions,
duplicates and unknown fields. It cannot discover an account omitted from all
three lists: the owner must attest to the scope of the original inventory.

Run `cargo xtask household-readiness < /path/to/private-inventory.json` locally.
Input is read only from stdin, capped at 1 MiB; the command does not persist the
input, echo values, emit identifiers, or record its private contents/digest in
qualification logs. Output uses component ordinals in input order and fixed
field/action vocabulary. Treat even counts and kinds as private, not publishable.
No claim is made about secure memory erasure, OS swap, shell history, or terminal
retention. Never pass private answers as command arguments.

The result is an intake-scoped `ProductionReadinessV1`, not a financial DTO,
`QualificationReportV1`, or proof of production readiness. A structurally valid
synthetic input exits zero with `inventory_status: Passed`. An actual-household
profile always exits nonzero and retains `actual_household.status: Blocked`, even
when every answer is supplied. Activation, owner verification, signing, provider
consent, backup/recovery and other C16 gates belong to their own tickets. This
command neither accesses institutions nor changes any dataset.

The public engineering gate `cargo xtask qualify household-inputs` (also
`cargo xtask ticket CUR-047`) runs exactly `cur_047_1`, `cur_047_2`, `cur_047_3`,
rejects missing/ignored tests, and records synthetic fixture/schema digests and
normal tool/platform/test evidence in a `QualificationReportV1`. Its success
certifies this contract, never the actual household. Unrelated synthetic work
needs no private answers.

## Inventory interview

Record **every** institution/account and separate component: cash, cards, debt,
investments, manual assets, manual liabilities, Apple Card, and still-unknown or
unconnected components. Unknown value/history means unknown, never zero or absent.
Brokerage cash, holdings and reported totals may overlap: do not add them together
without an explicit adopted perimeter. Each component carries these answers:

- `institution`, `account`: private local reference/identity and any unresolved
  alias relationship. Do not enter credentials or full financial account numbers.
- `currencies`: native currency/security units, and unresolved unit identity.
  Preserve distinctions from reporting currency. Units/amounts eventually use
  C02 typed values; this questionnaire does not create financial records.
- `ownership`: exact household share (for example `1/2`, not a rounded float),
  ownership perimeter, supporting evidence, and any disputed share.
- `overlaps`: explicit other component references and overlap meaning, or an
  explicit owner statement that none are known. Unknown does not mean none.
- `exports`: available statement/file formats, coverage periods, source precision
  and gaps. Describe private references, not an upload or executable file path.
- `automatic_required`: confirmed `yes` or `no`, otherwise unresolved. A file or
  manual fallback is not equivalent to reliable automatic acquisition.
- `opening`: evidence reference, as-of date/precision, amount/unit reference and
  missing history. Preserve unknown openings without invented balancing activity.
- `inclusion`: explicit scope/boundary and exclusions, their evidence, and how
  overlapping components are kept visible without double counting.

For every component, a `coverage` row selects `automatic`, `file`, `manual`, or
`unknown`, and records `limitation.status` as `unassessed` or `owner_accepted`.
An accepted limitation requires nonblank `detail`: exact residual scope, owner
acceptance and evidence. If there is no limitation, the owner must explicitly
record why the route satisfies the requirement. A route label or acceptance text
is not independent feasibility evidence. Unknown routes remain unresolved even
with accepted limitations. Apple Card automation remains separately blocked
pending entitlement/distribution/account eligibility and coverage qualification.
Statement-only compromises require explicit owner acceptance; none is inferred.
Limitations cannot waive constitutional invariants.

## Material meanings

The `meaning` object records:

- `timezone`: explicit IANA household zone and timezone-data version reference;
  no ambient system timezone. Date-only source values remain date-only.
- `reporting_currency`: explicit reporting unit; native currencies stay intact.
- `source_precedence`: source order and scoped conflict-resolution decisions.
  Missing precedence remains `conflict`, not last-arriving-source wins.
- `freshness_threshold`: connection-specific threshold, unit and evidence;
  distinguish source refresh from financial as-of age. Missing is `not_assessed`.
- `withholding`: explicit owner treatment with evidence, otherwise `unresolved`.
- `commitments`: actual owner-confirmed commitments, amounts/units, schedule,
  timezone/calendar, uncertainty and evidence, or explicit confirmation of none.
  Do not invent deadlines or turn forecasts into facts. This is input collection,
  not a spending commitment or instruction to post accounting entries.

Every answer is an object with `status` equal to `unknown`, `conflict`,
`not_assessed`, `unresolved`, or `confirmed`. Only `confirmed` carries `value`, a
nonblank UTF-8 string of at most 4096 bytes. Unknown states carry no value. Missing
component answers default to `unknown`; missing meaning answers default to
`unknown` except precedence/freshness/withholding as specified above. Explicit
uncertainty is preserved. The schema documents structure; runtime additionally
checks cross-list identity, UTF-8 byte limits, and yes/no automation answers.

Confirmed text is a recorded answer, **not semantic validation** of dates,
fractions, currencies, source orders or commitments, and not evidence of consent.
Later owning subsystems must validate and adopt those values through their C02
and subsystem contracts. This input format does not extend C02 or bypass C16.
The unresolved-field report names each remaining field, its distinct state and
next action; it never fabricates an owner answer to make readiness green.
