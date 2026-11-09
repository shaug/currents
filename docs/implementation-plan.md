# Currents Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the complete local-first household financial record and native Mac experience defined by the constitution, with reliable evidence acquisition, inspectable accounting and conditional account forecasts.

**Architecture:** A host-independent Rust engine owns domain history, validated operations and replay. A private pinned doubleentry adapter and three narrowly reviewed storage seams compose all authoritative consequences in one SQLCipher transaction. A signed Swift owner process provides keys, XPC and owner approval to native Mac, CLI and restricted agent clients; an iOS satellite acquires evidence only.

**Tech Stack:** Rust 2024, doubleentry 0.9.0, SQLx 0.9.0, SQLCipher 4, Swift 6/SwiftUI/AppKit, bounded versioned JSON/C ABI, focused commodity libraries in [dependencies](planning/dependencies.md).

**Spec:** [Technical design](technical-design.md), SHA-256 `03cd2988bac8e02bd5757f45196aac6efae5f57a56806ce3d33857892c68e5b4`; [architectural constitution](constitution.md), including section 8 institutional read-only access.

**Status:** Proposed implementation work and GitHub-ready issue payloads, not application implementation or production qualification. The design's historical spike results do not establish that these future gates pass. This plan is a separate document; the original constitution/design are not altered. Independent plan-review closure is recorded separately by the coordinating reviewer.

## Global Constraints

- The present financial position is primary. Full household scope includes cash, cards, debt, investments, manual assets/liabilities, Apple Card, organization/review/history/reconciliation and one/three/twelve calendar-month account forecasts. No synthetic slice is called a usable household MVP.
- Institutions are read-only: no payments, transfers, trades or institution-held financial-state changes. Local auditable recognition is different from executing a financial transaction.
- Preserve evidence, receipt/source/parser revisions and meaningful temporal precision. Import does not recognize money. Similarity is a proposal, not identity. Active source-facet and economic-leg ownership both conserve exact amounts.
- One total domain commit order. Evidence/decisions/native consequences/result/invalidation are atomic. Replay frozen decisions, not current inference, policies, prices, random IDs or wall clock. Compare canonical native history and full domain/posting hashes, not just ending balances.
- One `SqlStore<Sqlite,18>`; exact i128 coefficient with scale 0..18 and unit divisibility, checked widened intermediate arithmetic, no binary float. USD exponent 2; half-even cash/valuation quantization with explicit remainder; split remainder ties use stable component ID. No rounding plug for unbalanced input.
- One native unsealed period `0001-01-01` through `9999-12-31`, floor-dated metadata/fixed policy, immutable opaque account paths and permanent collision-checked unit codes. Preserve nullable actual dates and explicit bookkeeping fallback; reversals set original booking and value dates. No native financial seals/closes.
- Metadata declarations commit before recognition preview. Native batch limit 1,000, entry postings 1,024, Documents 16, canonical leaf 65,535 bytes; Currents atomic closure at most 20,000 operations/64 MiB. Never split a correction into partial financial commits.
- Ordinary evidence file limit 100 MiB, chunks/structured messages 1 MiB, normalized observations 10,000 per commit. Full archive defaults 64 GiB/10 million logical records/1 MiB chunks; separate validated override, never ordinary-file limit reused silently.
- Forecasts/planning are separate portable authority with no journal-write capability. Unknown openings, rates, ownership, histories and basis never become zero. Source assertion, journal and selected aggregate remain distinguishable.
- Production profile fails closed on encryption/authentication/integrity/recovery gaps. SQLCipher raw 256-bit key, encrypted 4096-byte pages, HMAC SHA-512, WAL/FULL and no disk temp stores. No plaintext fallback, telemetry, automatic cloud sync, remote AI, financial diagnostics or private public fixtures.
- Default lock is explicit/OS session lock/15 minutes inactivity. Five-minute one-use owner preview. Only confirmed predispatch cancellation or rollback proves no commit; commit-submission races remain pending/unknown until audit/status resolution.
- Retain committed evidence/decisions/saved forecasts and all successful backup generations by default. Transient failed staging may expire after seven days. Backups due after changed-data 24 hours and on unlock; no silent pruning. Whole-dataset destruction is explicit and cannot promise inaccessible-copy or physical SSD erasure.
- Rust release qualification starts at 1.99.0; initial MSRV candidate 1.94.0 must be proved. Edition 2024/resolver 3. macOS 15.0 arm64 and iOS 18.0 are conservative qualification proposals, not already tested claims. Linux x86_64 headless core/verification required. No Intel Mac support implied.
- Private crates, committed lockfiles, pinned native sources/tools/actions, safe-crate unsafe prohibition, release overflow checks and unwind containment. No mandatory server, cloud backend, plugin platform, tax optimization, budgeting system or AI dependency.
- Current work is documentation only. Later implementation, live account access, credential provisioning, signing, remote publication and new agreements have their own authorization requirements. Qualification tickets specify exact prerequisites and disabled behavior, not invented approvals or deadlines.

