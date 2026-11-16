# Native Runtime Baseline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the invented exact-15.0 runtime promise with an explicit planned patched baseline without claiming qualification.

**Architecture:** Lock a requested runtime baseline separately from the compile floor and observed/qualified runners. Match actual macOS receipts exactly; retain every existing matrix requirement.

**Tech Stack:** Rust xtask, JSON inventory, Python planning renderer, Markdown.

**Spec:** docs/superpowers/specs/2026-11-16-native-runtime-baseline.md

## Global Constraints
- Compile/API floor 15.0 stays unchanged; initial requested runtime baseline is 15.8.2 arm64.
- No old evidence relabeling, native installation, security-gate change, network retry, commit or publication.
- Full five-role matrix, clean exact candidate and production Blocked remain mandatory.

## Review Focus
- Validly rehashed 15.0 or other wrong-version receipts must fail.
- Null, missing or malformed baseline cannot qualify.
- Synthetic fixtures must never populate observed qualification fields.
- Compile target and runtime support prose cannot be conflated.
- Hosted supplemental output remains supplemental, not full qualification.

### Task 1: Explicit baseline and coherent contract
- [x] Add evaluator regression proving locked15.8.2 passes and other actual versions fail with valid hashes; prove invalid lock fails.
- [x] Run bounded offline native evaluator tests and retain red.
- [x] Add lock reader and exact baseline comparison; update synthetic fixture and planned inventory.
- [x] Update CUR004 manifest/source payload via renderer and native qualification documentation, retaining historical facts explicitly.
- [x] Run native evaluator tests, offline planning/collector tests, format and diff checks; retain green.
- [x] Obtain independent review before any commit or publication.

### Task 2: Bound ephemeral hosted simulator identities
- [x] Add synthetic matrix tests for successful runtime-pinned selection, mismatched runtime/platform/UUID, missing/duplicate/unavailable inventory, conflicting modes, stale candidate/tree, and Debug/Release mismatch.
- [x] Record red against unchanged destination implementation.
- [x] Add pure simulator inventory selector and mode validation; capture inventories before collection and bind exact destination/runtime during evaluation.
- [x] Keep fixed destination path unchanged; record requested simulator runtime 18.6 separately from qualification.
- [ ] Run offline tests and independent review. Design hosted collection separately before changing workflow/CLI.


### Task 3: Hosted partial collection without false pass
- [x] Add helper regression tests and record stub red, then green for exact partial-collection sentinel, candidate/role/path/freshness and failure rejection.
- [x] Use immutable PR head/dispatch SHA, read-only workflow token and no credentials; avoid duplicate supplemental native runs.
- [x] Preserve canonical artifacts and xcresult bundles, not DerivedData; keep thirty-minute job and ten-minute role limits.
- [x] Run all xtask unit tests, strict xtask Clippy, Python native tests, actionlint, formatting and diff checks offline.
- [ ] Obtain independent review before commit, push or actual runtime collection.
