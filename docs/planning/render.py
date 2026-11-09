#!/usr/bin/env python3
"""Deterministic local-only Markdown renderer. manifest.json is publication input."""
import json,pathlib
ROOT=pathlib.Path(__file__).resolve().parent
M=json.loads((ROOT/'manifest.json').read_text())
for t in M['tickets']:
    text=f"# {t['issue_title']}\n\n**Status:** Proposed, not published. **Milestone:** {t['milestone']}. **Epic:** {t['epic']}.\n\n**Accountable role:** {t['accountable_role']} (assign a person at implementation kickoff).\n\n**Labels:** "+', '.join(t['labels'])+'\n\n'
    text+='**Hard dependencies:** '+(', '.join(t['depends_on']) or 'None')+'. These are finish-to-start prerequisites, not merely related work.\n\n'
    text+='**Spec:** [Technical design](../../technical-design.md), sections '+', '.join(t['design_sections'])+'; [constitution](../../constitution.md).\n\n'
    text+='**Shared contracts:** '+', '.join(f'[{c}](../contracts.md#{c.lower()})' for c in t['contracts'])+'. Read the exact names/types, owner ticket and constraints there; they are part of this issue.\n\n'
    text+='**Requirements:** '+', '.join(t['requirements'])+' (definitions and reverse coverage in [manifest](../manifest.json)).\n\n'
    text+='## Why\n\n'+t['why']+'\n\n## Scope\n\n'+t['scope']+'\n\n## Non-goals and authority\n\n'+t['non_goals']+'\n\n'
    text+='## Files\n\n'+'\n'.join('- Create or extend during authorized implementation: `'+f+'`.' for f in t['files'])+'\n\n'
    text+='## Interfaces\n\n- Consumes: '+', '.join(t['consumes'])+' from the linked shared contracts and prerequisite tickets.\n- Produces: '+t['produces']+'\n\n'
    if t['gate']:
        g=t['gate'];text+='## Input or external gate\n\n- Input: '+g['input']+'\n- Accountable decision: '+g['owner']+'\n- Until resolved: '+g['disabled']+'\n- Unrelated synthetic work continues. This issue does not grant external access, create credentials, accept terms, or make spending commitments.\n\n'
    text+='## Acceptance tests and counterexamples\n\n'
    for x in t['tests']:text+=f"- [ ] `{x['name']}`: {x['assertion']}\n"
    text+='\n## Test-first implementation checklist\n\n'+'\n'.join('- [ ] '+x for x in t['step_checklist'])+'\n\n'
    text+='## Verification\n\nRun after implementation:\n\n```sh\n'+t['verification']['command']+'\n```\n\nExpected: '+t['verification']['expected']+'\n\n'
    text+='## Measurable done\n\n'+t['done']+'\n\nAttach test output/version/platform/fixture hashes and review disposition, with no private financial data. Do not claim these prospective tests have already run.\n'
    (ROOT/t['body_path']).write_text(text)
for e in M['epics']:
    children=[t for t in M['tickets'] if t['id'] in e['child_ids']]
    text=f"# [{e['id']}] {e['title']}\n\n**Accountable role:** {e['accountable_role']}. **Label:** type:epic. **Status:** Proposed, not published.\n\nImplement the linked child contracts under [the implementation plan](../../implementation-plan.md) and [technical design](../../technical-design.md). This parent is an organizational issue, not a hard dependency of its children. Milestones belong to child tickets; the epic spans "+', '.join(sorted(set(t['milestone'] for t in children)))+'.\n\n## Children and acceptance\n\n'
    for t in children:text+=f"- [ ] [{t['id']}: {t['title']}]({t['id']}.md), {t['milestone']}.\n"
    text+='\nClose only when every listed child meets its measurable done and required tests, independent findings are resolved, and any remaining external gate is explicitly blocked rather than claimed passed. Epic closure cannot waive a constitutional invariant or convert a synthetic preview into household MVP.\n\n## Publication\n\nCreate child links using native parent/sub-issue relations where supported. Add blocked-by edges only from each child’s depends_on list after all issue IDs are mapped. Do not add dependencies on this parent. No assignee, due date, remote or publication authorization is inferred.\n'
    (ROOT/e['body_path']).write_text(text)
print(f"Rendered {len(M['tickets'])+len(M['epics'])} complete proposed issue bodies")
plan=ROOT.parent/'implementation-plan.md'
s=plan.read_text().split('<!-- Generated milestone and ticket index follows; renderer maintains this block. -->')[0]
s+='<!-- Generated milestone and ticket index follows; renderer maintains this block. -->\n\n'
for x in M['milestones']:
    s+=f"### {x['id']}: {x['title']}\n\n{x['description']}\n\n**Objective exit:** {x['exit_gate']}\n\n"
    s+='| Ticket | Epic | Hard predecessors | Outcome |\n|---|---|---|---|\n'
    for t in M['tickets']:
        if t['milestone']==x['id']:s+=f"| [{t['id']}](planning/{t['body_path']}) | {t['epic']} | {', '.join(t['depends_on']) or 'None'} | {t['title']} |\n"
    s+='\n'
s+='## Epic ownership\n\n'
for e in M['epics']:s+=f"- [{e['id']}: {e['title']}](planning/{e['body_path']}), accountable role {e['accountable_role']}; {len(e['child_ids'])} leaf tickets.\n"
s+='''
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
'''
plan.write_text(s)
