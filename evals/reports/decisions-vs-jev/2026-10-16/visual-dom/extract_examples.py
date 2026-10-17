"""Extract small, attributable excerpts from retained native requests."""
import gzip
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parent

def rows(name):
    return [json.loads(line) for line in gzip.open(ROOT/name,'rt') if line.strip()]
def state(request):
    value=request['input']
    if isinstance(value,list):
        value=next(part['text'] for message in value for part in message['content'] if part['type']=='input_text')
    return json.loads(value)
row=next(r for r in rows('confirmation.jsonl.gz') if r['task']=='contact_form' and r['repeat']==1 and r['telemetry']['provider']=='vision_dom')
request=row['telemetry']['wire'][0]['request'];current=state(request)
example={'source':'confirmation.jsonl.gz','task':row['task'],'repeat':row['repeat'],'provider':row['telemetry']['provider'],'wire_index':0,
         'dom_snapshot':current['dom_snapshot'],
         'controls':[{k:e[k] for k in ['index','node','label','form'] if k in e} for e in current['elements']]}
(ROOT/'dom-example.json').write_text(json.dumps(example,indent=2)+'\n')
for row in rows('complex-dev.jsonl.gz'):
    if row['telemetry']['provider']!='vision_activation':continue
    wires=row['telemetry']['wire']
    for n,w in enumerate(wires):
        request=w['request'];questions=request['questions']
        if not isinstance(request['input'],list) or not any(part.get('type')=='input_image'
            for message in request['input'] for part in message.get('content',[])):continue
        if not questions or not all(q['type']=='choice' and q['name'].endswith('_target') for q in questions):continue
        visual=state(request);visual.pop('candidate_rectangles',None)
        paired=next(((i,x) for i,x in enumerate(wires) if isinstance(x['request']['input'],str)
                     and any(q['name']=='operation' for q in x['request']['questions'])
                     and state(x['request'])==visual),None)
        if paired is None:raise SystemExit('No matching text batch for image example')
        i,text=paired
        example={'source':'complex-dev.jsonl.gz','task':row['task'],'repeat':row['repeat'],'provider':row['telemetry']['provider'],
                 'text_batch':{'wire_index':i,'questions':[q['name'] for q in text['request']['questions']],'input_type':'text'},
                 'image_batch':{'wire_index':n,'questions':[q['name'] for q in questions],'input_type':'text+image'},
                 'same_observed_state_excluding_image_rectangles':True}
        (ROOT/'image-scope-example.json').write_text(json.dumps(example,indent=2)+'\n')
        print('Extracted native DOM and same-state scoped batch examples')
        raise SystemExit(0)
raise SystemExit('No scoped image batch found')
