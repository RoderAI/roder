"""Summarize DONE judgments on correlated recorded states, not task accuracy."""
import collections
import gzip
import json
from pathlib import Path
ROOT = Path(__file__).resolve().parent
summary = {}
for name, head in [('wording', 'operation'), ('modalities', 'operation'), ('joint', 'browser_action')]:
    groups = collections.defaultdict(lambda: dict(requests=0, errors=0, false_done=0, missed_done=0))
    for line in gzip.open(ROOT / ('probe-' + name + '.jsonl.gz'), 'rt'):
        row = json.loads(line)
        group = groups[row['variant']]
        group['requests'] += 1
        group['errors'] += bool(row.get('error'))
        if row.get('error'):
            continue
        operation = row['response']['answers'][head]['choice']
        group['false_done'] += operation == 'DONE' and not row['expected_done']
        group['missed_done'] += operation != 'DONE' and row['expected_done']
    summary[name] = groups
summary['interpretation'] = ('Repeated judgments of correlated recorded states, not independent browser outcomes. '
    'Non-DONE is not proof the next action is correct. Other probe formats retained raw; '
    'replan mismatches excluded from matched-state validation.')
print(json.dumps(summary, indent=2))
