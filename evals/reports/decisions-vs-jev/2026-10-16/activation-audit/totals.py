"""Aggregate the seven validated frozen datasets, retaining every failure."""
import collections
import gzip
import json
import statistics
from pathlib import Path
ROOT = Path(__file__).resolve().parent
protocol = json.loads((ROOT / 'protocol.json').read_text())
grid = json.loads((ROOT / 'task-grid.json').read_text())
rows = []
for suite in protocol['suites']:
    batch = [json.loads(line) for line in gzip.open(ROOT / (suite['name'] + '.jsonl.gz'), 'rt') if line.strip()]
    keys = [(r['task'], r['repeat'], r['telemetry']['provider']) for r in batch]
    expected = {(task, repeat, provider) for task in grid[suite['name']]
                for repeat in range(1, suite['repeats'] + 1) for provider in protocol['providers'].split(',')}
    if len(keys) != len(set(keys)) or set(keys) != expected:
        raise SystemExit('Duplicate, missing or non-frozen attempt: ' + suite['name'])
    rows.extend(batch)
providers = protocol['providers'].split(',')
if len(rows) != protocol['total_attempts']:
    raise SystemExit('Unexpected total attempt count; run summarize.py first')
result = {}
groups = {}
for provider in providers:
    group = [r for r in rows if r['telemetry']['provider'] == provider]
    if len(group) != protocol['attempts_per_provider']:
        raise SystemExit('Unexpected provider attempt count')
    groups[provider] = group
    wires = [w for r in group for w in r['telemetry']['wire']]
    result[provider] = {'attempts': len(group),
        'passed': sum(r['truth_ok'] and r['verdict_ok'] for r in group),
        'false_green': sum(r['false_green'] for r in group),
        'median_wall_ms': statistics.median(r['wall_ms'] for r in group),
        'wire_calls': len(wires),
        'input_tokens': sum(r['telemetry']['usage']['decision']['input_tokens'] for r in group),
        'failures': [{'task': r['task'], 'repeat': r['repeat'], 'status': r['status'],
                      'failures': r['failures']} for r in group if not (r['truth_ok'] and r['verdict_ok'])]}
(ROOT / 'totals.json').write_text(json.dumps(result, indent=2) + '\n')
paired = {}
subject = 'jev_activation-vision@jev-1.14.0'
for control in providers:
    if control == subject:
        continue
    def keyed(provider):
        group = groups[provider]
        mapped = {(r['task'], r['repeat']): r for r in group}
        if len(mapped) != len(group):
            raise SystemExit('Duplicate pooled attempt')
        return mapped
    a, b = keyed(subject), keyed(control)
    if a.keys() != b.keys():
        raise SystemExit('Unpaired pooled attempts')
    def passed(r):
        return r['truth_ok'] and r['verdict_ok']
    both = [k for k in a if passed(a[k]) and passed(b[k])]
    paired[control] = {'both_pass': len(both),
        'candidate_only_pass': [f'{k[0]}#{k[1]}' for k in a if passed(a[k]) and not passed(b[k])],
        'control_only_pass': [f'{k[0]}#{k[1]}' for k in a if passed(b[k]) and not passed(a[k])],
        'median_paired_delta_ms_both_pass': statistics.median(a[k]['wall_ms'] - b[k]['wall_ms'] for k in both) if both else None}
(ROOT / 'pooled-paired.json').write_text(json.dumps(paired, indent=2) + '\n')
print(json.dumps(result, indent=2))
