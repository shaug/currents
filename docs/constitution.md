# Currents — Architectural Constitution
**Status:** Foundational draft · **Date:** 2026-11-08

## Preamble
Currents preserves the history of financial observations and interpretations, enforcing accounting consistency while understanding stabilizes over time.

Currents is a personal, local-first system for understanding a complete financial picture. It brings multiple independent streams of evidence into a coherent, inspectable accounting representation. **Its central purpose is a trustworthy representation of the present financial position.** That understanding is grounded in the past and supports nonauthoritative projections of the future. Its primary product is the durable financial record. A client-independent financial engine preserves and interprets that record; interfaces are replaceable ways to examine and work with it. The primary user experience is a genuinely Mac-native application, not a constraint on the engine.

Currents is **personal in scope, not private in development**. Its source code and technical design are developed publicly; the user's actual financial data and credentials are private.

This constitution defines **what must remain true**, not the schemas, algorithms, libraries, or interfaces used to make it true. Technical designs must demonstrate how they satisfy these principles.

## 1. The things we represent
**Observations.** A source's report about financial activity, balances, holdings, valuations, or identity. An observation establishes what was reported, by whom, and when—not necessarily what happened economically.

**Financial events and relationships.** Economic activity and the connections among observations, accounts, parties, and events. Relationships may be one-to-one, one-to-many, many-to-one, or many-to-many, across institutions and dates. They may be provisional, incomplete, or unresolved; neither relationship nor identity may be silently presumed.

**Interpretations and decisions.** The system's evolving understanding of observations, including user-confirmed judgments, automated inferences, classifications, and corrections. This includes a durable organizational overlay: normalized merchant and counterparty identities, categories, tags, labels, annotations, and other user-defined meanings. These interpretations must not overwrite source evidence. Their provenance and differing authority must remain distinguishable.

**Accounting consequences.** Recognized financial activity expressed through balanced double-entry accounting, including assets, liabilities, equity, income, and expenses. A familiar personal-finance transaction is a presentation of this underlying accounting reality, not its authoritative storage form.

**Reconciliation claims.** Scoped, dated, evidence-backed assertions about how accounting state compares with external records. Reconciliation can link fragmented activity across sources and may remain partially unresolved; it is revisable, not an eternal property of an account.

**Present-state representations.** Current account balances, liabilities, holdings, valuations, cash flow, and net worth, assembled from accounting state and appropriately scoped external assertions. Their freshness, completeness, and unresolved discrepancies are integral to the representation.

**Historical and prospective projections.** Registers, spending histories, reports, expected transactions, recurring patterns, and forecasts of future account balances and financial position. Historical views explain the present; prospective views explore possible futures. Different views may legitimately emphasize different dates and meanings.

