> Historical proposal. Superseded by [technical-design.md](technical-design.md), which incorporates the comparative evaluation and integration evidence. Retained for decision history; its open architecture choices are not the current implementation contract.

# Currents: preliminary technical design

**Status:** Proposal for review, not an implementation plan

**Date:** 2026-11-09 UTC

**Governing document:** [Architectural constitution](constitution.md)

## 1. Purpose and decision status

Currents should answer a practical question: **What is my financial position now, how well do I know it, and what is likely to happen next?** Its durable financial record must remain useful when an institution revises an export, an interpretation changes, a client is replaced, or history is incomplete.

This document translates the constitution into testable boundaries and scenarios. It does not select an institution provider, programming language, database product, cloud service, or final schema. Proposed mechanisms below are technical recommendations, not decisions already made by the owner. There is no application implementation in this repository yet.

### Established requirements

- A client-independent financial engine owns financial semantics and validated operations. The primary client is genuinely native to macOS. CLI and controlled agent clients must be possible without duplicating financial rules.
- The present financial picture and the durable record are both core. Historical evidence supports that picture; future projections remain conditional.
- Institutional connections are read-only. Currents cannot initiate payments, transfers, or trades. It can record, reconcile, classify, and correct its own local financial record.
- Evidence, interpretations, accounting consequences, reconciliation, and forecasts have different authority. Neither a bank observation nor a forecast silently becomes a journal entry.
- The initial product spans cash, credit, debt, investments, and manual assets and liabilities. Organization and useful account-level forecasts are part of that product, not optional post-MVP features.
- Development is intended to be public. Actual financial state, credentials, backups, and private interpretations are not public development artifacts.

### Proposed baseline

Start with a local engine, one ordered decision history per household dataset, and a native Mac client. Treat the accounting journal and query models as derived products of recorded decisions. Use a shared planning module for portable commitments and forecast semantics; let clients own their presentation. Prove these boundaries with synthetic scenarios before connecting live institutions.

A small vertical slice validates the design. It is not the release definition. No additional constitutional decision is needed to write this proposal; implementation choices and integration access require later evidence and review.

## 2. Product scope and success

### Full-picture MVP

The first usable release should support all of the following, with explicit limits rather than fabricated completeness:

1. A household account inventory spanning cash, cards, loans, investment accounts, and manually observed assets and liabilities. An account with no usable integration remains representable.
2. Repeated imports or read-only connections with source provenance, coverage, duplicate handling, revision history, and visible stale/error states. Manual observations remain a first-class fallback.
3. A present-position view that distinguishes institution-reported values, journal-derived values, pending activity, valuation dates, and unresolved differences. It can be useful before history is complete.
4. A consistent local journal with traceable recognition, correction, split activity, internal transfers, and opening-position treatment.
5. A durable organizational overlay: normalized counterparties, categories, tags, annotations, and correctable splits, usable without AI.
6. Reconciliation with defined scope and cutoff, partial matches, exceptions, and later invalidation.
7. Basic holdings and valuation coverage without requiring tax-lot reconstruction. Missing cost basis, unknown historical activity, and stale prices remain visible.
8. Account-level cash-flow forecasts at approximately one month, three months, and one year, including recurring commitments, expected income, variable spending, and internal transfers. Users can inspect and change assumptions and compare saved forecasts with actual activity.
9. A native Mac workflow for inspecting evidence, reviewing interpretations, resolving differences, and examining forecasts. A headless client must demonstrate that essential operations and recovery do not depend on that interface.
10. Private storage, secure credential handling, portable export, backup, restore, and integrity verification demonstrated before trusting real records to the system.

Success is not measured by the percentage of transactions given a category. It is measured by whether a displayed amount can be explained, uncertainty stays visible, local corrections are recoverable, and a restored dataset produces the same financial meaning.

### Deliberate limits

This is a personal application, not a multi-tenant financial service. Institution-side transaction execution, corporate reporting, comprehensive accrual accounting, tax preparation, tax-lot optimization, autonomous financial decisions, real-time market trading, and mandatory AI are outside scope. Cross-device concurrent editing, hosted access, and a general plugin marketplace are not prerequisites for the first release.

No limitation permits hiding an unsupported currency, an unknown holding, a missing account, or a failed import. A feature can say it cannot yet account for something; it cannot quietly produce a complete-looking answer.

## 3. Design-driving scenarios

All amounts and institutions in these examples are synthetic. Each scenario defines a behavior to prove, not a proposed screen layout.

### S1. Authorization, posting, enrichment, and statement

A card reports a pending restaurant authorization of USD 80. Two days later it reports a posted charge of USD 96 including tip. An enrichment source supplies a merchant name. A statement later includes the USD 96 charge.

