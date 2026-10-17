import os,json,subprocess,gzip
from pathlib import Path
ROOT=Path(__file__).resolve().parent
BINARY=os.environ.get('JEV_EVAL_BINARY')
if not BINARY:raise SystemExit('Set JEV_EVAL_BINARY to an isolated copy built from freeze.json code_commit')
protocol=json.loads((ROOT/'protocol.json').read_text())
for suite in protocol['suites']:
 env=os.environ.copy()
 for key in ['BROWSER_EVAL_SUITE','DECISIONS_EVAL_HOLDOUT','JEV_EVAL_TASKS']:env.pop(key,None)
 env.update(JEV_REQUIRE_CHROME='1',JEV_EVAL_CHOICE_ORDER='original',JEV_EVAL_REPEATS=str(suite['repeats']),DECISIONS_EVAL_VARIANTS=protocol['providers'])
 env.update({k:v for k,v in suite.items() if k not in ['name','repeats']})
 out=ROOT/(suite['name']+'.jsonl.gz')
 if out.exists():raise SystemExit('Refusing to overwrite '+str(out))
 log=ROOT/(suite['name']+'.log')
 with log.open('x') as handle:
  result=subprocess.run([BINARY,'decisions_vs_jev','--ignored','--nocapture','--test-threads=1'],env=env,stdout=handle,stderr=subprocess.STDOUT)
 if result.returncode:raise SystemExit(str(log)+' failed')
 source=next((line.split('rows: ',1)[1].strip() for line in log.read_text().splitlines() if 'rows: ' in line),None)
 if not source:raise SystemExit('Missing rows in '+str(log))
 with out.open('xb') as raw:
  with gzip.GzipFile(fileobj=raw,mode='wb',mtime=0) as handle:handle.write(Path(source).read_bytes())
 print('Completed',suite['name'],source,flush=True)