## 2. Nonnegotiable invariants
1. **Evidence is preserved.** Original source observations, meaningful revisions, source identity, and temporal context remain recoverable. A source's latest value never silently replaces its history.
2. **Observations are not journal entries.** Importing data does not, by itself, establish a financial event or its accounting treatment.
3. **Accounting is double-entry.** Every committed journal entry balances under explicit currency and valuation rules. Incomplete understanding must be represented explicitly, not hidden by fabricated counterparty or classification data.
4. **Accounting decisions are auditable.** Material changes to financial interpretation or recognized accounting activity are attributable, ordered, and recoverable. Corrections preserve prior decisions rather than silently rewriting them.
5. **The journal is reproducible.** The authoritative domain history contains enough accounting decisions to deterministically reconstruct the journal. Replaying recorded decisions is distinct from reinterpreting historical evidence using new rules.
6. **Time retains meaning.** Economic/effective time, institutional posting time, observation time, and decision time remain distinguishable wherever applicable. Original temporal precision and uncertainty must not be silently discarded.
7. **Uncertainty is explicit.** Unknown, pending, inferred, disputed, unmatched, stale, and reconciled are not interchangeable states. Confidence is not proof.
8. **Reconciliation is evidence-based.** Its scope, cutoff, supporting observations, exceptions, and later invalidation must be inspectable.
9. **Views cannot manufacture truth.** Derived balances, market valuations, cash flows, and net worth must disclose their basis, coverage, and freshness; missing information is never silently treated as zero.
10. **Data is independently recoverable.** Authoritative evidence and domain history survive UI replacement, projection rebuilds, and application upgrades. Backup, restore, and portable export must be demonstrable.
11. **Failure is visible.** Ingestion gaps, stale connections, invalid projections, reconciliation differences, and integrity failures cannot be disguised as successful or complete financial state.
12. **Incomplete history is valid.** Missing historical transactions, partial coverage, and unknown opening positions must be representable without invented activity. Current verified balances may coexist with incomplete historical reconstruction; their evidentiary status must remain distinguishable.
13. **External account authority is respected.** An institution's reports are authoritative assertions about the accounts it maintains, within the reports' stated scope, timing, and semantics. Ledger consistency cannot override such assertions, nor can an external balance silently force an unexplained journal adjustment. Disagreements remain visible until explained or explicitly resolved.
14. **Fragmented activity can be associated.** One economic event may span multiple accounts, institutions, dates, and observations, and may be only partially visible. The system must support provisional, explainable associations without requiring complete or synchronized source records.
15. **User meaning is durable and independent.** Merchant normalization, categories, tags, labels, annotations, and other organizational interpretations are user-controlled financial knowledge, distinct from provider descriptions and accounting evidence. They must be preservable, explainable, revisable, and recoverable across imports and application changes. Automated or AI-assisted proposals must be distinguishable from user-confirmed decisions; their adoption must respect accounting integrity.
16. **Forecasts are not facts.** Expected transactions, recurring patterns, projected balances, cash flow, and prospective financial positions must remain separate from observations and recognized accounting activity. Their assumptions, time horizons, uncertainty, and subsequent comparison with actual events must be inspectable. Passage of time alone must never turn a forecast into a journal entry.
17. **Client-independent financial semantics.** The financial record, accounting invariants, domain decisions, and their validated operations must be accessible through defined interfaces independently of any particular client. A graphical application, CLI, automation, or agent-facing tool must not acquire a separate, incompatible definition of financial truth or bypass integrity controls. This does not mandate a network service, a universal API, or platform-neutral implementation.
18. **Public code, private financial state.** Public development must not expose real financial records, account identifiers, secrets, access tokens, or sensitive user-derived interpretations. Private data, backups, and credentials require explicit protection and must remain separate from publicly distributable source code, examples, tests, logs, and diagnostics.

## 3. Consistency and stability
**Structural consistency is immediate:** committed accounting state must satisfy its invariants.

**Interpretive stability emerges:** as observations arrive and relationships are clarified, the system's understanding may become more settled. Stability does not mean finality; later evidence may legitimately revise an earlier interpretation.

Different sources may accurately describe different facets of an economic event without agreeing on a single amount, timestamp, or identity. Currents preserves those distinctions while constructing an explainable accounting interpretation.

**Accounting consistency and financial fidelity are independent.** A balanced journal proves internal consistency, not agreement with external account reality. A source-reported balance and a journal-derived balance can disagree without either observation being discarded or an invented balancing transaction being introduced. Differences are explicitly scoped and tracked until resolved, and may remain unresolved.

**Completeness is not a prerequisite for integrity.** Historical gaps, partial source coverage, opening balances, and positions of uncertain provenance are first-class conditions. A coherent present-day picture need not imply a complete reconstruction of the past.

**The present is the primary concern.** Currents uses historical evidence and interpretation to establish the most reliable available account of current financial position. Forecasts extend that understanding into the future, but remain conditional and nonauthoritative. Neither historical gaps nor uncertain forecasts may be hidden to create an illusion of present-day precision.

## 4. Authority and reconstruction
External institutions are authoritative about the accounts they maintain, subject to the scope and temporal meaning of their reports; the evidence store is authoritative about exactly what those institutions reported. The domain event history is authoritative about what Currents recognized and decided. The journal is a deterministic accounting projection of that history. Present-state, historical, and forecasting views are further projections. Forecasts have no authority to change recognized accounting activity.

Snapshots and caches may accelerate reconstruction but never become independent financial authorities. They must be replaceable, verifiable, and compatible with reconstruction from the authoritative record.

Ordering of domain decisions must be unambiguous. This constitution does **not** prescribe whether the technical design uses one physical event stream or multiple streams with a defined total order.

## 5. Engine, clients, and portability
The durable financial record is the product; the financial engine is responsible for its integrity, interpretation, and access. Clients may present, organize, automate, and extend those capabilities without becoming the sole custodians of financial truth.

Currents must support multiple kinds of clients, including a native macOS application, command-line tools, and controlled automation or agent interfaces. Shared financial operations must enforce the same domain invariants regardless of the client invoking them. Access interfaces should make provenance, uncertainty, freshness, and validation outcomes available rather than reducing them to display-ready values.

