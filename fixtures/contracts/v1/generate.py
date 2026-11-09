from pathlib import Path
import json
r=Path(__file__).resolve().parents[3]; f=r/'fixtures/contracts/v1'
def save(n,v): (f/n).write_text(json.dumps(v,ensure_ascii=False,indent=2)+'\n')
u='00000000-0000-0000-0000-000000000001'
s=json.loads((r/'schemas/v1/contracts.json').read_text())['$defs']
def example(s):
 if 'const' in s:return s['const']
 if 'enum' in s:return s['enum'][0]
 if 'oneOf' in s:return example(s['oneOf'][0])
 if 'anyOf' in s:return None if any(x.get('type')=='null' for x in s['anyOf']) else example(s['anyOf'][0])
 t=s.get('type');t=t[0] if isinstance(t,list) else t
 if t=='object':return {k:example(v) for k,v in s['properties'].items()}
 if t=='array':return []
 if t=='boolean':return False
 if t=='integer':return 1
 if t=='null':return None
 if s.get('format')=='uuid':return u
 if s.get('format')=='date':return '2026-11-09'
 if s.get('format')=='date-time':return '2026-11-09T12:00:00Z'
 if 'pattern' in s:
  return '0'*64 if '64' in s['pattern'] else '1'
 return 'synthetic'
save('records.json',[example(x) for x in s['AuthoritativeRecordV1']['oneOf']])
save('command.json',example(s['CommandBodyV1']))
save('operations.json',[dict(example(s['CommandBodyV1']),operation=example(x)) for x in s['CommandBodyV1']['properties']['operation']['oneOf']])
save('canonical.json',{'source':'RFC 8785 section 3.2.3 UTF16 property sorting and section 3.2.2.2 escaping','utf16':{'input':{'\u20ac':'Euro Sign','\r':'Carriage Return','\ufb33':'Hebrew Letter Dalet With Dagesh','1':'One','\U0001f600':'Emoji: Grinning Face','\u0080':'Control','\u00f6':'Latin Small Letter O With Diaeresis'},'expected_keys':['\r','1','\u0080','\u00f6','\u20ac','\U0001f600','\ufb33']},'escaping':{'input':'\u0000\b\t\n\f\r"\\/€','expected':'"\\u0000\\b\\t\\n\\f\\r\\"\\\\/€"'}})
save('bounds.json',{'scale19':{'coefficient':'1','scale':19,'unit_id':u},'minimum':{'coefficient':str(-2**127),'scale':0,'unit_id':u},'overflow':{'coefficient':str(2**127),'scale':0,'unit_id':u}})
save('temporal.json',{'kind':'Date','civil':'2026-11-09','original_text':'11/09/2026'})
save('dtos.json',{name:example(s[name]) for name in ['ReadBasis','DomainCommitV1','CommandResultV1','PreviewV1','QueryV1','QueryResponseV1','DecisionPlan']})
save('requests.json',[example(v) for v in s['RequestV1']['oneOf']])

import hashlib
body=json.loads((f/'command.json').read_text())
preimage=json.dumps(body,ensure_ascii=False,sort_keys=True,separators=(',',':')).encode()
save('command-digest.json',{'canonical_utf8':preimage.decode(),'sha256':hashlib.sha256(preimage).hexdigest()})
