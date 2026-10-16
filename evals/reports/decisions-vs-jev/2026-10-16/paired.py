"""Paired outcomes and latency between variants of one dataset.

Usage: paired.py DATA.jsonl[.gz] SUBJECT:CONTROL [SUBJECT:CONTROL ...]
Run analyze.py first to validate the dataset's task/repetition grid.
"""
import gzip
import json
import statistics
import sys

path = sys.argv[1]
opener = gzip.open if path.endswith('.gz') else open
with opener(path, 'rt') as source:
    rows = [json.loads(line) for line in source if line.strip()]
groups = {}
for r in rows:
    r.setdefault('pass', bool(r.get('verdict_ok') and r.get('truth_ok')))
    r.setdefault('repeat', r['telemetry'].get('repeat'))
    groups.setdefault(r['telemetry']['provider'], {})[(r['task'], r['repeat'])] = r
out = {}
for pair in sys.argv[2:]:
    subject, control = pair.split(':')
    a, b = groups[subject], groups[control]
    assert a.keys() == b.keys(), 'Unpaired attempts'
    both = [k for k in a if a[k]['pass'] and b[k]['pass']]
    out[f'{subject} vs {control}'] = {
        'both_pass': len(both),
        'subject_only_pass': sorted(f'{k[0]}#{k[1]}' for k in a if a[k]['pass'] and not b[k]['pass']),
        'control_only_pass': sorted(f'{k[0]}#{k[1]}' for k in a if b[k]['pass'] and not a[k]['pass']),
        'neither_pass': sum(not a[k]['pass'] and not b[k]['pass'] for k in a),
        'subject_median_ms_both_pass': statistics.median(a[k]['wall_ms'] for k in both) if both else None,
        'control_median_ms_both_pass': statistics.median(b[k]['wall_ms'] for k in both) if both else None,
        'median_paired_delta_ms': statistics.median(a[k]['wall_ms'] - b[k]['wall_ms'] for k in both) if both else None,
    }
print(json.dumps(out, indent=2))