The primary interface should feel native to macOS, including where platform-specific presentation or workflows provide real value. Client independence must not require lowest-common-denominator interfaces. Equally, platform-specific features must not trap the authoritative financial record or essential accounting logic inside the graphical application.

Not every application capability must reside in the core engine. Forecast visualizations, exploratory analyses, and other client-specific functionality may live above it; durable financial knowledge such as confirmed future commitments must remain portable and usable independently of the client that created it. The technical design determines these boundaries without weakening the constitutional guarantees.

Data portability includes the ability to recover the financial record and its meaning, not merely to export a flattened transaction list. The technical design must provide stable, documented ways to access, migrate, and reconstruct authoritative data without depending on a particular interface or proprietary runtime.

## 6. Scope and restraint
Currents targets a complete household financial picture: cash, credit, debt, investments, manually observed assets and liabilities, and their changes over time. **Accurate present-state representation is the core product goal.** The initial product must also support meaningful organization of activity and forward-looking account-level cash-flow forecasts over useful short-, medium-, and longer-term horizons. Forecasting quality may improve over time without weakening the separation between predictions and accounting facts.

Double-entry rigor serves **data integrity**, not corporate reporting or legal compliance. The system need not implement comprehensive accrual accounting, tax-lot accounting, or enterprise audit infrastructure merely to satisfy this constitution.

It should be useful without AI. Automated interpretation may assist with merchant normalization, classification, relationship discovery, recurring-activity detection, and forecasting, but cannot bypass accounting validation, erase provenance, or present guesses as verified facts. The technical design may choose the appropriate mix of rules, statistical methods, AI, and user judgment.

Local ownership and user control are design goals. External integrations may incur ongoing costs, but continued access to the accumulated financial record must not depend on a vendor's hosted application.

The application and its design are developed **in public on GitHub**, while personal financial data is managed, stored, and backed up privately and securely. Public development must be safe by design: demonstrations and automated tests should use synthetic or appropriately sanitized fixtures, and private data must not be required to build or validate the public codebase. Specific storage, encryption, synchronization, and backup mechanisms belong in the technical and security designs.

## 7. How designs are judged
Every subsequent technical design should show:
- How each invariant is enforced or tested, including failure behavior.
- How a native macOS client, CLI, and restricted agent interface can access the same validated financial capabilities without duplicating accounting rules.
- How client-specific features can evolve without locking durable financial knowledge or the authoritative record into a single application.
- How a different client or rebuilt application can recover and interpret the financial record through documented, portable interfaces.
- How a pending card authorization, final posted charge, later enrichment, and statement reconciliation coexist without overwriting one another.
- How an internal transfer is represented without double-counting spending, including split and delayed transfers across institutions.
- How a paycheck spanning net deposits, retirement contributions, HSA contributions, and withholding is associated even when observations arrive separately or are missing.
- How an externally reported balance and a balanced journal can disagree without fabricated adjustments.
- How verified current holdings and balances coexist with permanently incomplete historical records.
- How a historical correction changes today's understanding without changing what the system previously believed.
- How incomplete investment history and stale valuations appear without invented transactions or false precision.
- How changing payment processor descriptions can map to a stable merchant identity without destroying source text, and how category, tag, and split interpretations remain independently correctable.
- How automated or AI-assisted classifications are proposed, confirmed, corrected, and audited without confusing inference with authority.
- How expected paychecks, recurring bills, variable spending, and inter-account transfers contribute to account-level forecasts without double-counting or posting hypothetical transactions.
- How forecasts expose assumptions and uncertainty, and how actual activity can be compared with predictions without retroactively rewriting them.
- How a complete present-day position is represented despite historical gaps, stale accounts, or unresolved discrepancies.
- How public builds, tests, diagnostics, and examples operate without exposing private financial data or credentials, and how private backups can be restored securely.
- How a journal can be rebuilt from domain history, and how snapshot-assisted recovery is verified against full replay.

Where a design trades away an invariant, it must be rejected or this constitution must be deliberately amended. Implementation convenience is not an implicit amendment.

---
*The present is the product. The past is its evidence. The future is its projection. The record endures; the interface is a perspective. Stability is earned through evidence, not assumed by software.*


## 8. Clarification: read-only institutional access
**Confirmed 2026-11-09.**

Currents does not carry out financial transactions at connected institutions. Its institutional connections are read-only: it cannot initiate payments, transfer funds, submit trades, or otherwise change institution-held financial state.

Currents may change its own local ledger and associated financial record through validated, auditable operations that satisfy this constitution. Recording or correcting an account transfer locally describes financial activity; it does not execute a transfer at an institution.
