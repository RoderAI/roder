"""Inspect every completion audit without claiming proposals were executed."""
import argparse
import gzip
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('sources', nargs='+', type=Path)
args = parser.parse_args()
records = []
for source in args.sources:
    opener = gzip.open if source.suffix == '.gz' else open
    with opener(source, 'rt') as handle:
        rows = [json.loads(line) for line in handle if line.strip()]
    for row in rows:
        previous = None
        for wire in row['telemetry'].get('wire', []):
            request = wire['request']
            questions = request.get('questions')
            if isinstance(questions, dict) and set(questions) == {'required', 'performed', 'ready'}:
                if not previous or previous.get('response', {}).get('answers', {}).get('operation', {}).get('choice') != 'DONE':
                    raise ValueError('Audit lacks its preceding DONE response')
                answers = (wire.get('response') or {}).get('answers', {})
                probabilities = {name: answers.get(name, {}).get('noul') for name in questions}
                valid = all(isinstance(p, (int, float)) and not isinstance(p, bool) and 0 <= p <= 1
                            and answers[name].get('type') == 'noul' for name, p in probabilities.items())
                support = max(0, probabilities['required'] + probabilities['ready'] - probabilities['performed'] - 1) if valid else None
                records.append({'source': source.name, 'task': row['task'], 'repeat': row['repeat'],
                    'provider': row['telemetry']['provider'], 'passed': bool(row['truth_ok'] and row['verdict_ok']),
                    'control': questions['required']['instructions']['control'],
                    'probabilities': probabilities, 'support': support,
                    'proposes_click': support is not None and support > 0.5 + 1e-9,
                    'wire_error': wire.get('error')})
            previous = wire
print(json.dumps({'audits': len(records), 'proposed_corrections': sum(r['proposes_click'] for r in records),
                  'records': records}, indent=2))