- Preserve all observations and their source identifiers, dates, precision, and revisions. Link them as a proposed lifecycle, not as one overwritten row.
- A pending authorization contributes to an explicitly pending exposure view. It is not booked as a final expense merely because it was imported.
- A recognition decision for the posted charge records balanced USD consequences: debit dining expense 96, credit card liability 96. The decision identifies the evidence and interpretation used.
- Merchant normalization and later categorization are separate decisions. An enrichment feed cannot rewrite the card's source text or financial amount.
- Statement reconciliation establishes a scoped claim supported by the statement and the journal version, not a permanent Boolean on the transaction.
- If the authorization cannot confidently be linked to the posted charge, show unresolved pending exposure rather than silently subtracting it twice or discarding it.

**Acceptance:** Reimport is idempotent; the journal has one recognized charge; original and corrected beliefs are queryable; removing a projection and replaying decisions reproduces the same journal.

### S2. Delayed and split internal transfer

A checking account reports an outflow of USD 1,000. A brokerage reports incoming USD 600 and USD 400 on later dates. One leg may be absent when the first import occurs.

- Associations support multiple observations and financial legs. Matching can consider amount, time window, account relationship, and identifiers, but confidence alone does not settle identity.
- When evidence supports a transfer but the receiving leg is not available, a recognition can debit an explicit transfer-clearing asset 1,000 and credit checking 1,000. Receipt of 600 and 400 debits brokerage cash and credits clearing in the same amounts. This clearing balance represents an unresolved leg; it does not claim a fabricated destination deposit.
- If transfer intent itself is unknown, preserve an unresolved outflow classification instead of assigning a known transfer counterparty. Later correction records the change.
- Linking must not recognize the original outflow a second time. Residuals, fees, and currency differences are separate explanations, not automatic balancing plugs.
- Household spending excludes supported internal movements. Account cash-flow views still show each leg on its applicable date. Any in-transit amount is separately labeled and included at most once.

**Acceptance:** Observations may arrive in any order; partial settlement remains visible; household spending is not inflated; transfer-clearing does not disappear because the destination feed is stale.

### S3. A fragmented paycheck

Synthetic payroll evidence reports gross pay 5,000, withholding 1,000, retirement contribution 500, HSA contribution 200, and net checking deposit 3,300. The deposit arrives first. The retirement and HSA observations arrive later, and a pay stub may never be available.

- A net deposit alone cannot establish gross pay or the size and identity of deductions. It can support a qualified income recognition with an explicit limitation, or remain unresolved under the selected recognition policy.
- A confirmed complete decomposition could debit checking 3,300, retirement asset 500, HSA asset 200, and withholding expense 1,000, and credit wage income 5,000. Withholding classification is a local reporting policy, not a claim that tax liability is final. An employer contribution, if present, needs separate evidence and treatment.
- When a prior net-income interpretation is replaced by that decomposition, append a linked correction. Do not book 3,300 of income a second time alongside 5,000 of gross income.
- A contribution confirmed by payroll but not yet visible in the investment feed can remain in an explicitly described receivable or clearing position. A payroll association can be partially supported without every component being recognized.

**Acceptance:** Partial evidence does not imply invented gross income. A later decomposition is balanced, avoids duplicate income, and preserves the original interpretation. The user can inspect which components are reported, inferred, and absent.

### S4. Current authority with incomplete history

A bank reports a current ledger balance of USD 10,000 but supplies only 90 days of activity. A brokerage supplies current holdings with no acquisition history. A manual property valuation is older than either feed.

- Retain each balance or holding assertion with its effective date, semantics, coverage, and source. A bank's available balance is not interchangeable with its ledger balance or a statement closing balance.
- Show the institution's scoped current assertion alongside the journal-derived amount and unexplained difference. A difference is not a reason to post an adjustment automatically.
- If the user chooses to establish an opening accounting position, record a specific anchoring decision referencing the source and cutoff. Use disclosed opening equity or an explicit unresolved opening position, never fictitious historic income or purchases. Future or backfilled evidence does not silently overlap the anchor.
- Holdings may have known quantities and current market value while cost basis and performance history remain unknown. Do not fabricate purchases to populate an investment register.
- For present net worth, select one supported representation per account/component. Do not add the reported total to the ledger total, or add a brokerage's total value to its already included holdings and cash.

**Acceptance:** The app gives a useful present picture with coverage and freshness attached, and cannot present an incomplete spending history or unknown investment return as a complete result.

### S5. Revising a historical decision

A charge recorded as an expense is later identified as reimbursable, or a provider revises its amount after reconciliation.

- Preserve the original observation, the prior decision, and the newer evidence or user judgment.
- Distinguish an organizational reclassification with no accounting effect from an accounting correction that needs reversal and replacement or explicit balanced delta entries.
- Query both “what we currently understand about that period” and “what we believed at a prior evidence-visibility and decision cutoff.” A backdated effective date does not erase when evidence arrived or a correction was made.
- Mark affected reconciliation claims as requiring review; retain the original claim with its original scope and inputs.

**Acceptance:** Earlier reports remain reproducible, corrected reports explain their changed basis, and neither the journal nor an adopted category is silently rewritten.

