# Agent instructions

These rules apply to every change in this repository. CONTRIBUTING.md explains
the gates in detail, and docs/engineering-guide.md explains how work moves from
ticket to merge.

## Data

Use synthetic data only. Never commit, paste or attach household data,
institutional credentials, raw private diagnostics or financial state.

## Checks

Run the gates that cover your change before you push:

- `cargo xtask check quality`
- `cargo xtask check core`
- `cargo xtask check msrv`
- `cargo xtask check docs`
- `cargo xtask check privacy`

A failing check means the change is not ready. Do not weaken lints, compiler
settings, `deny.toml` or CI to make a check pass. A function-scoped lint
exception needs a reason and a matching entry in `docs/lint-allowlist.json`.

Worktrees under `.claude/worktrees` sit inside the main checkout. Tests that run
a nested `cargo` (`cur_001_3`, `cur_003`, `cur_005`) fail there because cargo
finds the parent checkout's `Cargo.toml`. Before you treat those failures as
real, copy the worktree without `target/` and `.git/` to a directory outside the
repository, run `git init` and commit there, and run the gate from the copy.

## Test cost

CI runs each test in its own process and fails any test that runs past its time
limit. Unlisted tests get 30 seconds. Tests listed in `docs/slow-tests.json` get
their own limit and start first, in list order. CI prints each test's duration
as a `core test start_ms=... duration_ms=...` line in the core and MSRV logs.

- Reopen or restart the service only when reopening is the behavior under test.
  Each reopen replays the whole history, so a test that reopens after every
  commit gets slower with the square of its commit count. To change a service
  test's planner, swap it through `PlannerSlot`
  (`crates/currents-service/tests/support/lifecycle.rs`) instead of restarting.
- Do not start `cargo` from a test. Each nested build competes for the shared
  target directory and slows every test running beside it.
- When a test runs past its limit, make it faster. Add or raise an entry in
  `docs/slow-tests.json` only when the test needs the time, give a specific
  reason, and say so in the pull request. The maintainer reviews every change
  to that file.
- When you rename or delete a listed test, update its entry. The core gate
  fails when the list names a test that no longer exists.

## Pull requests

Fill in every section of `.github/pull_request_template.md`. For a new or
changed slow-test entry, give the durations from the pull request's own CI run.