## Review Focus

- An old export or new parser can bypass deduplication unless source-facet ownership survives both revision lineages. CUR-015 tests same fact/new parser/new leg/new command and valid multifacet payroll.
- A successful native write followed by response-copy failure, key lock or COMMIT I/O error can be accidentally called rollback. CUR-010/034 test durable postdispatch failures, admission/status races and identical retry after reconnect.
- A backfill/cost correction can preserve balances while changing the wrong dates or leaving counterpart/gain/claim stale. CUR-016/022 test whole-entry closure, explicit reversal dates, unknown cohorts and all affected dependencies.
- A source balance can already include an incoming transfer while its feed is absent; naive clearing/forecast arithmetic double counts it. CUR-018/026 test inclusion alternatives, ranges/indeterminate state and per-planned-leg consumption.
- A verified archive can still depend on an unshipped WAL or activate before its key/directory is durable. CUR-029/031 test isolated single-file reopen and every generation/key/marker fault boundary.

## How to use the deliverables

1. Read the design and this plan. [Shared contracts](planning/contracts.md) fixes exact names, schema ownership, files and boundaries; [dependency decisions](planning/dependencies.md) fixes libraries and qualification procedure.
2. Pick a ticket whose **hard predecessors are done**. Each body includes why, scope/non-goals, files, consumed/produced contracts, positive and counterexample assertions, verification command, gate inputs, measurable done and the same seven-step test-first/review/check-in checklist. The [manifest](planning/manifest.json) is the machine-readable publication input.
3. Build tests first, record red, implement minimally, run green and affected regressions, obtain review, then make a focused check-in only during authorized implementation. Do not copy a spike wholesale or claim future tests already passed.
4. When a new external prerequisite is unavailable, record `Blocked` with exact required input and leave that feature disabled. Do not disable unrelated synthetic work, infer a private value, or silently relax scope. Separate research/mock completion from actual production qualification.
5. Do not start application implementation from this plan until separately authorized. Issue publication is a separate operation after the target repository and publication scope are confirmed.

The 58 leaf tickets are sized by independently reviewable contracts, not by a deadline. Each has one accountable role; a person is assigned at kickoff, not guessed here. CUR-043 is the selected-provider adapter template: if inventory requires more than one provider, instantiate one child per provider before implementation and retain its common acceptance umbrella. Native workflows are split into position/review and financial/maintenance journeys; every task still owns specific view files and tests.

## Workspace and file responsibilities