### S6. A merchant changes processors

Two different raw card descriptions refer to the same coffee merchant. A rule proposes the same normalized merchant; one purchase is actually equipment rather than coffee.

- Keep source descriptions. Version alias relationships and classification rules. Store why a proposal was made and whether it was accepted by a user or an explicitly configured rule.
- Merchant identity does not force a category. Categories, tags, notes, and item splits have separate provenance and override behavior.
- A rule change can generate new proposals. It cannot retroactively mutate confirmed decisions during replay. User overrides survive later import and enrichment.

**Acceptance:** The user can answer “how much did I spend on coffee?” and trace exceptions, undo an adopted interpretation through history, and rebuild the same overlay in a second client.

### S7. Forecast without fictitious money

Expected pay, rent, a credit-card payment, variable spending, and an internal savings transfer affect balances over the next 30, 90, and 365 days.

- Begin from a saved present-state basis, including its date, account coverage, pending exposure, and unresolved differences. Distinguish an asserted bank starting balance from a ledger starting balance.
- A commitment has a schedule, amount or range, affected accounts, uncertainty, and provenance. It is a durable plan, not a journal entry.
- Model paired transfer legs together. They affect individual accounts without becoming household income or expense. A card purchase affects liability and spending; paying the card reduces bank cash and the liability, not spending a second time.
- Reconcile expected occurrences with pending and posted actual activity by explicit associations. Reforecasting must not include an actual event and its still-expected occurrence twice. Passage of a due date alone never posts a financial fact.
- Save forecast inputs, engine version, assumptions, horizon, and outputs when a comparison is requested. A later forecast does not replace what was predicted earlier.

**Acceptance:** A visible downside/base/upside range reflects stated assumptions, not an unsupported probability guarantee. Missing data is disclosed. No forecast command writes recognized accounting consequences.

### S8. An alternate client and a damaged projection

A user categorizes an entry in the Mac app, inspects it through a CLI, and asks an agent client to propose a correction. A crash or damaged cache then requires rebuilding.

- Each client uses the same command validation and query contract. None writes raw journal tables. Read-only agent access is the default; locally approved mutations have explicit capabilities.
- Restore the authoritative dataset and rebuild projections without launching the Mac app or contacting institutions. The agent's proposal is not an adopted decision unless it passes the applicable approval and validation path.
- Missing evidence blobs, an unknown event version, ordering gaps, or a mismatched snapshot make the dataset visibly degraded. Do not skip unreadable history to present a healthy balance.

**Acceptance:** All clients observe the same durable interpretation and provenance. Full replay and verified snapshot replay agree. Failure is explicit and recoverable without rewriting source history.

## 4. Authority, modules, and ownership

The dependency direction is clients and adapters toward shared domain contracts. The engine must not import a graphical UI framework or rely on a view model to decide accounting behavior.

| Area | Owns | Does not own |
|---|---|---|
| Source adapters | Read-only fetching, source capability descriptions, raw evidence capture, import checkpoints | Recognition, reconciliation decisions, institution-side execution |
| Evidence store | Original observations, revisions, attachments, source identity and coverage | A single canonical transaction or preferred financial interpretation |
| Financial engine | Account identities, validated commands, ordered decisions, recognition, corrections, associations, reconciliation, organizational meaning | Native UI state, institutional credentials as domain data |
| Accounting projector | Deterministic balanced journal derived from adopted decisions | New interpretation, live exchange-rate lookup, presentational assumptions |
| Query models | Registers, present position, explanations, review queues, history views | Independent financial authority |
| Planning module | Portable commitments, recurrence rules, scenario assumptions, forecast computation and saved forecast runs | Observations or recognized accounting activity |
| Mac client | Native navigation, interaction, selection, local view preferences, accessible explanation | A private copy of accounting logic or sole ownership of user meaning |
| CLI and agent adapters | Typed command/query translation, capabilities, explicit approval presentation | Direct storage mutations or broader institutional authority |
| Storage/security adapters | Transactions, file access, key management, backup transport | Alternative domain semantics |

This is a responsibility map, not a mandate for separate services, repositories, or databases. One local process and one transactional store may implement most boundaries initially.

### Durable data versus client state

Durable financial knowledge includes recognized decisions, accepted merchant mappings, categories, tags, notes, reconciliation scopes, account identity mappings, and confirmed future commitments. It is stored and exported independently of the Mac UI. Saved forecast assumptions belong to the portable planning layer even if the initial UI is their only editor.

Window geometry, current selection, column widths, and temporary filters may remain Mac-specific. A saved financial rule or named scenario is not merely view state. Experimental charts can be client-specific; financial semantics that another client needs must move behind a shared contract before that client depends on them.

## 5. Evidence and the ordered decision history

### Capture before interpretation

