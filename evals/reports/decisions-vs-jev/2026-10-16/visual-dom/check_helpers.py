import copy,gzip,json,os,shutil,subprocess,sys,tempfile
from pathlib import Path
root=Path(__file__).resolve().parent
def run(folder,script,*args,opt=False,ok=True,env=None):
 p=subprocess.run([sys.executable,*(['-O'] if opt else []),str(folder/script),*map(str,args)],capture_output=True,text=True,env=env)
 if (p.returncode==0)!=ok:raise RuntimeError((script,opt,p.returncode,p.stderr[-1000:]))
 return p
with tempfile.TemporaryDirectory(prefix='native-report-checks-') as tmp:
 folder=Path(tmp)/'visual-dom';folder.mkdir()
 for f in root.glob('*.py'):shutil.copy(f,folder/f.name)
 for name in ['protocol.json','freeze.json']:shutil.copy(root/name,folder/name)
 shutil.copy(root.parent/'analyze.py',folder.parent/'analyze.py')
 source=folder/'confirmation.jsonl.gz';shutil.copy(root/source.name,source)
 expected=json.loads((root/'confirmation-summary.json').read_text())
 for opt in [False,True]:
  run(folder,'summarize.py','confirmation',opt=opt)
  if json.loads((folder/'confirmation-summary.json').read_text())!=expected:raise RuntimeError('Summary changed')
 print('Valid native summary matches, normal and optimized Python')
 content=gzip.open(source,'rt').read();rows=[json.loads(l) for l in content.splitlines() if l.strip()]
 bad=copy.deepcopy(rows[0]);bad['task']='non-frozen-task'
 with gzip.open(source,'wt') as f:f.write('\n'.join(map(json.dumps,[bad,*rows[1:]])))
 for opt in [False,True]:run(folder,'summarize.py','confirmation',opt=opt,ok=False)
 print('Substituted task rejected, normal and optimized Python')
 with gzip.open(source,'wt') as f:f.write('\n\n'+content+'\n')
 baseline=json.loads(run(root,'inspect.py',root/source.name).stdout)
 for opt in [False,True]:
  if json.loads(run(folder,'inspect.py',source,opt=opt).stdout)!=baseline:raise RuntimeError('Blank lines changed inspection')
  run(folder,'inspect.py',source,root/source.name,opt=opt,ok=False)
 print('Blank lines preserve inspection; ambiguous duplicate basenames rejected, normal and optimized Python')
 bad=copy.deepcopy(rows[0]);bad['model_calls']+=1
 with gzip.open(source,'wt') as f:f.write(json.dumps(bad)+'\n')
 for opt in [False,True]:run(folder,'inspect.py',source,opt=opt,ok=False)
 print('Component call mismatch rejected, normal and optimized Python')
 # Audit evidence must follow DONE and carry typed predicate answers.
 audit_row=next(copy.deepcopy(r) for r in rows if r['telemetry']['provider']=='vision_activation' and any({q['name'] for q in w['request']['questions']}=={'required','performed','ready'} for w in r['telemetry']['wire']))
 audit_index=next(i for i,w in enumerate(audit_row['telemetry']['wire']) if {q['name'] for q in w['request']['questions']}=={'required','performed','ready'})
 for opt in [False,True]:
  bad=copy.deepcopy(audit_row)
  bad['telemetry']['wire'][audit_index-1]['response']['answers']=[]
  with gzip.open(source,'wt') as f:f.write(json.dumps(bad)+'\n')
  run(folder,'inspect.py',source,opt=opt,ok=False)
  bad=copy.deepcopy(audit_row)
  bad['telemetry']['wire'][audit_index]['response']['answers'][0]['type']='choice'
  with gzip.open(source,'wt') as f:f.write(json.dumps(bad)+'\n')
  inspected=json.loads(run(folder,'inspect.py',source,opt=opt).stdout)
  if inspected['audits'][0]['support'] is not None:raise RuntimeError('Wrong answer type gained audit support')
 print('Audits require preceding DONE and typed predicates, normal and optimized Python')
 # Synthetic complete grids test the all-failure analysis path, not model quality.
 protocol=json.loads((folder/'protocol.json').read_text())
 prototypes={}
 for row in rows:
  provider=row['telemetry']['provider']
  if provider not in prototypes:
   proto=copy.deepcopy(row);proto['telemetry']['wire']=[];proto['decisions']=[]
   proto['model_calls']=0;proto['telemetry']['browser_decision_attempts']=0
   proto['telemetry']['usage']['decision']={'calls':0,'input_tokens':0,'output_tokens':0}
   proto['truth_ok']=proto['verdict_ok']=proto['false_green']=False
   prototypes[provider]=proto
 for suite in protocol['suites']:
  with gzip.open(folder/(suite['name']+'.jsonl.gz'),'wt') as f:
   for task in suite['task_ids']:
    for repeat in range(1,suite['repeats']+1):
     for provider in protocol['providers'].split(','):
      row=copy.deepcopy(prototypes[provider]);row['task']=task;row['repeat']=repeat
      f.write(json.dumps(row)+'\n')
 for opt in [False,True]:
  run(folder,'summarize.py',opt=opt)
  if any(v['median_paired_delta_ms_both_pass'] is not None for v in json.loads((folder/'paired.json').read_text()).values()):raise RuntimeError('All-failure paired latency is not null')
  if any(v['passed'] for v in json.loads((folder/'totals.json').read_text()).values()):raise RuntimeError('All-failure grid gained a success')
 print('Synthetic complete all-failure grids retain zero passes and null paired latency, normal and optimized Python')
 for opt in [False,True]:
  env=os.environ.copy();env['JEV_EVAL_BINARY']='/usr/bin/true'
  p=run(folder,'reproduce.py','--output',Path(tmp)/'paid-output',opt=opt,ok=False,env=env)
  if 'Binary hash mismatch' not in p.stderr or (Path(tmp)/'paid-output').exists():raise RuntimeError('Wrong binary did not fail before output')
 print('Wrong binary refused before source lookup/output/calls, normal and optimized Python')