| Location | Responsibility | First owner |
|---|---|---|
| Cargo/workspace/xtask/tools and .github | Reproducible toolchain, dependency direction, local commands and secure CI | CUR-001/003 |
| crates/currents-types, currents-protocol; schemas/v1 | Pure stable IDs, exact values, records, wire schemas/errors/canonical bytes | CUR-002 |
| crates/currents-domain | Pure financial interpretation, associations, corrections, positions, claims, durable meaning and replay application | CUR-011–024 |
| crates/currents-accounting | Sole native accounting caller; frozen batches, numbers/date/catalog adaptation | CUR-009 |
| vendor/doubleentry; patches/doubleentry | Pinned upstream source plus independently reviewed atomic/key-init/multi-op patches | CUR-005/006/007 |
| crates/currents-storage | SQLCipher/transactions/staging/history/audit/archive/generation/snapshot/backup | CUR-006/008/011/029–033 |
| crates/currents-service | Admission, authorized command orchestration, pure-domain execution, storage replay orchestration | CUR-010/011/036 |
| crates/currents-planning | Pure commitments/calendar/matches/forecasts/recurrence; no accounting/storage authority | CUR-025–028 |
| crates/currents-acquisition | Bounded parsers/read-only sources/mobile evidence ingestion; no dataset key | CUR-012/041–046 |
| crates/currents-ffi; include | Small versioned C ABI, private host key/session injection, generation handles and owned buffers | CUR-034 |
| native/CurrentsPlatform; native/Currents.xcodeproj | Swift actor/XPC owner, Keychain, owner helper/approval and lifecycle | CUR-004/032/035/036 |
| native/CurrentsApp | Native views/controllers; command/query translation, never alternative financial math | CUR-037/038 |
| native/CurrentsCapture | Protected iOS capture-only queue, FinanceKit eligibility, pairing/file bundles | CUR-044–046 |
| currents-cli/currents-mcp/currents-verify | Authorized client, restricted stdio agent bridge, independent offline archive verification | CUR-029/039/040 |
| fixtures/tests/fuzz/docs | Synthetic scenario/property/fault/oracle corpus, schemas/ADR/runbooks and public explanations | Every owning ticket; CUR-049/050/055 |

Domain functions consume types-owned validated evidence/decisions, never SQLx/storage handles. The service resolves private encrypted staging handles and orchestrates replay into storage; pure domain ReplayEngine only applies frozen records. Consumer work may reference contracts before implementation, but no task may merge against a guessed substitute. A schema/signature change updates owner contract, vectors, all consumers and dependency graph in the same reviewed change.

## Milestones, epics and parallel paths

Milestones are completion gates, not a forced single-file waterfall. After M0, source/parser work and native platform proof can progress alongside storage. After M1, financial semantics, planning, and recovery proceed independently where the DAG permits. UI can build against exact contract fixtures but may not claim integrated acceptance before its hard dependencies pass. Owner inventory/provider/FinanceKit research can run during synthetic engineering; their unresolved decisions block only the named production paths and final usable-release gate.

The main technical critical path is CUR-001→002→005→006→007/008→009→010→011/013→016→029→031/032→034→035→036→038→053→054. The complete graph, including parallel accounting/organization/investment/planning and acquisition routes, is in the manifest. This illustrative path does not replace depends_on.

A parent epic is not a predecessor of its child; it simply groups work. Only `depends_on` creates finish-to-start blocking edges. `related_to` is nonblocking context. Milestones are assigned to leaf issues; epics spanning milestones have no invented milestone or due date.

<!-- Generated milestone and ticket index follows; renderer maintains this block. -->

### M0: Qualify foundations and scope

Pinned tools/contracts/secure CI/native target qualification and explicit household input contract. Owner-private input can remain blocked without halting synthetic work.

**Objective exit:** CUR-001–004 pass engineering checks; CUR-047 template passes and unresolved owner inputs explicitly listed. No live account prerequisite to M1.

| Ticket | Epic | Hard predecessors | Outcome |
|---|---|---|---|
| [CUR-001](planning/issues/CUR-001.md) | E01 | None | Qualify workspace and dependency pins |
| [CUR-002](planning/issues/CUR-002.md) | E01 | CUR-001 | Define versioned records and contracts |
| [CUR-003](planning/issues/CUR-003.md) | E01 | CUR-001, CUR-002 | Establish secure pull request checks |
| [CUR-004](planning/issues/CUR-004.md) | E07 | CUR-001 | Qualify native platform targets |
| [CUR-047](planning/issues/CUR-047.md) | E09 | CUR-002 | Define household scope and meaning inputs |

### M1: Prove encrypted synthetic slice

Three separately qualified storage seams, ordered authority, exact accounting, commands/replay and bounded import/recognition.

**Objective exit:** CUR-005–014 pass on Mac arm64/Linux x86_64 including encrypted1,200-op/fault/status and exact canonical replay. Synthetic-only developer preview; not MVP.

