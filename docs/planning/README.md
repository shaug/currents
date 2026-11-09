# GitHub implementation issue package

The complete plan is published in [GitHub issues](https://github.com/shaug/currents/issues). See the [verified publication receipt](publication.md) and [actual issue mapping](publication-map.json). Start with [the implementation plan](../implementation-plan.md), [contracts](contracts.md) and [dependencies](dependencies.md).

The manifest, generated plan index and source bodies below are the frozen reviewed pre-publication snapshot. Their pending/proposed status and unset repository/authorization fields are historical source metadata. They are not the current publication state; implementation progress belongs in the live issues.

## Contents and regeneration

- `author_payloads.py`: editable authored ticket/milestone/label/coverage definitions; local-only, no network or GitHub writes.
- `manifest.json`: machine-readable proposed issues, `schema_version=1.0.0`, stable CUR-001..CUR-058 ticket IDs, E01..E09 epic IDs, M0..M6 milestone IDs, labels, hard dependencies and requirement coverage.
- `render.py`: deterministic Markdown bodies and plan index from manifest.
- `issues/`: 67 complete proposed bodies. Relative links work in checkout; publication rewrites them to verified repository/blob links at the approved commit.
- `validate.py`: package completeness, graph/coverage/link/privacy/contract hash validation. Uses Python standard library, no install needed.

Run from repository root:

```sh
python3 docs/planning/author_payloads.py
python3 docs/planning/render.py
python3 docs/planning/validate.py
```

Generation does not run application tests. Verification commands inside issues refer to future code/test suites owned by those issues; they are not assertions of current success.

## Publication procedure, only when separately authorized

1. Verify the destination owner/repository and that its visibility and source/document content are approved. Read current labels/milestones/issues first. Search stable IDs to avoid duplicates. Never infer a remote from a local directory or private account identity.
2. Create/update missing approved labels and seven milestones without deadlines. Create nine epic issues and 58 leaf issues from the complete bodies; no guessed assignees. Record each successful returned issue node ID,number,URL and payload hash in `publication-map.json`. This mapping is output of actual publication, not prefilled input. Do not push the map until appropriate approval.
3. Replace local design/contract links with verified exact repository blob links and stable IDs with returned issue links. Preserve complete issue bodies. Add native parent/sub-issue membership using child `epic`; epics do not block their children.
4. Add blocked-by dependencies from every `depends_on` edge only after all IDs exist. These are hard finish-to-start edges. Never turn nonblocking `related_to`, coverage references or epic grouping into blockers. Native GitHub relation support is preferred; if unavailable, disclose it and include exact dependency links in each body rather than falsely claim native relationships were created.
5. Verify each remote title/body hash (after deterministic link substitution), label,milestone,parent and dependency against manifest. Record publication receipt/report. Partial or uncertain tool results are checked remotely before retry; never blindly recreate issues.
6. Do not trigger CI, change branch rules, create credentials or publish application artifacts as a side effect of issue creation. Repository rules/settings, signing, provider permissions and releases have their own scoped tasks/approvals.

## Editing and review

Edit authored definitions and shared documents, regenerate and validate. Child issues contain exact scope, files, contracts, tests, gates and done criteria; do not shorten them into vague headlines during publication. Epics are organizational parents across milestones, not additional implementation tasks. All public examples are synthetic; private financial input and conversation provenance must stay outside this package.
