#!/usr/bin/env python3
"""Validate local documentation payloads only. No network or GitHub mutation."""
import json,pathlib,re,sys,hashlib
from render import render_bodies
from urllib.parse import unquote, urlsplit
ROOT=pathlib.Path(__file__).resolve().parent
REPO=ROOT.parent.parent
m=json.loads((ROOT/'manifest.json').read_text()); errors=[]
def check(x,msg):
    if not x:errors.append(msg)
ids={t['id']:t for t in m['tickets']}; epics={e['id']:e for e in m['epics']}; milestones={x['id'] for x in m['milestones']};labels={x['name'] for x in m['labels']}
check(len(ids)==len(m['tickets']),'Duplicate ticket ID')
check(len(epics)==len(m['epics']),'Duplicate epic ID')
required=['id','title','issue_title','milestone','epic','labels','accountable_role','depends_on','related_to','contracts','design_sections','requirements','why','scope','non_goals','files','consumes','produces','test_package','tests','verification','done','gate','body_path','step_checklist','suite_commands']
for t in ids.values():
    for k in required:check(k in t,f"{t['id']} missing {k}")
    check(t['milestone'] in milestones,f"{t['id']} bad milestone")
    check(t['epic'] in epics,f"{t['id']} bad epic")
    check(set(t['labels'])<=labels,f"{t['id']} bad labels")
    check(set(t['depends_on'])<=ids.keys(),f"{t['id']} unknown predecessor")
    check(t['id'] not in t['depends_on'],f"{t['id']} self dependency")
    check(t['id'] in epics[t['epic']]['child_ids'],f"{t['id']} missing parent membership")
    n=int(t['id'][4:]); expected=f"crates/{t['test_package']}/tests/cur_{n:03}.rs"
    check(expected in t['files'],f"{t['id']} missing exact test path {expected}")
    for p,suite in re.findall(r'cargo test --locked -p ([\w-]+) --test (cur_\d+)',t['verification']['command']):
        check(f'crates/{p}/tests/{suite}.rs' in t['files'],f"{t['id']} verification package/file mismatch")
    check(all('cargo xtask ticket '+t['id'] not in c for c in t['suite_commands']),f"{t['id']} recursive suite dispatch")
    check('cargo xtask ticket '+t['id'] not in t['verification']['command'],f"{t['id']} recursive verification alias")
    check(len(t['tests'])>=3,f"{t['id']} no counterexamples")
    check(len(t['step_checklist'])==7,f"{t['id']} inconsistent checklist")
    check((ROOT/t['body_path']).is_file(),f"{t['id']} missing body")
for e in epics.values():
    check(not e['depends_on'],f"{e['id']} epic must not create child blocking cycles")
    check(set(e['child_ids'])<=ids.keys(),f"{e['id']} unknown child")
    for i in e['child_ids']:check(ids[i]['epic']==e['id'],f'{i} duplicate/wrong parent')
seen=set();active=set();order=[]
def visit(i):
    if i in active:errors.append('Cycle at '+i);return
    if i in seen:return
    active.add(i)
    for d in ids[i]['depends_on']:
        if d in ids:visit(d)
    active.remove(i);seen.add(i);order.append(i)
for i in ids:visit(i)
check('--prerequisites-for CUR-054' in ids['CUR-054']['verification']['command'],'Final gate evaluates itself as prerequisite')
check(ids['CUR-054']['verification']['command'].rfind('cargo xtask gates --profile production') > ids['CUR-054']['verification']['command'].find('qualify household-release'),'Final all-gates audit must follow own persisted qualification')
check('CUR-056' in ids['CUR-048']['depends_on'],'Private activation lacks signed-owner qualification')
check('CUR-057' in ids['CUR-054']['depends_on'],'Final release lacks protected Apple coverage')
check('CUR-044' not in ids['CUR-046']['depends_on'],'Synthetic mobile implementation unnecessarily blocked on credentials')
coverage={r['id']:r for r in m['requirement_coverage']}
for r in coverage.values():
    check(bool(r['ticket_ids']),f"Uncovered {r['id']}")
    for i in r['ticket_ids']:
        check(i in ids and r['id'] in ids[i]['requirements'],f"Bad reverse coverage {r['id']} {i}")
