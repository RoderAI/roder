import gzip,hashlib,json,os,shutil,subprocess,sys,tempfile
from pathlib import Path
root=Path(__file__).resolve().parent.parent/'activation-audit'
def run(folder,script,*args,opt=False,ok=True,env=None):
 p=subprocess.run([sys.executable,*(['-O'] if opt else []),str(folder/script),*map(str,args)],capture_output=True,text=True,env=env)
 if (p.returncode==0)!=ok:raise RuntimeError((script,opt,p.returncode,p.stderr[-1000:]))
 return p
with tempfile.TemporaryDirectory(prefix='visual-report-checks-') as tmp:
 folder=Path(tmp)/'activation-audit';folder.mkdir()
 for f in root.glob('*.py'):shutil.copy(f,folder/f.name)
 for name in ['protocol.json','task-grid.json','freeze.json']:shutil.copy(root/name,folder/name)
 for name in ['analyze.py','paired.py']:shutil.copy(root.parent/name,folder.parent/name)
 protocol=json.loads((root/'protocol.json').read_text())
 for s in protocol['suites']:(folder/(s['name']+'.jsonl.gz')).symlink_to(root/(s['name']+'.jsonl.gz'))
 for opt in [False,True]:
  run(folder,'totals.py',opt=opt)
  for name in ['totals.json','pooled-paired.json']:
   if json.loads((folder/name).read_text())!=json.loads((root/name).read_text()):raise RuntimeError('Valid output changed')
 print('Valid pooled artifacts match under normal and optimized Python')
 source=folder/'confirmation.jsonl.gz';content=gzip.open(source,'rt').read();source.unlink()
 rows=[json.loads(x) for x in content.splitlines() if x.strip()]
 rows[0]['task']='non-frozen-task'
 with gzip.open(source,'wt') as f:f.write('\n'.join(map(json.dumps,rows)))
 for opt in [False,True]:
  run(folder,'summarize.py','confirmation',opt=opt,ok=False)
  run(folder,'totals.py',opt=opt,ok=False)
 print('Substituted tasks rejected by both validators, normal and optimized Python')
 for s in protocol['suites']:
  source=folder/(s['name']+'.jsonl.gz')
  if source.exists() or source.is_symlink():source.unlink()
  rows=[json.loads(x) for x in gzip.open(root/source.name,'rt') if x.strip()]
  for r in rows:
   r['truth_ok']=r['verdict_ok']=False;r['false_green']=False
   r['telemetry']['wire']=[]
  with gzip.open(source,'wt') as f:
   for row in rows:f.write(json.dumps(row)+'\n')
 for opt in [False,True]:
  run(folder,'totals.py',opt=opt)
  if any(v['median_paired_delta_ms_both_pass'] is not None for v in json.loads((folder/'pooled-paired.json').read_text()).values()):raise RuntimeError('All-failure median is not null')
 print('Complete all-failure grids produce null paired latency, normal and optimized Python')
 blank=folder/'blank.jsonl.gz'
 with gzip.open(blank,'wt') as f:f.write('\n\n'+content+'\n')
 expected=run(root,'inspect.py',root/'confirmation.jsonl.gz').stdout
 actual=run(folder,'inspect.py',blank).stdout
 if next(iter(json.loads(expected).values()))!=next(iter(json.loads(actual).values())):raise RuntimeError('Blank lines changed inspection')
 print('Blank lines preserve complete inspection')
 for opt in [False,True]:
  p=run(folder,'audits.py',source,root/source.name,opt=opt,ok=False)
  if 'Source basenames must be unique' not in p.stderr:raise RuntimeError('Ambiguous audit sources were not rejected first')
 print('Audit helper rejects ambiguous source basenames, normal and optimized Python')
 for opt in [False,True]:
  env=os.environ.copy();env['JEV_EVAL_BINARY']='/usr/bin/true'
  p=run(folder,'reproduce.py','--output',Path(tmp)/'paid-output',opt=opt,ok=False,env=env)
  if 'Binary hash mismatch' not in p.stderr or (Path(tmp)/'paid-output').exists():raise RuntimeError('Wrong binary did not fail before output')
 print('Wrong binary refused before campaign setup, normal and optimized Python')
