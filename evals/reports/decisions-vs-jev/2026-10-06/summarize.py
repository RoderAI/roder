import json,statistics,pathlib,sys,collections
p=pathlib.Path(sys.argv[1]);rows=[json.loads(s) for s in p.read_text().splitlines()]
summary={}
for provider in ['decisions','jev']:
 r=[x for x in rows if x['telemetry']['provider']==provider]
 ok=[x for x in r if x['pass']]
 lat=[n for x in r for n in x['telemetry'].get('decision_latency_ms',[])]
 usage=[x['telemetry'].get('usage',{}).get('decision',{}).get('input_tokens') for x in r]
 summary[provider]={'attempts':len(r),'passed':len(ok),'pass_percent':round(100*len(ok)/len(r),2),'median_wall_seconds':statistics.median(x['wall_ms'] for x in r)/1000,'median_success_wall_seconds':statistics.median(x['wall_ms'] for x in ok)/1000 if ok else None,'total_wall_seconds':sum(x['wall_ms'] for x in r)/1000,'decision_calls':sum(x['model_calls'] for x in r),'median_decision_ms':statistics.median(lat) if lat else None,'parsed_decisions':len(lat),'reported_input_tokens':sum(x for x in usage if isinstance(x,int)),'unknown_usage_rows':sum(not isinstance(x,int) for x in usage),'models':sorted(set(str(x['telemetry'].get('model')) for x in r)),'failed_tasks':dict(collections.Counter(x['task'] for x in r if not x['pass']))}
pairs=collections.defaultdict(dict)
for row in rows:pairs[row['task'],row['telemetry']['repeat']][row['telemetry']['provider']]=row
both=[v for v in pairs.values() if len(v)==2 and all(x['pass'] for x in v.values())]
summary['pairs']={'complete':sum(len(v)==2 for v in pairs.values()),'both_pass':len(both),'decisions_only':sum(len(v)==2 and v['decisions']['pass'] and not v['jev']['pass'] for v in pairs.values()),'jev_only':sum(len(v)==2 and v['jev']['pass'] and not v['decisions']['pass'] for v in pairs.values()),'both_fail':sum(len(v)==2 and not any(x['pass'] for x in v.values()) for v in pairs.values()),'median_jev_over_decisions_wall_ratio_both_pass':statistics.median(v['jev']['wall_ms']/v['decisions']['wall_ms'] for v in both) if both else None,'median_decisions_seconds_both_pass':statistics.median(v['decisions']['wall_ms'] for v in both)/1000 if both else None,'median_jev_seconds_both_pass':statistics.median(v['jev']['wall_ms'] for v in both)/1000 if both else None}
print(json.dumps(summary,indent=2))