for t in ids.values():
    for r in t['requirements']:check(r in coverage and t['id'] in coverage[r]['ticket_ids'],f"Bad forward coverage {t['id']} {r}")
for r in [f'S{i}' for i in range(1,13)]+[f'CINV-{i:02}' for i in range(1,19)]+[f'C-SECTION{i}' for i in [1,3,4,5,6,7,8]]+['C-PREAMBLE']:
    check(r in coverage,f'Missing required coverage {r}')
check(hashlib.sha256((ROOT.parent/'technical-design.md').read_bytes()).hexdigest()==m['design_sha256'],'Design changed; revalidate reviewed baseline')
contracts=(ROOT/'contracts.md').read_text()
for t in ids.values():
    for c in t['contracts']:check(f'id="{c.lower()}"' in contracts,f"Unknown contract {c}")
# Compare in memory: validation must never repair unreviewed body drift.
for relative_path, expected in render_bodies(m).items():
    body = ROOT / relative_path
    check(body.is_file(), f'Missing rendered body {relative_path}')
    if body.is_file():
        check(body.read_text() == expected, f'Rendered body differs: {relative_path}; regenerate and review')
retired_names = (
    'docs/implementation-plan.md', 'docs/planning/publication.md',
    'docs/planning/publication-map.json')
retired = {REPO / name for name in retired_names}


def retired_github_dependency(destination):
    """Reject current repo references, but allow explicit historical commit links."""
    url = urlsplit(destination)
    path = unquote(url.path)
    if url.hostname == 'github.com':
        match = re.match(r'/shaug/currents/(?:blob|raw)/(.+)$', path, re.I)
    elif url.hostname == 'raw.githubusercontent.com':
        match = re.match(r'/shaug/currents/(.+)$', path, re.I)
    else:
        return False
    if not match:
        return False
    reference_path = match.group(1)
    for name in retired_names:
        suffix = '/' + name
        if reference_path.endswith(suffix):
            reference = reference_path[:-len(suffix)]
            return re.fullmatch(r'[0-9a-fA-F]{40}', reference) is None
    return False

files = [REPO / 'README.md'] + sorted((REPO / 'docs').rglob('*.md'))
links=0
for f in files:
    text=f.read_text()
    for bad in ['/Users/','chatgpt.com/c/','Sentinel_','currents-design-plan-2026','/tmp/']:
        check(bad not in text,f'Private material marker {bad} in {f.relative_to(REPO)}')
    for dest in re.findall(r'\]\(([^)]+)\)',text):
        if re.match(r'https?://',dest):
            check(not retired_github_dependency(dest), f'Retired document dependency {f.relative_to(REPO)} -> {dest}')
            continue
        filepart,_,anchor=dest.partition('#')
        target=(f.parent/filepart).resolve() if filepart else f
        check(target not in retired, f'Retired document dependency {f.relative_to(REPO)} -> {dest}')
        check(target.exists(),f'Broken local link {f.relative_to(REPO)} -> {dest}')
        if anchor and target.exists():
            content=target.read_text()
            headings={re.sub(r'[^\w\- ]','',h.lower()).replace(' ','-') for h in re.findall(r'^#+ (.*)$',content,re.M)}
            check(f'id="{anchor}"' in content or anchor in headings,f'Broken anchor {dest} in {f.name}')
        links+=1
# No prefilled remote, assignee, dates, or accidental publication flag.
check(m['repository'] is None and not m['publication']['authorized'],'Unexpected publication authorization')
for t in ids.values():check(not any(k in t for k in ['assignee','due_date','repository','github_number']),'Invented remote identity')
if errors:
    print('\n'.join('ERROR: '+x for x in errors));sys.exit(1)
print(f"PASS: {len(ids)} tickets; {len(epics)} epics; {len(milestones)} milestones; {len(labels)} labels; {len(coverage)} fully mapped requirements; {sum(len(t['depends_on']) for t in ids.values())} acyclic hard dependency edges; {links} local links/anchors; rendered bodies match; package/test paths consistent; public-material scan clean; design hash matches.")
print('Topological order: '+', '.join(order))
print('Not performed: application tests, HTTP reachability checks, production qualification or GitHub publication.')
