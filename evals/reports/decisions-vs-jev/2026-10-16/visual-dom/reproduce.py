"""Paid frozen browser campaign; refuses a different binary or existing outputs."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
ROOT = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, default=ROOT)
parser.add_argument('--source-checkout', type=Path, help='Original frozen build checkout; required before paid calls')
args = parser.parse_args()
binary = os.environ.get('JEV_EVAL_BINARY')
if not binary:
    raise SystemExit('Set JEV_EVAL_BINARY to the isolated frozen test executable')
freeze = json.loads((ROOT / 'freeze.json').read_text())
if hashlib.sha256(Path(binary).read_bytes()).hexdigest() != freeze['binary_sha256']:
    raise SystemExit('Binary hash mismatch; use the recorded build or declare a new verified freeze')
if hashlib.sha256((ROOT / 'protocol.json').read_bytes()).hexdigest() != freeze['protocol_sha256']:
    raise SystemExit('Protocol hash mismatch')
if not args.source_checkout:
    raise SystemExit('Set --source-checkout to the original frozen build checkout')
subprocess.run([sys.executable, str(ROOT / 'verify_source.py'), '--source-checkout', str(args.source_checkout)], check=True)
protocol = json.loads((ROOT / 'protocol.json').read_text())
args.output.mkdir(parents=True, exist_ok=True)
for suite in protocol['suites']:
    for extension in ['.log', '.jsonl.gz']:
        if (args.output / (suite['name'] + extension)).exists():
            raise SystemExit('Refusing to overwrite campaign output')
for suite in protocol['suites']:
    env = os.environ.copy()
    for key in ['BROWSER_EVAL_SUITE', 'DECISIONS_EVAL_HOLDOUT', 'JEV_EVAL_TASKS']:
        env.pop(key, None)
    env['CARGO_TARGET_DIR'] = str(args.output.resolve() / 'live-source')
    env.update(RUST_MIN_STACK='16777216', JEV_REQUIRE_CHROME='1', JEV_EVAL_CHOICE_ORDER='original',
               JEV_EVAL_REPEATS=str(suite['repeats']), DECISIONS_EVAL_VARIANTS=protocol['providers'])
    env.update({k: v for k, v in suite.items() if k in ['BROWSER_EVAL_SUITE','DECISIONS_EVAL_HOLDOUT','JEV_EVAL_TASKS']})
    log = args.output / (suite['name'] + '.log')
    with log.open('x') as handle:
        result = subprocess.run([binary, 'decisions_vs_jev', '--ignored', '--nocapture', '--test-threads=1'],
                                env=env, cwd=args.source_checkout, stdout=handle, stderr=subprocess.STDOUT)
    if result.returncode:
        raise SystemExit(str(log) + ' failed; retain partial data and logs')
    source = next((line.split('rows: ', 1)[1].strip() for line in log.read_text().splitlines() if 'rows: ' in line), None)
    if not source:
        raise SystemExit('No rows recorded: ' + str(log))
    content = Path(source).read_bytes()
    rows = [json.loads(line) for line in content.decode().splitlines() if line.strip()]
    keys = [(r['task'], r['repeat'], r['telemetry']['provider']) for r in rows]
    expected = {(task, repeat, provider) for task in suite['task_ids']
                for repeat in range(1, suite['repeats'] + 1) for provider in protocol['providers'].split(',')}
    if len(keys) != len(set(keys)) or set(keys) != expected:
        raise SystemExit('Recorded dataset differs from frozen task grid: ' + str(source))
    with (args.output / (suite['name'] + '.jsonl.gz')).open('xb') as raw:
        with gzip.GzipFile(fileobj=raw, mode='wb', mtime=0) as output:
            output.write(content)
    print('Completed', suite['name'], source, flush=True)
