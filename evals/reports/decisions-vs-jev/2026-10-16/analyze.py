"""Summarize retained attempt rows; failures are never excluded.

Extends the 2026-10-08 analyzer with per-call decision latency, output tokens,
wire errors (including any HTTP 400 from the image path), image assets sent,
and the per-attempt time spent capturing screenshots.
"""
import argparse, collections, gzip, json, statistics
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--tasks', type=int, required=True)
parser.add_argument('--repeats', type=int, required=True)
parser.add_argument('--providers', required=True)
parser.add_argument('paths', nargs='+')
args = parser.parse_args()
rows = [json.loads(line) for path in args.paths
        for line in (gzip.open(path, 'rt').read() if path.endswith('.gz') else Path(path).read_text()).splitlines()
        if line.strip()]
for r in rows:
    # Current rows grade the verdict and the page separately; both must hold.
    r.setdefault('pass', bool(r.get('verdict_ok') and r.get('truth_ok')))
    r.setdefault('repeat', r['telemetry'].get('repeat'))
tasks = {r['task'] for r in rows}
keys = [(r['task'], r['repeat'], r['telemetry']['provider']) for r in rows]
if len(tasks) != args.tasks:
    raise SystemExit('Unexpected task count')
if len(keys) != len(set(keys)):
    raise SystemExit('Duplicate attempt')
providers = args.providers.split(',')
if set(keys) != {(t, n, p) for t in tasks for n in range(1, args.repeats + 1) for p in providers}:
    raise SystemExit('Incomplete paired dataset')


def median(values):
    values = list(values)
    return statistics.median(values) if values else None


def known(value):
    return value if isinstance(value, int) else 0


groups = collections.defaultdict(list)
for row in rows:
    groups[row['telemetry']['provider']].append(row)
summary = {}
for name in providers:
    group = groups[name]
    wires = [w for r in group for w in r['telemetry'].get('wire', [])]
    def answers(wire):
        found = (wire.get('response') or {}).get('answers') or []
        return [dict(a, name=a.get('name', n)) for n, a in found.items()] if isinstance(found, dict) else found
    refusals = collections.Counter(a.get('name') for w in wires for a in answers(w)
                                   if isinstance(a, dict) and a.get('type') == 'refusal')
    errors = collections.Counter(w['error'] for w in wires if w.get('error'))
    terminal = [(r['telemetry']['decisions'][-1], r['pass']) for r in group
                if r['status'] == 'done' and r['telemetry'].get('decisions') and r['telemetry']['decisions'][-1]['operation'] == 'DONE']
    thresholds = {}
    for t in [0, .5, .7, .8, .9, .95, .99]:
        accepted = [p for d, p in terminal if d['confidence'] >= t]
        thresholds[str(t)] = {'accepted': len(accepted), 'correct': sum(accepted), 'false_done': len(accepted) - sum(accepted),
                              'correct_rejected': sum(p for d, p in terminal if d['confidence'] < t)}
    models = collections.Counter(r['telemetry'].get('model') for r in group)
    usage = [r['telemetry'].get('usage', {}).get('decision', {}) for r in group]
    wire_out = sum(known(((w.get('response') or {}).get('usage') or {}).get('output_tokens')) for w in wires)
    laps = [r.get('watch', {}).get('laps', {}) for r in group]
    summary[name] = {
        'models': dict(models),
        'passed': sum(r['pass'] for r in group),
        'attempts': len(group),
        'pass_rate': round(sum(r['pass'] for r in group) / len(group), 4),
        'median_wall_ms': median(r['wall_ms'] for r in group),
        'median_wall_ms_passed': median(r['wall_ms'] for r in group if r['pass']),
        'browser_decisions': sum(r['model_calls'] for r in group),
        'wire_calls': len(wires),
        'median_wire_latency_ms': median(w['latency_ms'] for w in wires),
        'p90_wire_latency_ms': statistics.quantiles([w['latency_ms'] for w in wires], n=10)[-1] if len(wires) > 1 else None,
        'median_decision_latency_ms': median(ms for r in group for ms in r['telemetry'].get('decision_latency_ms', [])),
        'median_decide_ms_per_attempt': median(l.get('decide', {}).get('ms', 0) for l in laps),
        'screenshot_captures': sum(l.get('screenshot', {}).get('calls', 0) for l in laps),
        'median_screenshot_ms_per_attempt': median(l.get('screenshot', {}).get('ms', 0) for l in laps),
        'image_assets_sent': sum(len(w['request'].get('assets') or {}) for w in wires),
        'input_tokens': sum(known(u.get('input_tokens')) for u in usage),
        'output_tokens_wire': wire_out,
        'unknown_usage_rows': sum(not isinstance(u.get('input_tokens'), int) for u in usage),
        'wire_errors': dict(errors),
        'errored_rows': sum(r['status'] == 'error' for r in group),
        'statuses': dict(collections.Counter(r['status'] for r in group)),
        'refused_questions': dict(refusals),
        'inferred_done': sum(bool(r['status'] == 'done' and r['telemetry'].get('decisions') and r['telemetry']['decisions'][-1]['operation'] != 'DONE') for r in group),
        'false_done': sum(r['status'] == 'done' and not r['pass'] for r in group),
        'false_green': sum(bool(r.get('false_green')) for r in group),
        'per_task': {task: {'passed': sum(r['pass'] for r in group if r['task'] == task), 'attempts': sum(r['task'] == task for r in group)}
                     for task in dict.fromkeys(r['task'] for r in group)},
        'terminal_confidence_thresholds': thresholds,
    }
print(json.dumps(summary, indent=2))
