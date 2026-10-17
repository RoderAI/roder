"""Summarize diagnostic probes and pilot stdout without claiming missing wire data."""
import collections
import gzip
import json
from pathlib import Path
import re
ROOT = Path(__file__).resolve().parent
result = {}
for name in ['probe-blocked', 'probe-secret']:
    rows = [json.loads(line) for line in gzip.open(ROOT / (name + '.jsonl.gz'), 'rt') if line.strip()]
    variants = {}
    for variant in dict.fromkeys(r['variant'] for r in rows):
        group = [r for r in rows if r['variant'] == variant]
        selected = [next((a.get('choice') for a in (r.get('response') or {}).get('answers', [])
                         if a.get('name') == 'operation'), None) for r in group]
        summary = {'requests':len(group), 'errors':sum(bool(r.get('error')) for r in group),
                   'operation_counts':dict(collections.Counter(selected))}
        if name == 'probe-blocked':
            summary['missed_done'] = sum(r['expected_done'] and s != 'DONE' for r,s in zip(group,selected))
            summary['false_done'] = sum(not r['expected_done'] and s == 'DONE' for r,s in zip(group,selected))
        variants[variant] = summary
    result[name] = variants
for name in ['pilot-v1','pilot-v2']:
    pattern = r'repeat=(\d+) provider=(\S+) task=(\S+) pass=(true|false) false_green=(true|false)'
    matches = re.findall(pattern, gzip.open(ROOT / (name + '.log.gz'), 'rt').read())
    keys = [(n,p,t) for n,p,t,_,_ in matches]
    if len(keys) != len(set(keys)):
        raise SystemExit('Duplicate pilot outcome')
    result[name] = {p:{'attempts':len(group),'passed':sum(r[3]=='true' for r in group),
                       'false_green':sum(r[4]=='true' for r in group), 'wire_archive_available':False}
                   for p in dict.fromkeys(r[1] for r in matches)
                   for group in [[r for r in matches if r[1] == p]]}
(ROOT / 'development-summary.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
