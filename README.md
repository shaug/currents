# Currents

A personal, local-first financial system: a trustworthy present, a durable record, and explainable forecasts.

The shared financial engine is independent of its clients. The primary client will be a native macOS application. Institutional access is read-only; local ledger changes are validated and auditable.

## Project state

This repository contains the architectural constitution, the reviewed Rust-based technical design, and the reviewed implementation source baseline. It is documentation-only: application behavior, dependencies, encryption and release controls described here are requirements to implement and qualify, not working product claims.

- [Architectural constitution](docs/constitution.md): the governing principles, with the confirmed institutional access clarification.
- [Technical design](docs/technical-design.md): authoritative production architecture and financial, storage, security and client contracts.
- [Engineering guide](docs/engineering-guide.md): coordination, contract ownership and whole-project qualification guidance.
- [Historical preliminary design](docs/preliminary-technical-design.md): retained superseded proposal.
- [Review record](docs/review-record.md): corrected adversarial findings, final artifact hashes, checks and remaining qualification gates.
- [Live GitHub implementation tracker](https://github.com/shaug/currents/issues): 58 tickets and 9 epic parents, with milestones, labels and native dependencies. GitHub owns current work state; the [reviewed source package](docs/planning/README.md) retains requirement definitions, coverage and complete acceptance content.
- [Constitution provenance](docs/constitution-provenance.md): source identity and preservation check.

The selected direction is a client-independent Rust engine with a pinned doubleentry accounting adapter and private SQLite transaction seam, plus a native SwiftUI/AppKit Mac client. SQLCipher and the unified persistent Rust/Swift path remain explicit qualification gates. The initial usable product covers the household financial picture, organization and one-, three- and twelve-month account forecasts; a synthetic proving slice does not reduce that scope. No live provider access or shipping release is implied by the documentation.

## Public development, private state

The intended code and design can be public. Actual financial data, credentials, exports, backups, and user-derived interpretations must stay outside the repository. Public tests and demonstrations must use synthetic fixtures. Ignore rules are a backstop, not a data-security boundary.

## Documentation checks

Before changing the constitution, identify the deliberate amendment. Preserve its source history. Review every design against the invariants and scenario acceptance checks. At this documentation-only stage there is no application build or test suite.

Run the [source package checks](docs/planning/README.md#contents-and-regeneration) for deterministic generation, documentation validation and tooling regression tests. These checks do not establish application readiness.