An ingestion attempt records source, connector and parser version, account scope, request coverage, receipt time, result status, and completeness or truncation signals. Each observation preserves source-native identifiers and raw representation when available, along with normalized fields and the loss or uncertainty introduced by normalization.

Identical repeated retrieval can point to the same immutable evidence content while recording a new receipt/checkpoint. The same provider ID with changed content is a revision, not an overwrite. Similar amounts and descriptions across providers are candidate associations, not duplicate identities. A provider's deletion signal is retained as evidence of that signal; it does not erase an earlier observation or automatically reverse a journal entry.

Evidence receipts also need a durable ingestion order independent of decision order. A source assertion can affect the present view before anyone adopts an accounting decision. Therefore a reproducible read basis is a consistent pair of **evidence visibility cutoff and decision cutoff**, together with the applicable selection-policy and projection versions. A later-arriving, backdated observation belongs only to read bases that include its receipt; it cannot change what an earlier view was able to know. Deduplicating identical payload bytes must not discard this receipt history. Capture these cutoffs from one consistent storage snapshot, or an explicitly equivalent coordinated boundary.

Evidence has to be durable before a decision references it. If attachments are stored separately, ingestion stages and verifies the bytes, atomically publishes the manifest reference, and detects incomplete finalization after a crash. Orphan staging objects can be reclaimed without deleting referenced evidence. A raw payload containing secrets must be scrubbed at capture using an explicit source contract; secret authentication material is never part of the evidentiary record.

### Proposed ordering model

Use one monotonically ordered domain-decision sequence per household dataset initially. Source feeds can be fetched concurrently, but a single transaction boundary orders adopted decisions. The scale is one household, so a global order is simpler to replay and reason about than multiple authoritative streams with distributed ordering.

The design does not require a network server. Multiple processes must nevertheless share one writer/serialization policy, not hope that file access is coordinated. A local owner process, a lock around transactional commands, or database-supported transactions are candidates; evaluate them before choosing the runtime model.

### Information a durable decision must carry

Exact field names and serialization are open. Semantically, a decision needs:

- Stable dataset and decision identity, total-order position, event type, and schema/semantic version.
- Decision time and actor, including whether adoption was user-driven or an explicitly configured automated policy.
- Command identity for retry safety, applicable expected revision, and links to the evidence, prior decisions, and interpretations it relies on.
- Economic/effective dates, source posting dates where relevant, original precision and timezone information, and uncertainty. Dates without times must stay dates, not be invented midnight instants.
- The adopted result, including complete accounting instructions when there is an accounting effect. Persist exact amounts, units, account references, dates, stable posting identifiers, and governing conventions needed to replay them.
- The policy/rule/model version that produced a proposal and the reason for adoption. Model explanations are supporting context, not authoritative evidence.
- Supersession, correction, or revocation links where applicable. Earlier decisions remain addressable.

Do not event-source every mouse movement or disposable cache update. Record the decisions needed to recover financial meaning. An initial schema should emerge from S1-S8, not from a speculative universal financial ontology.

### Commands, retries, and atomicity

A command checks capability, validates inputs and references, detects stale expected revisions, and computes its consequences before committing. A preview identifies the dataset revision and exact proposed effect. Execution revalidates that basis; it does not commit a stale preview just because the UI previously showed approval.

Commit the decision and its accounting projection/checkpoint atomically when they share a transactional store. If analytical projections lag, return their processed sequence explicitly. Command identity plus an input digest makes retry return the original result; reusing an identity with different input is an error. A client that loses a response queries command status before retrying with a new identity.

Accounting consistency is mandatory at commit. A storage failure must leave either the complete prior state or the complete new state. A later cache/rebuild failure can degrade query availability, but cannot be concealed as a current, validated result.

## 6. Accounting and incomplete knowledge

### Monetary rules

Use exact decimal or scaled integer amounts with explicit units and rounding policy. Binary floating-point must not determine financial balance. Monetary scale is currency-specific; security quantities may need a different exact scale. Reject overflow, unsupported precision, and ambiguous units rather than silently rounding them away.

Every committed journal entry must balance under an explicit accounting convention. The initial proving scenarios use one currency. Multi-currency support needs a separately demonstrated policy: preserve native amounts and dated exchange-rate evidence, represent conversion through balanced currency-specific legs or an explicit functional-currency convention, and make fees/rounding visible. Simply adding USD and EUR together is invalid. Unsupported currencies remain visible as evidence and qualified present-state components instead of being silently excluded or given an invented conversion.

### Unknown is a state, not a guessed expense

The engine may recognize a known asset/liability movement while carrying an explicit unresolved counter-entry or clearing account. That entry must disclose what is known and what remains unexplained. Such accounts are useful review surfaces, not hidden devices to make reconciliation pass. Aging and unresolved amounts are visible. A difference between institution and journal alone is insufficient authorization to post one.

