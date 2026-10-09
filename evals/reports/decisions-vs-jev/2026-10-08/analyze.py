"""Summarize retained attempt rows; failures are never excluded."""
import argparse,collections,gzip,json,statistics,sys
from pathlib import Path
parser=argparse.ArgumentParser()
parser.add_argument('--tasks',type=int,required=True)
parser.add_argument('--repeats',type=int,required=True)
parser.add_argument('--providers',required=True)
parser.add_argument('paths',nargs='+')
args=parser.parse_args()
rows=[json.loads(line) for path in args.paths for line in (gzip.open(path,'rt').read() if path.endswith('.gz') else Path(path).read_text()).splitlines() if line.strip()]
tasks={r['task'] for r in rows}
keys=[(r['task'],r['telemetry']['repeat'],r['telemetry']['provider']) for r in rows]
assert len(tasks)==args.tasks, 'Unexpected task count'
assert len(keys)==len(set(keys)), 'Duplicate attempt'
assert set(keys)=={(t,n,p) for t in tasks for n in range(1,args.repeats+1) for p in args.providers.split(',')}, 'Incomplete paired dataset'
groups=collections.defaultdict(list)
for row in rows: groups[row['telemetry']['provider']].append(row)
summary={}
for name,group in groups.items():
 wires=[wire for r in group for wire in r['telemetry'].get('wire',[])]
 refusals=collections.Counter(a.get('name') for w in wires for a in (w.get('response') or {}).get('answers',[]) if a.get('type')=='refusal')
 def tokens(r):
  v=r['telemetry'].get('usage',{}).get('decision',{}).get('input_tokens')
  return v if isinstance(v,int) else 0
 terminal=[(r['telemetry'].get('decisions',[])[-1],r['pass']) for r in group if r['status']=='done' and r['telemetry'].get('decisions') and r['telemetry']['decisions'][-1]['operation']=='DONE']
 thresholds={}
 for t in [0,.5,.7,.8,.9,.95,.99]:
  accepted=[passed for d,passed in terminal if d['confidence']>=t]
  thresholds[str(t)]={'accepted':len(accepted),'correct':sum(accepted),'false_done':len(accepted)-sum(accepted),'correct_rejected':sum(p for d,p in terminal if d['confidence']<t)}
 summary[name]={'passed':sum(r['pass'] for r in group),'attempts':len(group),'median_wall_ms':statistics.median(r['wall_ms'] for r in group), 'wire_calls':len(wires) if wires else None,'browser_decisions':sum(r['model_calls'] for r in group),'input_tokens':sum(map(tokens,group)),'unknown_usage_rows':sum(not isinstance(r['telemetry'].get('usage',{}).get('decision',{}).get('input_tokens'),int) for r in group),'refused_questions':dict(refusals),'inferred_done':sum(bool(r['status']=='done' and r['telemetry'].get('decisions') and r['telemetry']['decisions'][-1]['operation']!='DONE') for r in group),'false_done':sum(r['status']=='done' and not r['pass'] for r in group),'per_task':{task:{'passed':sum(r['pass'] for r in group if r['task']==task),'attempts':sum(r['task']==task for r in group)} for task in dict.fromkeys(r['task'] for r in group)},'terminal_confidence_thresholds':thresholds}
print(json.dumps(summary,indent=2))