| Ticket | Epic | Hard predecessors | Outcome |
|---|---|---|---|
| [CUR-005](planning/issues/CUR-005.md) | E02 | CUR-001, CUR-002 | Preserve atomic composition seam |
| [CUR-006](planning/issues/CUR-006.md) | E02 | CUR-005 | Qualify encrypted connection initialization |
| [CUR-007](planning/issues/CUR-007.md) | E02 | CUR-005, CUR-006 | Support atomic multi-batch closures |
| [CUR-008](planning/issues/CUR-008.md) | E02 | CUR-002, CUR-006 | Persist ordered evidence and decisions |
| [CUR-009](planning/issues/CUR-009.md) | E03 | CUR-002, CUR-005, CUR-008 | Freeze exact native accounting batches |
| [CUR-010](planning/issues/CUR-010.md) | E02 | CUR-007, CUR-008, CUR-009 | Implement serialized command lifecycle |
| [CUR-011](planning/issues/CUR-011.md) | E02 | CUR-008, CUR-009, CUR-010 | Verify and rebuild complete history |
| [CUR-012](planning/issues/CUR-012.md) | E08 | CUR-002, CUR-008, CUR-010 | Import bounded files without recognition |
| [CUR-013](planning/issues/CUR-013.md) | E03 | CUR-009, CUR-010, CUR-012 | Recognize explicit financial activity |
| [CUR-014](planning/issues/CUR-014.md) | E09 | CUR-003, CUR-011, CUR-013 | Prove encrypted synthetic vertical slice |

### M2: Complete financial semantics

Associations/corrections/reconciliation, present positions/organization/history, investments/FX/loans/manual and complete forecasts.

**Objective exit:** CUR-015–028 pass all financial counterexamples and S1–S10 engine fixtures; no financial scope postponed beyond usable MVP.

| Ticket | Epic | Hard predecessors | Outcome |
|---|---|---|---|
| [CUR-015](planning/issues/CUR-015.md) | E03 | CUR-012, CUR-013 | Associate source facets and economic legs |
| [CUR-016](planning/issues/CUR-016.md) | E03 | CUR-007, CUR-013, CUR-015 | Correct recognitions and replace segments |
| [CUR-017](planning/issues/CUR-017.md) | E04 | CUR-015, CUR-016 | Evaluate scoped reconciliation claims |
| [CUR-018](planning/issues/CUR-018.md) | E04 | CUR-013, CUR-015, CUR-016, CUR-017 | Select honest household positions |
| [CUR-019](planning/issues/CUR-019.md) | E03 | CUR-010, CUR-015, CUR-016 | Preserve organization and rule authority |
| [CUR-020](planning/issues/CUR-020.md) | E04 | CUR-017, CUR-018, CUR-019 | Build review inbox and history reports |
| [CUR-021](planning/issues/CUR-021.md) | E03 | CUR-009, CUR-015, CUR-016 | Track investment quantities and actions |
| [CUR-022](planning/issues/CUR-022.md) | E03 | CUR-021, CUR-016 | Allocate known and unknown investment basis |
| [CUR-023](planning/issues/CUR-023.md) | E03 | CUR-009, CUR-018, CUR-021 | Validate FX and valuation conversion |
| [CUR-024](planning/issues/CUR-024.md) | E03 | CUR-013, CUR-018 | Represent loans and manual positions |
| [CUR-025](planning/issues/CUR-025.md) | E05 | CUR-002, CUR-010 | Persist commitments and calendar expansion |
| [CUR-026](planning/issues/CUR-026.md) | E05 | CUR-015, CUR-018, CUR-025 | Conserve forecast actual and pending matches |
| [CUR-027](planning/issues/CUR-027.md) | E05 | CUR-023, CUR-024, CUR-025, CUR-026 | Generate reproducible household forecasts |
| [CUR-028](planning/issues/CUR-028.md) | E05 | CUR-019, CUR-020, CUR-025, CUR-026, CUR-027 | Suggest recurrence and rolling spending rates |

### M3: Protect and recover private records

Portable archives, snapshots, generation activation, keys/locking, backup/retention and privacy checks.

**Objective exit:** CUR-029–033 and049 pass full-replay/offline lost-device/hostile-input/fault tests with real-data gate still off until signed host and owner setup.