Opening positions are deliberate boundary assertions. An anchoring decision defines its effective cutoff, source, included accounts/components, and which later movements apply. When older transactions arrive, the engine must detect overlap with the anchor and propose a revised opening interpretation or a separate historical segment. It must never count both the opening balance and the same pre-cutoff activity.

### Journal reconstruction and corrections

Replay recorded adopted consequences, not today's classification rules. A rule or AI-model upgrade may propose reinterpretation, but changes require new decisions. The projector must not consult the network, current time, mutable UI settings, or live prices during replay.

Corrections append explicit balanced reversals/replacements or deltas with links to the corrected decisions. The final choice between those forms should favor simple, inspectable histories in the acceptance fixtures. Reproducing a previous accounting belief uses a decision-sequence cutoff; reproducing the complete position view also pins evidence visibility and policy versions. Reporting a period uses economic/effective time as well. Neither replaces the recorded knowledge cutoffs.

### Investments without invented history

Separate security identity, observed position quantity, cash balance, price observation, market valuation, and known acquisition/cost-basis history. The present view can use verified holdings and price evidence without claiming that the journal knows every acquisition. Manual assets and loans use the same source/valuation discipline.

Market value can be an analytical valuation rather than a synthetic investment trade. If accounting recognition of revaluation is later adopted, it requires a named valuation convention and a separate auditable decision. Do not blend unrealized value movement, deposits, and investment return into one number. Performance reporting remains qualified or unavailable when cash-flow or cost-basis history is insufficient.

## 7. Reconciliation and the trustworthy present

### Reconciliation is a claim

A claim names the account or component scope, period/cutoff, units, basis of comparison, authoritative observations, evidence-visibility and decision cutoffs, relevant policy/projector versions, matched activity, exclusions, and unresolved residuals. States may include proposed, partially supported, confirmed for scope, disputed, and superseded. These are domain states with explicit reasons, not color alone.

For a complete statement interval, an account-specific comparison can check opening balance plus recognized movements against closing balance. Its semantics must account for liability signs, posting/effective dates, and whether pending items belong in the comparison. Tolerances never hide a difference; any permitted rounding rule and the amount it explains are shown.

A claim depends on inputs, not merely its creation date. A new observation does not invalidate every historic claim. A revision or correction that changes its scope or basis flags the affected claim for review. A claim remains reproducible against its original inputs even after that review.

### Present-position assembly

Each account/component exposes a position record containing at least the source assertion, journal amount if meaningful, effective/as-of date, receipt/freshness state, coverage, unresolved difference, valuation basis, and reason for the selected display basis. The aggregate is built from these records.

A provisional selection policy is to prefer the most relevant supported institutional assertion for the account and requested meaning, while showing the journal and discrepancy. “Newest received” alone is not enough: a newly downloaded old statement does not outrank a newer balance assertion. Conflicting equally applicable reports remain a conflict, not a silently arbitrary winner.

The household view must define account/component ownership and eliminate overlap: brokerage totals versus holdings, shared accounts represented in multiple feeds, and transfer-clearing versus confirmed destination assets. In-transit money is labeled as such. Unknown values stay unknown. A total can report known coverage and omitted components, but must not masquerade as complete net worth.

Useful query modes include:

- Current understanding of financial position at an effective date.
- The position and interpretations as understood at a prior evidence-visibility and decision cutoff, using the recorded selection policy.
- Institution-reported position with its source-specific meaning.
- Recognized accounting position and the explanation of any difference.

These modes need not become four competing default dashboards. They are the basis for an honest primary view and its explanations.

## 8. Organization and forecasting

### Organizational overlay

Model normalized parties, categories, tags, notes, allocations, and rule definitions as durable domain knowledge. Imported labels remain evidence. A rule-generated proposal records its evidence, rule version, and scope. An adopted mapping records who or what adopted it and how user overrides interact with later suggestions.

A split used only for reporting need not change accounting recognition. A split that changes accounting accounts or amounts does. The command contract makes that distinction explicit. Reclassification should not force a user to reimport evidence or lose reconciliation context.

AI is an optional proposal producer behind the same interfaces as deterministic rules and manual work. An AI result cannot write storage directly or invent provenance. The default implementation should work with manual choices and ordinary rules. Any external AI use requires an explicit data-sharing design; no financial records go to a remote model by default.

### Planning boundary

Keep portable commitments and recurrence semantics in a shared planning module above the accounting engine, with durable versions and export support. The Mac app can initially own its visual editing and exploratory charts. This avoids trapping future commitments in one client while keeping hypothetical activity out of the recognized journal.

A forecast run binds its starting position, evidence-visibility and decision cutoffs, selection-policy version, known actual/pending activity, commitment versions, calendar conventions, variable-spending assumptions, valuation/exchange assumptions where needed, horizon, and computation version. The initial approach can be deterministic scenarios and ranges. Statistical confidence intervals require later calibration; attractive ranges must not claim statistical certainty they do not have.

