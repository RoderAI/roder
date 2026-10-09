#!/usr/bin/env python3
"""Run native input/recovery qualification and delete two owned sandboxes."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time


def cleanup(name,workspace):
    delete = subprocess.run(['bl','delete','sandbox',name,'--workspace',workspace],capture_output=True,timeout=60)
    status = 'unavailable'
    for _ in range(20):
        probe = subprocess.run(['bl','get','sandbox',name,'--workspace',workspace,'-o','json'],capture_output=True,text=True,timeout=30)
        try:status=json.loads(probe.stdout)[0]['status']
        except (ValueError,KeyError,IndexError,TypeError):pass
        if status=='TERMINATED':break
        time.sleep(1)
    return {'delete_accepted':delete.returncode==0,'status':status}


def run(args):
    if len(set(args.sandbox))!=2:
        raise ValueError('two distinct owned sandboxes required')
    args.output.mkdir(parents=True,exist_ok=True)
    env=os.environ.copy()
    env.update(RODER_CUA_INPUT_SANDBOXES=','.join(args.sandbox),RODER_CUA_INPUT_WORKSPACE=args.workspace,
               RODER_CUA_INPUT_OUTPUT=str(args.output.resolve()))
    process=None
    try:
        process=subprocess.run([str(args.binary.resolve())],env=env,capture_output=True,text=True,timeout=300)
        if process.returncode:
            raise RuntimeError('Native input test failed: '+process.stderr[-1200:])
    finally:
        report_path=args.output/'report.json'
        report=json.loads(report_path.read_text()) if report_path.exists() else {'passed':False}
        report['cleanup']={name:cleanup(name,args.workspace) for name in args.sandbox}
        report['passed']=report['passed'] and all(c['delete_accepted'] and c['status']=='TERMINATED' for c in report['cleanup'].values())
        report_path.write_text(json.dumps(report,indent=2)+'\n')
    if not report['passed']:
        raise RuntimeError('Input qualification or cleanup failed; inspect report.json')
    print(json.dumps({'passed':report['passed'],'checks':report['checks'],'cleanup':report['cleanup']}))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True,help='Built input_smoke Rust example')
    parser.add_argument('--sandbox',action='append',required=True,help='Owned provisioned desktop; supply twice; both will be deleted')
    parser.add_argument('--workspace',required=True)
    parser.add_argument('--output',type=Path,required=True)
    run(parser.parse_args())