| Ticket | Epic | Hard predecessors | Outcome |
|---|---|---|---|
| [CUR-029](planning/issues/CUR-029.md) | E06 | CUR-006, CUR-011, CUR-016, CUR-019, CUR-027 | Build self-contained encrypted archives |
| [CUR-030](planning/issues/CUR-030.md) | E06 | CUR-011, CUR-027, CUR-029 | Verify disposable replay snapshots |
| [CUR-031](planning/issues/CUR-031.md) | E06 | CUR-029, CUR-030 | Activate safe migration and key generations |
| [CUR-032](planning/issues/CUR-032.md) | E06 | CUR-004, CUR-006, CUR-010, CUR-031 | Enforce key custody and lock lifecycle |
| [CUR-033](planning/issues/CUR-033.md) | E06 | CUR-029, CUR-031, CUR-032 | Manage backup protection and retention |
| [CUR-049](planning/issues/CUR-049.md) | E09 | CUR-003, CUR-006, CUR-012, CUR-029, CUR-032 | Audit privacy and hostile input surfaces |

### M4: Ship coherent native clients

Persistent FFI, XPC, owner authority, complete Mac UX, CLI/offline reader and restricted optional MCP.

**Objective exit:** CUR-034–040 and056 pass parity/accessibility/retry/scope tests on qualified platforms; signed trust certification finishes in M6. No web shell or independent client math.

| Ticket | Epic | Hard predecessors | Outcome |
|---|---|---|---|
| [CUR-034](planning/issues/CUR-034.md) | E07 | CUR-004, CUR-010, CUR-011, CUR-032 | Harden persistent Rust Swift boundary |
| [CUR-035](planning/issues/CUR-035.md) | E07 | CUR-004, CUR-032, CUR-034 | Authenticate owner process and XPC sessions |
| [CUR-036](planning/issues/CUR-036.md) | E07 | CUR-010, CUR-019, CUR-035 | Enforce scoped authority and owner approval |
| [CUR-037](planning/issues/CUR-037.md) | E07 | CUR-018, CUR-020, CUR-024, CUR-035, CUR-036 | Build native position and review experience |
| [CUR-038](planning/issues/CUR-038.md) | E07 | CUR-017, CUR-019, CUR-022, CUR-023, CUR-027, CUR-028, CUR-033, CUR-037 | Complete native financial workflows |
| [CUR-039](planning/issues/CUR-039.md) | E07 | CUR-029, CUR-035, CUR-036 | Expose CLI and offline verification |
| [CUR-040](planning/issues/CUR-040.md) | E07 | CUR-036, CUR-039 | Restrict local MCP proposals and reads |
| [CUR-056](planning/issues/CUR-056.md) | E07 | CUR-032, CUR-035, CUR-036, CUR-039, CUR-040 | Qualify signed owner host before private data |

### M5: Qualify household acquisition

Read-only refresh/provider coverage, Apple entitlement/mobile evidence and owner-private activation.

**Objective exit:** CUR-041–046,048,057,058 pass per-account coverage/consent/recovery gates or permitted owner-accepted limitations recorded. Missing Apple/provider feasibility remains visible release blocker.

| Ticket | Epic | Hard predecessors | Outcome |
|---|---|---|---|
| [CUR-041](planning/issues/CUR-041.md) | E08 | CUR-012, CUR-015, CUR-032, CUR-036 | Coordinate read-only source refreshes |
| [CUR-042](planning/issues/CUR-042.md) | E08 | CUR-041, CUR-047 | Qualify provider coverage and consent |
| [CUR-043](planning/issues/CUR-043.md) | E08 | CUR-041, CUR-042, CUR-048 | Implement qualified provider adapters |
| [CUR-044](planning/issues/CUR-044.md) | E08 | CUR-004, CUR-047 | Qualify Apple Card acquisition route |
| [CUR-045](planning/issues/CUR-045.md) | E08 | CUR-029, CUR-031, CUR-032, CUR-041 | Secure mobile bundles and acknowledgments |
| [CUR-046](planning/issues/CUR-046.md) | E08 | CUR-004, CUR-045, CUR-058 | Build protected iOS capture satellite |
| [CUR-048](planning/issues/CUR-048.md) | E09 | CUR-033, CUR-036, CUR-038, CUR-047, CUR-049, CUR-035, CUR-056 | Gate private dataset activation |
| [CUR-057](planning/issues/CUR-057.md) | E08 | CUR-044, CUR-046, CUR-048, CUR-056, CUR-058 | Verify protected Apple Card account coverage |
| [CUR-058](planning/issues/CUR-058.md) | E08 | CUR-004, CUR-006, CUR-029, CUR-045 | Qualify iOS encrypted transfer bridge |

