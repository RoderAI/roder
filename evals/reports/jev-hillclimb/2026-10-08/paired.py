"""Paired outcome changes and latency on pairs where both providers passed."""
import collections,gzip,json,statistics,sys
rows=[json.loads(line) for path in sys.argv[1:] for line in (gzip.open(path,'rt').read() if path.endswith('.gz') else open(path).read()).splitlines()]
by={(r['task'],r['telemetry']['repeat'],r['telemetry']['provider']):r for r in rows}
assert len(by)==len(rows), "Duplicate paired attempt"
result={}
for reference in ['jev','vision']:
 pairs=[(r,by[(r['task'],r['telemetry']['repeat'],reference)]) for r in rows if r['telemetry']['provider']=='jev_workflow']
 both=[(a,b) for a,b in pairs if a['pass'] and b['pass']]
 result[reference]={'pairs':len(pairs),'candidate_only_pass':sum(a['pass'] and not b['pass'] for a,b in pairs),'reference_only_pass':sum(b['pass'] and not a['pass'] for a,b in pairs),'both_pass':len(both),'both_fail':sum(not a['pass'] and not b['pass'] for a,b in pairs),'candidate_median_ms_on_both_pass':statistics.median(a['wall_ms'] for a,b in both) if both else None,'reference_median_ms_on_both_pass':statistics.median(b['wall_ms'] for a,b in both) if both else None,'median_paired_ms_saved':statistics.median(b['wall_ms']-a['wall_ms'] for a,b in both) if both else None}
result['side_effects']={}
for provider in sorted({r['telemetry']['provider'] for r in rows}):
 group=[r for r in rows if r['telemetry']['provider']==provider]
 probes=[r['telemetry'].get('dom_probes',{}).get('window.audit.forbidden',{}).get('Ok') for r in group]
 result['side_effects'][provider]={'attempts_with_audit':sum(isinstance(p,int) for p in probes),'attempts_with_forbidden_mutations':sum(isinstance(p,int) and p>0 for p in probes),'false_done_tasks':[{'task':r['task'],'repeat':r['telemetry']['repeat'],'failures':r['failures']} for r in group if r['status']=='done' and not r['pass']]}
print(json.dumps(result,indent=2))
