# FinanceKit Route Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an executable route-only FinanceKit prerequisite gate that fails closed without accessing financial accounts.

**Architecture:** A pure xtask prerequisite evaluator feeds the existing C16 blocked result. Public integration tests exercise the real command and existing synthetic statement parser. No new live data intake or eligibility override is exposed.

**Tech Stack:** Existing Rust workspace, serde_json, xtask; no new dependency.

**Spec:** `docs/superpowers/specs/2026-11-17-financekit-route-design.md`

## Global Constraints
- Unchanged iOS 18.0 API floor; no Apple/account access or capability activation.
- C15/C16 remain unchanged; production and real coverage stay Blocked.
- Unknown owner acceptance is unknown; file imports are not automation parity.
- Fixed public reason codes only; no private strings echoed.

## Review Focus
- Unknown/personal developer evidence cannot establish organization eligibility.
- Unrelated or absent entitlement/bundle evidence cannot establish a route.
- Complete synthetic prerequisites cannot activate live account capture.
- Successful file parsing cannot remove the automation blocker.
- Unsupported CLI arguments cannot smuggle approvals or leak private values.

### Task 1: Route-only blocked gate and evidence

**Files:**
- Create `xtask/src/financekit_gate.rs`, `crates/currents-acquisition/tests/cur_044.rs`.
- Modify `xtask/src/main.rs`, `xtask/src/checks.rs` for minimal registration.
- Create `docs/integrations/apple-card-qualification.md` and `native/CurrentsCapture/Config/Entitlements/README.md`.

**Interfaces:**
- Consumes existing `QualificationReportV1`, `FileParser::parse`, and fixed native capture policy.
- Produces `financekit_gate::qualify(args: &[String]) -> Result<serde_json::Value, String>`; operational no-argument result is Blocked with fixed prerequisite codes. No input permits promotion.

- [ ] Run clean-base `cargo xtask qualify financekit`; preserve unregistered-command result.
- [ ] Add four named public tests (`cur_044_1`, `cur_044_3`, `cur_044_4`, `cur_044_no_account_probe`) and evaluator unit counterexamples before implementation. Test real report status/reasons; successful synthetic parser remains separate from automation. Verify missing behavior red.
- [ ] Implement pure prerequisite assessment and blocked command, rejecting all extra arguments without including supplied strings. Keep live facts unresolved and automation false even for positive unit fixtures.
- [ ] Register `ticket CUR-044` as named synthetic suite, distinguish suite pass from operational route state. Check exact required test inventory, no zero-match success.
- [ ] Record current official source findings in docs; leave entitlement configuration inactive.
- [ ] Run focused tests green, xtask regressions, fmt/Clippy and full quality; coordinate heavy builds. Preserve any unavailable gate honestly.
- [ ] Commit only scoped files after verification, freeze clean candidate and submit the complete candidate to canonical review. Publish/merge only through the existing lifecycle after all required gates.