### M6: Qualify complete household release

Deep properties/faults, performance, docs, SBOM/provenance, signed distribution and all journey acceptance.

**Objective exit:** CUR-050–055 pass exact candidate commit; S1–S12 and every inventory component/forecast horizon/protection gate satisfied. Explicit owner release approval still required before publication.

| Ticket | Epic | Hard predecessors | Outcome |
|---|---|---|---|
| [CUR-050](planning/issues/CUR-050.md) | E09 | CUR-016, CUR-017, CUR-018, CUR-022, CUR-023, CUR-028, CUR-030, CUR-031, CUR-034, CUR-036, CUR-040, CUR-045, CUR-049 | Exercise full faults and semantic properties |
| [CUR-051](planning/issues/CUR-051.md) | E09 | CUR-018, CUR-020, CUR-027, CUR-030, CUR-038, CUR-050 | Qualify performance without weakening truth |
| [CUR-052](planning/issues/CUR-052.md) | E09 | CUR-003, CUR-004, CUR-029, CUR-031, CUR-039, CUR-049 | Automate release metadata and provenance |
| [CUR-053](planning/issues/CUR-053.md) | E09 | CUR-004, CUR-035, CUR-036, CUR-038, CUR-046, CUR-050, CUR-052, CUR-056, CUR-044 | Qualify signed native distribution |
| [CUR-054](planning/issues/CUR-054.md) | E09 | CUR-014, CUR-017, CUR-018, CUR-020, CUR-022, CUR-023, CUR-024, CUR-028, CUR-033, CUR-038, CUR-039, CUR-040, CUR-043, CUR-046, CUR-048, CUR-050, CUR-051, CUR-053, CUR-055, CUR-057, CUR-058 | Accept all household release journeys |
| [CUR-055](planning/issues/CUR-055.md) | E09 | CUR-029, CUR-031, CUR-033, CUR-039, CUR-040, CUR-046, CUR-052 | Document independent recovery and compatibility |

## Epic ownership

- [E01: Establish reproducible project foundations](planning/issues/E01.md), accountable role Build engineer; 3 leaf tickets.
- [E02: Preserve atomic durable authority](planning/issues/E02.md), accountable role Storage engineer; 6 leaf tickets.
- [E03: Implement financial interpretation integrity](planning/issues/E03.md), accountable role Domain engineer; 9 leaf tickets.
- [E04: Explain present and historical position](planning/issues/E04.md), accountable role Query engineer; 3 leaf tickets.
- [E05: Build portable conditional forecasts](planning/issues/E05.md), accountable role Planning engineer; 4 leaf tickets.
- [E06: Protect and recover durable records](planning/issues/E06.md), accountable role Recovery engineer; 5 leaf tickets.
- [E07: Deliver native controlled clients](planning/issues/E07.md), accountable role Native engineer; 9 leaf tickets.
- [E08: Acquire reliable read-only evidence](planning/issues/E08.md), accountable role Acquisition engineer; 9 leaf tickets.
- [E09: Qualify complete household releases](planning/issues/E09.md), accountable role Quality lead; 10 leaf tickets.

## Test, CI and release policy

CUR-001 creates exact local commands and frozen tools. CUR-003 makes fast PR checks mandatory: format, compiler warnings, selected strict Clippy, rustdoc/doctests, unit/integration/property contracts, dependency direction, schema/header/migration drift, privacy and cargo-deny. Rust current/MSRV and Mac arm64/Linux x86_64 core storage lanes use locked production features, not indiscriminate all-features. CUR-004/034/035 add Swift 6 strict-concurrency and native ABI/XPC debug/release checks; a missing native runner is a blocked gate, not a skipped green check.

CUR-049/050 add nightly/weekly deeper properties, differential Ledger/Beancount fixtures, fuzzing, memory boundary tools, shutdown and disk/I/O/crash matrices with triage owners and scoped expiring exceptions. Miri cannot certify SQLite/XPC. Kill tests cannot prove physical power-loss durability. CUR-051 separately measures realistic performance without weakening accounting/encryption. Core correctness gates, not an arbitrary aggregate coverage percentage, decide acceptance.

