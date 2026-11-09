#!/usr/bin/env python3
"""Deterministic local-only Markdown renderer. manifest.json is the reviewed source baseline."""
import json,pathlib
ROOT=pathlib.Path(__file__).resolve().parent

def render_bodies(M):
    """Return reviewed source bodies without reading or writing other documents."""
    bodies = {}
    for t in M['tickets']:
        text=f"# {t['issue_title']}\n\n**Status:** Proposed, not published. **Milestone:** {t['milestone']}. **Epic:** {t['epic']}.\n\n**Accountable role:** {t['accountable_role']} (assign a person at implementation kickoff).\n\n**Labels:** "+', '.join(t['labels'])+'\n\n'
        text+='**Hard dependencies:** '+(', '.join(t['depends_on']) or 'None')+'. These are finish-to-start prerequisites, not merely related work.\n\n'
        text+='**Spec:** [Technical design](../../technical-design.md), sections '+', '.join(t['design_sections'])+'; [constitution](../../constitution.md).\n\n'
        text+='**Shared contracts:** '+', '.join(f'[{c}](../contracts.md#{c.lower()})' for c in t['contracts'])+'. Read the exact names/types, owner ticket and constraints there; they are part of this issue.\n\n'
        text+='**Engineering guidance:** [Coordination, contract changes and qualification](../../engineering-guide.md).\n\n'
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
        bodies[t['body_path']] = text
    for e in M['epics']:
        children=[t for t in M['tickets'] if t['id'] in e['child_ids']]
        text=f"# [{e['id']}] {e['title']}\n\n**Accountable role:** {e['accountable_role']}. **Label:** type:epic. **Status:** Proposed, not published.\n\nImplement the linked child contracts under [the engineering guide](../../engineering-guide.md) and [technical design](../../technical-design.md). This parent is an organizational issue, not a hard dependency of its children. Milestones belong to child tickets; the epic spans "+', '.join(sorted(set(t['milestone'] for t in children)))+'.\n\n## Children and acceptance\n\n'
        for t in children:text+=f"- [ ] [{t['id']}: {t['title']}]({t['id']}.md), {t['milestone']}.\n"
        text+='\nClose only when every listed child meets its measurable done and required tests, independent findings are resolved, and any remaining external gate is explicitly blocked rather than claimed passed. Epic closure cannot waive a constitutional invariant or convert a synthetic preview into household MVP.\n\n## Publication\n\nCreate child links using native parent/sub-issue relations where supported. Add blocked-by edges only from each child’s depends_on list after all issue IDs are mapped. Do not add dependencies on this parent. No assignee, due date, remote or publication authorization is inferred.\n'
        bodies[e['body_path']] = text
    return bodies


if __name__ == '__main__':
    manifest = json.loads((ROOT / 'manifest.json').read_text())
    bodies = render_bodies(manifest)
    for relative_path, text in bodies.items():
        (ROOT / relative_path).write_text(text)
    print(f'Rendered {len(bodies)} reviewed source issue bodies')