Schedule semantics must distinguish an intended due date from settlement date, define local timezone/calendar behavior and weekend handling, and permit user overrides. Those rules are versioned input, not guesses made differently by each client. Exceptions, skipped occurrences, partial payments, and transfers crossing the forecast boundary need explicit treatment.

Occurrence-to-actual associations track open, partly matched, satisfied, disputed, or overdue expectations. A match updates planning interpretation; accounting recognition still follows evidence and adopted financial decisions. Comparing forecast with actual uses the saved forecast, not a retrospectively improved version of its assumptions.

## 9. Interfaces and native client

### Contract shape

Expose domain-level commands and queries rather than database CRUD. Illustrative capabilities include ingest evidence, propose/adopt interpretation, recognize/correct activity, associate observations, establish opening position, evaluate/confirm reconciliation, query/explain position, manage commitments, run forecast, export, and verify/rebuild. These names are examples, not a frozen API.

Results include dataset identity, evidence-visibility cutoff, processed decision sequence, policy/projection versions, relevant source dates, uncertainty, coverage/freshness, explanations, and validation outcomes. Queries cannot return only a formatted amount when the omitted basis would change its meaning. Contracts are versioned; clients must handle unknown fields safely and reject unsupported command semantics.

The simplest initial public boundary can be a library plus a headless command-line adapter. A local service or IPC adapter may be needed for process coordination; that is a runtime choice, not a requirement to deploy a server. A future MCP adapter wraps these same capabilities with restricted access and explicit local mutation approval. It does not expose SQL, file paths, or raw credential access to an agent.

### Mac-native experience

Prefer native controls and platform conventions for lists, keyboard navigation, menus, selection, accessibility, undo presentation, document/window behavior, and file import. Choose SwiftUI/AppKit composition after a focused interaction prototype, not from a desire to make every client look identical.

The first workflow should let the user move from a present-position discrepancy to its evidence and local decisions, compare competing interpretations, preview a correction, and see the resulting position and reconciliation status. An undo of durable financial meaning is a compensating decision, not deletion of history. Cancellation before commit writes no financial decision; repeated submission is idempotent; stale previews require refresh.

The native client can be expressive without owning accounting authority. UI design and implementation are not part of this initial document deliverable.

## 10. Storage, portability, and recovery

### Proposed local storage shape

A transactional embedded store plus an evidence-attachment area is a strong candidate for a single-user local system. SQLite is worth evaluating for atomicity, local tooling, and portability; its selection and any encryption layer are not settled. Domain contracts and documented exports must not depend on one storage library's object graph.

Store private datasets outside the source checkout, build directories, and Git worktrees. A dataset manifest identifies its format version, identity, ordered-history head, evidence-receipt high-water mark, referenced evidence, policy/projection versions, and encryption metadata needed for recovery. Secrets are not exportable domain fields.

### Portable export

A full-fidelity export includes evidence and revisions, adopted decisions in order, stable identities and relationships, organizational meaning, reconciliation claims, commitments and selected forecast history, temporal/monetary conventions, and a versioned manifest with integrity checks. Human-readable transaction CSV is useful but not a sufficient export of the record.

Document the format and provide a headless verifier/reconstructor. Recovery may require appropriate decryption keys and a compatible reader, but cannot require the original Mac UI or a vendor-hosted service. Institutions becoming unavailable must not prevent reading accumulated evidence.

### Backups, migration, and snapshots

- Take a coherent backup using the storage engine's snapshot/backup mechanism, not an arbitrary copy of an active database without its transaction state. Include referenced evidence and a manifest. Keep credentials separate.
- Encrypt private backups and define how the owner retains recovery keys. A restored dataset is verified and replayed before it is treated as trusted. Test missing keys, corrupted blobs, interrupted backup, and restore to a new path.
- Snapshots identify their decision cutoff, schema/projector versions, and integrity basis; snapshots of views that depend on external assertions also pin evidence visibility and selection-policy versions. They accelerate replay but are discardable. Regular verification compares full replay with snapshot-assisted reconstruction on synthetic and subsequently authorized private datasets.
- Schema changes preserve original event meaning. Versioned readers or deterministic migrations handle old formats; unknown financial event versions stop trusted replay rather than being skipped.
- Migrations operate with a recoverable prior copy, verify counts/references and financial equivalence, and atomically switch only after validation. A migration must never rerun current inference as a replacement for recorded decisions.
- A local integrity digest can detect accidental corruption; it does not prove an uncompromised machine or an institution's truthfulness. Avoid claiming enterprise tamper-proof audit guarantees.

## 11. Security and public development

### Threats to address before real data

The principal risks are accidental publication, credential leakage, malware or another local process reading unlocked data, overpowered integrations/agents, unreliable backups, and financial misrepresentation caused by corrupt or incomplete inputs. Local-first reduces some exposure but does not eliminate these risks.

Institution connectors must use the narrowest read scopes available. Do not implement payment, funds-transfer, trade, or institution-state mutation methods. If a provider cannot offer suitable read-only access, record that limitation and use file/manual imports rather than silently requesting broader authority. Authentication setup, secrets, and consent are separate from financial domain operations.