CUR-052 owns changelog/release notes, lock/build/native-feature/source inventories, native-inclusive SBOM/licenses, signed provenance, offline/cache-free build and measured unsigned reproducibility. CUR-053 owns actual nested signing/notarization/entitlements and minimum/current-OS install/upgrade/restore after explicit credential setup approval. Production secrets never enter fork PR jobs; protected release jobs alone get necessary write/OIDC permissions. No automatic update service, crates.io/Homebrew/container publishing or deployment is part of initial release. CUR-055 makes independent record interpretation/recovery documented and testable.

Every gate command must create a QualificationReportV1 naming exact test functions, target/tool inputs, output digests, pass/fail/blocked state and residual limitations. The ticket command is a planned executable, not evidence that code already exists. Native gate runners are implemented in the listed xtask files and invoke the named native Tests/CURNNNTests.swift assertions through the qualified Xcode scheme/device matrix established by CUR-004. They must fail if any required assertion or device/signing prerequisite is missing.

## Owner inputs and disabled behavior

- CUR-047 collects exact household/component/ownership/currency/timezone and financial meaning inputs privately. Unknown/conflicting items stay unknown; no institution identity or deadline is guessed.
- CUR-042/043 require selected provider, actual usable read-only credential scopes, recipient/data/cost/terms/retention and coverage authorization. No live connector or unapproved backend exists until resolved. File/manual capability continues but is not called equivalent to reliable automation.
- CUR-044 establishes the entitlement/distribution route; CUR-046 implements protected capture and CUR-058 qualifies the iOS-only transfer build without private credentials. CUR-057 verifies actual Apple account coverage only after CUR-048/056 protection gates. File-only Apple Card is an explicit owner-accepted compromise, not an engineering substitute silently satisfying automatic acquisition.
- CUR-033/048 require independent encrypted backup destination, offline key custody and demonstrated restore; optional protected device key copy is separate consent. No live acquisition until mandatory protection succeeds.
- CUR-040 defaults external agents off. Owner-selected disabled configuration is valid; any enabled agent needs named recipient/fields/accounts. Remote AI remains outside this release.
- CUR-056 first qualifies the signed owner host with synthetic data before private activation; CUR-053 separately requires authorized secure signing/notarization and license/distribution decisions. Current CLT selection does not prove full Xcode absent. Qualification may remain blocked while synthetic work continues.
- CUR-054 ties every account/component and S1–S12 journey to exact candidate evidence. It cannot waive constitutional invariants or self-authorize publication. No dates or external assignees are invented.

## Traceability and issue publication

[manifest.json](planning/manifest.json) maps all 18 numbered invariants, the other constitutional sections/preamble, 12 acceptance journeys, 53 detailed functional/security requirements and nine project-quality requirements to concrete ticket owners. Its reverse maps and tests are checked locally; each issue links the relevant design section and exact shared contract. Coverage means planned ownership, not proof of implemented behavior.

The payload includes seven milestones, labels, nine epic bodies and 58 leaf issue bodies. Use [publication instructions](planning/README.md) only after the repository is verified and publication is authorized. Create all issues, map stable IDs to returned GitHub IDs/numbers, then add parent/sub-issue and blocked-by relationships. Do not infer a remote or publish with unresolved template IDs. Preserve a private publication journal for uncertain outcomes and check before retry. The DAG may be shown in GitHub without creating epic/child dependency cycles.

## Planning verification and review limits

Run `python3 docs/planning/author_payloads.py`, `python3 docs/planning/render.py`, then `python3 docs/planning/validate.py` for deterministic local payload checks. The authoring source is the editable planning definition; manifest and bodies are generated. Any reviewer edit to source requires rerendering and revalidation. Checks cover IDs, fields, parent membership, hard-dependency acyclicity, requirement coverage, package/test-file consistency, links/anchors, privacy markers and the converged design hash. HTTP link reachability and all application/production tests are separately unperformed unless explicit evidence says otherwise.

Author self-review checks spec coverage, exact file/type consistency, test counterexamples, failure/gate behavior and plan proportion. Independent adversarial reviewers still decide closure; this document does not declare its own convergence. Execution is intentionally not started by creating these payloads.
