"""Validate exact frozen task grids and summarize retained attempts."""
import argparse
import collections
import gzip
import json
from pathlib import Path
import statistics
import subprocess
import sys
ROOT = Path(__file__).resolve().parent
protocol = json.loads((ROOT / 'protocol.json').read_text())
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('suites', nargs='*')
args = parser.parse_args()
known = {s['name']: s for s in protocol['suites']}
names = args.suites or list(known)
if set(names) - known.keys():
    raise SystemExit('Unknown suite')
allrows = []
for name in names:
    suite = known[name]
    source = ROOT / (name + '.jsonl.gz')
    rows = [json.loads(line) for line in gzip.open(source, 'rt') if line.strip()]
    keys = [(r['task'], r['repeat'], r['telemetry']['provider']) for r in rows]
    expected = {(t, n, p) for t in suite['task_ids'] for n in range(1, suite['repeats'] + 1)
                for p in protocol['providers'].split(',')}
    if len(keys) != len(set(keys)) or set(keys) != expected:
        raise SystemExit('Duplicate, missing or non-frozen task: ' + name)
    output = subprocess.check_output([sys.executable, str(ROOT.parent / 'analyze.py'),
        '--tasks', str(len(suite['task_ids'])), '--repeats', str(suite['repeats']),
        '--providers', protocol['providers'], str(source)], text=True)
    summary = json.loads(output)
    for provider, result in summary.items():
        group = [r for r in rows if r['telemetry']['provider'] == provider]
        # model_calls now includes component requests; separate browser attempts.
        wires = [w for r in group for w in r['telemetry'].get('wire', [])]
        native_images = sum(part.get('type') == 'input_image'
            for w in wires if isinstance(w['request'].get('input'), list)
            for message in w['request']['input'] if isinstance(message, dict)
            for part in message.get('content', []) if isinstance(part, dict))
        typesafe_images = sum(len(w['request'].get('assets') or {}) for w in wires)
        result['native_image_parts_sent'] = native_images
        result['typesafe_image_assets_sent'] = typesafe_images
        result['image_assets_sent'] = native_images + typesafe_images
        result['reported_model_calls'] = sum(r['model_calls'] for r in group)
        result['browser_decisions'] = sum(r['telemetry'].get('browser_decision_attempts') or 0 for r in group)
    (ROOT / (name + '-summary.json')).write_text(json.dumps(summary, indent=2) + '\n')
    allrows.extend(rows)
    print(name, {p: f"{s['passed']}/{s['attempts']}" for p, s in summary.items()})
if set(names) != known.keys():
    raise SystemExit(0)
if len(allrows) != protocol['total_attempts']:
    raise SystemExit('Unexpected total attempt count')
def passed(row):
    return row['truth_ok'] and row['verdict_ok']
totals = {}
groups = {}
for provider in protocol['providers'].split(','):
    group = [r for r in allrows if r['telemetry']['provider'] == provider]
    groups[provider] = {(r['task'], r['repeat']): r for r in group}
    wire = [w for r in group for w in r['telemetry'].get('wire', [])]
    usage = [r['telemetry'].get('usage', {}).get('decision', {}).get('input_tokens') for r in group]
    totals[provider] = {'attempts': len(group), 'passed': sum(passed(r) for r in group),
        'false_green': sum(r['false_green'] for r in group), 'wire_calls': len(wire),
        'wire_errors': sum(bool(w.get('error')) for w in wire),
        'reported_model_calls': sum(r['model_calls'] for r in group),
        'browser_decisions': sum(r['telemetry'].get('browser_decision_attempts') or 0 for r in group),
        'input_tokens': sum(n for n in usage if isinstance(n, int) and not isinstance(n, bool)),
        'unknown_input_token_rows': sum(not isinstance(n, int) or isinstance(n, bool) for n in usage),
        'median_wall_ms': statistics.median(r['wall_ms'] for r in group),
        'failures': [{'task': r['task'], 'repeat': r['repeat'], 'status': r['status'], 'failures': r['failures']}
                     for r in group if not passed(r)]}
(ROOT / 'totals.json').write_text(json.dumps(totals, indent=2) + '\n')
paired = {}
for subject in ['vision_dom', 'vision_activation']:
    controls = ['vision', 'jev_activation-vision@jev-1.14.0']
    if subject == 'vision_activation':
        controls.append('vision_dom')
    for control in controls:
        a, b = groups[subject], groups[control]
        if a.keys() != b.keys():
            raise SystemExit('Unpaired provider attempts')
        both = [k for k in a if passed(a[k]) and passed(b[k])]
        paired[subject + ':' + control] = {'both_pass': len(both),
            'subject_only_pass': [f'{k[0]}#{k[1]}' for k in a if passed(a[k]) and not passed(b[k])],
            'control_only_pass': [f'{k[0]}#{k[1]}' for k in a if passed(b[k]) and not passed(a[k])],
            'median_paired_delta_ms_both_pass': statistics.median(a[k]['wall_ms'] - b[k]['wall_ms'] for k in both) if both else None}
(ROOT / 'paired.json').write_text(json.dumps(paired, indent=2) + '\n')