Proposed controls, to be validated rather than assumed present:

- OS-backed secret storage for connector tokens and encryption keys; no credentials in the decision history, logs, Git, fixtures, or command-line arguments.
- Encrypted private datasets and backups with a tested recovery story. FileVault alone and application-level encryption have different unlocked-device behavior; choose the combination in the security design rather than claiming equivalence.
- Least-privilege adapters and local client capabilities. CLI diagnostics should redact by default; an agent's read scope should also be limited, because read-only access can still disclose sensitive information.
- No outbound telemetry or remote AI processing of financial content by default. Any opt-in must name the data and recipient, with a way to revoke it.
- Build/test/demo fixtures generated synthetically, with no account identifiers or disguised production exports. Sanitization requires review; changing names alone is not adequate.
- Public repository checks for secrets and prohibited data paths; diagnostic export that the owner can inspect before sharing. Ignore rules are only one safeguard.
- Resource limits and safe parsing for imported files. Treat source text and attachments as untrusted data, never executable instructions.

Append-only financial history does not require retaining secrets or accidental private imports forever. The separate data-lifecycle design must define dataset deletion, redacted diagnostic exports, and exceptional privacy-driven erasure or re-encryption with explicit user control and disclosure of the resulting evidentiary limits. Routine corrections remain append-only. Backup retention and erasure behavior cannot be promised until the storage/key design is tested.

This repository setup publishes nothing and configures no live financial connection.

## 12. Implementation approaches to evaluate

The shared-engine boundary is established. Its language and process placement are not.

| Approach | Why it fits | Main cost or uncertainty |
|---|---|---|
| Swift engine modules with a SwiftUI/AppKit client and CLI | Direct native integration; one language for the initial engine and client; a headless executable can prove client independence | Cross-platform packaging, exact-number libraries, migration tooling, and future agent/runtime bindings need validation |
| Rust engine with a native Swift client through a small binding | Strong headless/library boundary and broad non-UI reuse; exact domain types and deterministic code are practical | FFI lifecycle, error mapping, build packaging, and duplicate boundary types add complexity for a personal app |
| Local service with a versioned protocol and native client | Multiple clients share a single writer and language-neutral operations | Service startup, authentication, version negotiation, lifecycle, and recovery add operational surface |

**Provisional recommendation:** prefer the smallest in-process modular engine with a headless CLI, and add IPC only when concurrent client operation needs it. A Swift-first implementation is a plausible starting candidate because the primary product is Mac-native; Rust is credible if a short feasibility comparison demonstrates a material advantage for core correctness or reuse. Neither has been selected. The choice should follow a small, approved experiment exercising exact amounts, storage transactions, replay, CLI use, and native integration, not a language preference presented as a product requirement.

One logical decision sequence is recommended for the first dataset. Multiple independently authoritative streams or distributed event infrastructure add coordination without a demonstrated household-scale need. This is a reversible storage/runtime recommendation only if the portable history preserves an unambiguous ordering.

## 13. Validation against the constitution

The following are required test obligations for a future implementation. They are not test results from this documentation-only repository.

| Invariant | Enforcement or proof | Visible failure |
|---|---|---|
| 1. Preserve evidence | Immutable capture/revision references; duplicate and revision fixtures | Missing or unreadable evidence flagged |
| 2. Observation is not journal | Separate ingestion and recognition commands | Unrecognized evidence remains in review |
| 3. Balanced accounting | Exact arithmetic and atomic validation; generated unbalanced cases rejected | Command fails without partial commit |
| 4. Auditable decisions | Ordered identities, actors, correction links; S5 | History or reference gaps block trust |
| 5. Reproducible journal | Recorded consequences; byte/semantic canonical comparison across replay | Mismatch blocks trusted projection |
| 6. Meaningful time | Distinct dates/precision, evidence visibility, and decision cutoff tests | Unknown dates stay unknown; late evidence cannot alter prior knowledge |
| 7. Explicit uncertainty | Separate states through engine and every client | Unsupported state cannot look confirmed |
| 8. Evidence-based reconciliation | Scoped dependencies, residuals, invalidation; S1-S5 | Changed basis requires review |
| 9. Honest views | Basis/coverage/freshness query contracts; overlap fixtures | Qualified or unavailable totals |
| 10. Independent recovery | Full export/import, headless replay, backup restore | Failure reported before dataset trusted |
| 11. Visible failure | Interrupted imports, stale checkpoints, damaged projections | Persistent degraded status, no false success |
| 12. Incomplete history | Anchors, coverage gaps, unknown basis; S4 | Historical reports disclose limits |
| 13. External authority | Semantically scoped assertions alongside journal | Disagreement visible, no silent plug |
| 14. Fragmented activity | Many-to-many associations, partial legs; S2-S3 | Unmatched legs and residuals retained |
| 15. Durable user meaning | Versioned overlay, overrides, export/replay; S6 | Conflicting proposals do not erase adoption |
| 16. Forecasts are not facts | Separate planning capabilities; S7 | Due dates cannot write journal entries |
| 17. Client independence | Same commands through headless and Mac adapters; S8 | Direct storage writes unavailable to adapters |
| 18. Public/private separation | Synthetic CI, secret/data checks, recovery and diagnostic tests | Publication/diagnostic output blocked on detected private data |

