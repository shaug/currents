# Reviewed implementation source package

[GitHub issues](https://github.com/shaug/currents/issues), parent relationships, native hard dependencies and [milestones](https://github.com/shaug/currents/milestones) own current work state. Start implementation with the [engineering guide](../engineering-guide.md), [shared contracts](contracts.md), [dependency decisions](dependencies.md) and the relevant live issue.

This package retains the reviewed source baseline, including all requirement definitions, reverse coverage, scopes, commands and acceptance assertions. The manifest's pending/proposed status, unset repository/authorization fields and mapping filename are historical pre-publication metadata, not current publication instructions or dependencies on a local mapping file. Source bodies retain that baseline status while their documentation links follow the current shared guidance. Do not maintain progress, assignments or a parallel backlog here.

## Contents and regeneration

- `author_payloads.py`: reviewed authored ticket, milestone, label and coverage definitions; local-only, no network or GitHub writes.
- `manifest.json`: generated reviewed baseline with stable ticket/epic/milestone IDs, requirement definitions and coverage, and the reviewed dependency graph.
- `render.py`: deterministic rendering of the 67 source issue bodies only.
- `issues/`: complete reviewed source bodies. Local links work in checkout; authorized GitHub updates use verified repository links and actual issue IDs.
- `contracts.md` and `dependencies.md`: durable interface ownership, exact contracts, library decisions and qualification procedures referenced by the issues.
- `validate.py`: coverage, graph, invariant, contract/design hash, current-document link/privacy and rendered-body checks. Validation reports body drift without rewriting it.
- `test_planning.py`: standard-library regression tests for regeneration, acceptance preservation, determinism and validation failures.

Run from repository root, with Python's standard library and no install:

```sh
python3 docs/planning/author_payloads.py
python3 docs/planning/render.py
python3 docs/planning/validate.py
python3 -m unittest discover -s docs/planning -p 'test_*.py' -v
```

Generation does not recreate the retired plan, publication receipt or mapping. Their dated snapshots remain in Git history; the [review record](../review-record.md) retains review facts and hashes. Check the diff after regeneration. Intentional changes to reviewed definitions need review and reconciliation with the affected live issues; do not overwrite current GitHub work state from the historical snapshot.

Generation and validation do not run application tests or update GitHub. Verification commands inside issues belong to future code/test suites, not assertions of current success. Any separately authorized issue update must preserve scope and acceptance content, verify the current remote body and relationships, and check uncertain outcomes before retrying. Parent membership is not a blocking edge. Public examples remain synthetic.
