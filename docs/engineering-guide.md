# Engineering guide

Use the [technical design](technical-design.md), [constitution](constitution.md), [shared contracts](planning/contracts.md) and [dependency decisions](planning/dependencies.md) together. Exact interfaces and their owner tickets live in the shared contracts. [GitHub issues](https://github.com/shaug/currents/issues), parent relationships, hard dependencies and [milestones](https://github.com/shaug/currents/milestones) own current implementation work and completion gates. The [reviewed source package](planning/README.md) preserves requirement definitions, coverage and acceptance content; it is not a second mutable backlog.

## Coordination and contract ownership

Pick work whose hard predecessors are complete. Each ticket has one accountable role; assign a person at kickoff rather than guessing ownership. Coordinate changes to shared files with the contract owner. Consumer work may use exact contract fixtures before implementation, but no task may merge against a guessed substitute. A schema or signature change must update the owner contract, test vectors, all consumers and dependency graph in the same reviewed change.

Only hard dependencies create finish-to-start blockers. Related work is nonblocking context, and an epic groups children rather than blocking them. Milestones are completion gates, not a forced waterfall. Parser/source work and native platform qualification can run alongside storage; financial semantics, planning and recovery can proceed independently where dependencies permit. UI fixture work is not integrated acceptance until its hard dependencies pass.

Build tests first, record red, implement minimally, run green and affected regressions, obtain review, then make a focused check-in during authorized implementation. Do not copy a spike wholesale or treat planned commands as passed tests. Independent adversarial review decides closure; a checklist cannot certify itself.

Owner inventory, provider and FinanceKit research can proceed during synthetic engineering. When an external prerequisite is unavailable, record `Blocked` with the exact required input and keep the affected production path disabled. Continue unrelated synthetic work. Never infer private values, silently relax scope or call a synthetic preview a usable household MVP.

## Whole-project verification and qualification

Fast pull-request checks cover format, compiler warnings, selected strict Clippy, rustdoc/doctests, unit/integration/property contracts, dependency direction, schema/header/migration drift, privacy and cargo-deny. Rust current/MSRV and Mac arm64/Linux x86_64 storage lanes use locked production features, not indiscriminate all-features. Swift 6 strict-concurrency and native ABI/XPC checks cover debug and release. A missing native runner is a blocked gate, not a skipped green check.

Nightly/weekly qualification adds deeper properties, differential Ledger/Beancount fixtures, fuzzing, memory-boundary tools, shutdown and disk/I/O/crash matrices with triage owners and scoped expiring exceptions. Miri cannot certify SQLite/XPC. Process-kill tests cannot prove physical power-loss durability. Measure realistic performance separately without weakening accounting or encryption. Core correctness gates, not an arbitrary aggregate coverage percentage, decide acceptance.

Release qualification includes lock/build/native-feature/source inventories, native-inclusive SBOM/licenses, signed provenance, offline/cache-free builds and measured unsigned reproducibility. Actual nested signing, notarization, entitlements and minimum/current-OS install/upgrade/restore require their own authorized setup and evidence. Production secrets never enter fork PR jobs; only protected release jobs receive necessary write/OIDC permissions. Independent record interpretation and recovery remain testable obligations. No automatic update service or additional publishing/deployment channel is implied.

Every gate command must create a `QualificationReportV1` naming exact test functions, target/tool inputs, output digests, pass/fail/blocked state and residual limitations. Native gate runners invoke the ticket's named Swift assertions through the qualified Xcode scheme/device matrix and fail if an assertion or device/signing prerequisite is missing. Use each issue's exact commands and acceptance criteria. Documentation validation, application tests, production qualification and publication are separate claims; report only what actually ran.