Also test late-arriving backdated evidence against a saved read basis, import order permutations, retry after an uncertain response, duplicate commands, concurrent stale writers, transfer corrections after settlement, source deletions, foreign-currency refusal, zero/negative amounts, overflow, date precision and timezone changes, migration failure, lost evidence files, and pending/actual/forecast overlap. Performance targets should be measured on a realistic synthetic household history; no latency or scale guarantee is established by this draft.

## 14. First vertical slice and subsequent coverage

### Slice 1: a record that can explain and recover itself

Use a synthetic checking account and credit card, a starting position, a pending-to-posted charge, a card payment with delayed bank/card evidence, a changed merchant label, a reconciliation difference, and a recurring bill forecast. Run the sequence through a headless client and a minimal native inspection workflow.

The slice is complete only when it demonstrates:

1. Duplicate/revised evidence is preserved and recognized activity is not duplicated.
2. An adopted posted charge and local correction produce balanced, deterministic journal consequences.
3. Transfer/payment association does not double-count spending; a missing leg remains visible.
4. Present position shows source assertion, ledger position, freshness, and the difference.
5. A category or merchant correction survives reimport and replay.
6. A simple forecast includes a commitment, pairs an internal movement, and avoids counting matched actual activity twice without posting hypothetical entries.
7. The same domain command and explanation work without the GUI.
8. A full export/restore/replay reproduces the record; injected corruption fails visibly.

This is deliberately narrower than the MVP. It does not postpone deciding whether investments, incomplete history, or fragmented payroll fit the architecture. Their synthetic acceptance scenarios should shape the domain contracts before those contracts harden.

### Remaining MVP tracks

After the slice, expand scenario coverage to split transfers and payroll, incomplete historical anchors, holdings and manual valuations, broader organizational rules, useful forecast horizons and comparisons, native workflows, selected read-only adapters, and hardened private recovery. Maintain the full-picture release checklist from section 2. Do not call the application ready merely because the first cash/card slice works.

No dates or effort estimates are asserted yet. Connector coverage and storage/interop experiments could materially change sequencing.

## 15. Feasibility, open decisions, and next review

The architecture is plausible for a single-household local application: its hard problem is honest semantics under imperfect evidence, not raw transaction volume. Event sourcing helps preserve decisions, but adds versioning, correction, and recovery work that must earn its place through the tests above. No performance, provider coverage, or encryption integration has been validated yet.

### Essential product input before live integration

**Which actual account types/institutions and existing export formats must the first usable release cover?** This is an inventory question, not a request for credentials or balances. It determines read-only provider feasibility, costs, historical coverage, currencies, and which manual/file paths must exist. It does not block this preliminary design or the synthetic slice.

### Engineering decisions to resolve after design review

| Decision | Evidence needed | Proposed stance until resolved |
|---|---|---|
| Language, database, numeric and binding libraries | Small replay/atomicity/CLI/native feasibility experiment | No stack locked; no dependencies installed |
| Recognition and opening-position policy | S1-S5 examples including backfill, unknown counter-entries, and corrected anchors | Manual adoption first; never auto-plug discrepancies |
| Single-process versus local writer service | Concurrent Mac/CLI commands, crash recovery, packaging behavior | One logical writer, no hosted service |
| Currency and investment accounting extent | Account inventory plus representative synthetic FX/holding cases | Preserve unsupported evidence; disclose limits; no fabricated conversion or cost basis |
| Forecast calendar and starting-position policies | Realistic payday, card-payment, and transfer scenarios | Explicit, versioned assumptions in shared planning layer |
| Connector choice and update cadence | Verified institution coverage, read scopes, terms, cost, rate limits, deletion/revision semantics | File/manual inputs remain viable; no live connection yet |
| Dataset encryption, key recovery, and backup destination | Threat review and demonstrated restore after device/key loss scenarios | Private state outside repo; no claim that backup is already configured |
| Data retention and exceptional erasure | Recovery, privacy, and evidentiary tradeoffs | No routine history rewriting; deliberate user-controlled policy needed |
| Native interaction structure | Focused prototype of inspect, correct, reconcile, and forecast flows | Native platform conventions; no web-wrapper assumption |

The next review should challenge the scenarios and ownership boundaries first. Once accepted, write a bounded implementation plan and choose the minimum feasibility experiments needed to settle the stack and storage decisions. This document does not authorize application implementation, remote repository creation, publication, or live institution access.
