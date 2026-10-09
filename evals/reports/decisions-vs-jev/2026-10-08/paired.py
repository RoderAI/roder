"""Paired outcomes and latency for visual Decisions versus each control.

Run analyze.py first to validate the dataset's expected task/repetition grid.
"""
import gzip
import json
import statistics
import sys

with gzip.open(sys.argv[1], 'rt') as source:
    rows = [json.loads(line) for line in source]
groups = {
    name: {(r['task'], r['telemetry']['repeat']): r for r in rows
           if r['telemetry']['provider'] == name}
    for name in ['original', 'effects', 'vision', 'jev']
}
out = {}
for control in ['original', 'effects', 'jev']:
    a, b = groups['vision'], groups[control]
    assert a.keys() == b.keys(), 'Unpaired attempts'
    pairs = [key for key in a if a[key]['pass'] and b[key]['pass']]
    out['vision_vs_' + control] = {
        'both_pass': len(pairs),
        'vision_only_pass': sum(a[k]['pass'] and not b[k]['pass'] for k in a),
        'control_only_pass': sum(b[k]['pass'] and not a[k]['pass'] for k in a),
        'neither_pass': sum(not a[k]['pass'] and not b[k]['pass'] for k in a),
        'vision_median_ms': statistics.median(a[k]['wall_ms'] for k in pairs),
        'control_median_ms': statistics.median(b[k]['wall_ms'] for k in pairs),
        'median_paired_delta_ms': statistics.median(
            a[k]['wall_ms'] - b[k]['wall_ms'] for k in pairs),
    }
print(json.dumps(out, indent=2))
