"""Inspect native/TypeSafe image routing, audit support and request accounting."""
import argparse
import collections
import gzip
import json
import math
from pathlib import Path
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('sources', nargs='+', type=Path)
args = parser.parse_args()
if len({p.name for p in args.sources}) != len(args.sources):
    parser.error('Source basenames must be unique')
def require(condition, message):
    if not condition:
        raise ValueError(message)
result = {}
audits = []
for source in args.sources:
    groups = collections.defaultdict(lambda: {'wire_calls':0, 'wire_errors':0, 'models':collections.Counter(),
        'images':collections.Counter(), 'audits':0})
    for line in gzip.open(source, 'rt'):
        if not line.strip():
            continue
        row = json.loads(line)
        provider = row['telemetry']['provider']
        group = groups[provider]
        wires = row['telemetry'].get('wire', [])
        if not any(w.get('error') for w in wires):
            require(row['model_calls'] == len(wires), 'Component requests under/over-counted')
            require(row['telemetry']['usage']['decision']['calls'] == len(wires), 'Aggregated usage call count differs')
        for wire in wires:
            request, response = wire['request'], wire.get('response') or {}
            group['wire_calls'] += 1
            group['wire_errors'] += bool(wire.get('error'))
            group['models'][response.get('model') or 'unknown'] += 1
            native = isinstance(request['questions'], list)
            questions = {q['name']:q for q in request['questions']} if native else request['questions']
            if native:
                images = [part for message in request['input'] if isinstance(message, dict)
                          for part in message.get('content', []) if isinstance(part, dict) and part.get('type') == 'input_image'] if isinstance(request['input'], list) else []
                if images:
                    require(len(images) == 1, 'Expected one native viewport image')
                    require(images[0]['image_url'].startswith('data:image/'), 'Non-inline screenshot')
                image_scope = 'targets' if images and all(q['type']=='choice' and n.endswith('_target') for n,q in questions.items()) else 'shared' if images else 'text'
            else:
                assets = request.get('assets') or {}
                if assets:
                    require(len(assets) == 1, 'Expected one TypeSafe viewport image')
                    ref = next(iter(assets))
                    shared = json.dumps(request['state']).count(ref)
                    counts = {n:json.dumps(q).count(ref) for n,q in questions.items()}
                    if shared:
                        require(shared == 1 and not any(counts.values()), 'Mixed TypeSafe image routing')
                    else:
                        require(any(counts.values()), 'Unreferenced TypeSafe asset')
                        require(all(not c or (n.endswith('_target') and questions[n]['type']=='choice') for n,c in counts.items()), 'Image outside target Choice')
                    image_scope = 'shared' if shared else 'targets'
                else:
                    image_scope = 'text'
            if image_scope == 'targets':
                require(provider in ['vision_activation','jev_activation-vision@jev-1.14.0'], 'Unexpected scoped image provider')
            group['images'][image_scope] += 1
            if set(questions) == {'required','performed','ready'}:
                require(image_scope == 'text', 'Audit includes an image')
                group['audits'] += 1
                if native:
                    require(all(q['type']=='predicate' for q in questions.values()), 'Native audit is not predicate-only')
                    values = {a['name']:a for a in response.get('answers', [])}
                    probabilities = {n:values.get(n, {}).get('probability') for n in questions}
                    instruction = json.JSONDecoder().raw_decode(questions['required']['instructions'])[0]
                    control = instruction['control']
                else:
                    require(all(q['type']=='noul' for q in questions.values()), 'TypeSafe audit is not Noul-only')
                    probabilities = {n:response.get('answers', {}).get(n, {}).get('noul') for n in questions}
                    control = questions['required']['instructions']['control']
                valid = all(isinstance(p,(float,int)) and not isinstance(p,bool) and math.isfinite(p) and 0<=p<=1 for p in probabilities.values())
                support = max(0, probabilities['required'] + probabilities['ready'] - probabilities['performed'] - 1) if valid else None
                audits.append({'source':source.name,'provider':provider,'task':row['task'],'repeat':row['repeat'],
                    'control':control,'probabilities':probabilities,'support':support,
                    'proposes_click':support is not None and support>0.5+1e-9,
                    'passed':bool(row['truth_ok'] and row['verdict_ok'])})
    result[source.name] = groups
print(json.dumps({'datasets':result,'audit_count':len(audits),'proposed_corrections':sum(a['proposes_click'] for a in audits),'audits':audits},indent=2))
