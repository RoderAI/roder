#!/usr/bin/env python3
"""Compare the same harness/model on a frozen baseline and the current source.

Live mode is explicitly guarded; authentication is read by Rust and never put
in shell arguments or output artifacts. Builds and fixtures use temporary paths.
"""
import argparse
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib

BASELINE = '816974be81b5cb7a1d249c14bc5bd26d7afb14ff'
DEPENDENCIES = ['roder-api', 'roder-core', 'roder-tools', 'roder-ext-openai-responses', 'roder-ext-jsonl-thread-store', 'roder-codex-auth', 'serde_json', 'tokio', 'anyhow', 'tempfile']

def toml(value):
    if isinstance(value,dict):
        return '{ '+', '.join(f'{key} = {toml(item)}' for key,item in value.items())+' }'
    return json.dumps(value)

def prepare(repo, scratch, label, source):
    crate=scratch/label
    (crate/'src').mkdir(parents=True,exist_ok=True)
    (crate/'src/main.rs').write_bytes(source.read_bytes())
    config=tomllib.loads((repo/'Cargo.toml').read_text())
    shared=config['workspace']['dependencies']
    lines=['[workspace]','[package]',f'name = "responses-parity-eval-{label}"','version = "0.0.0"','edition = "2024"','[dependencies]']
    for name in DEPENDENCIES:
        value=shared.get(name,'3' if name=='tempfile' else None)
        if value is None: raise ValueError(name)
        entry={'version':value} if isinstance(value,str) else dict(value)
        if 'path' in entry: entry['path']=str(repo/entry['path'])
        if name=='tokio': entry['features']=sorted(set(entry.get('features',[])+['full']))
        lines.append(f'{name} = {toml(entry)}')
    for profile in ['dev','test']:
        lines.append(f'[profile.{profile}]')
        for key,value in config['profile'][profile].items(): lines.append(f'{key} = {toml(value)}')
    (crate/'Cargo.toml').write_text('\n'.join(lines)+'\n')
    return crate

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--live',action='store_true')
    parser.add_argument('--model',default='gpt-6-luna')
    parser.add_argument('--scratch',type=Path)
    parser.add_argument('--transport',choices=['http','websocket'],default='http')
    parser.add_argument('--current-only',action='store_true')
    parser.add_argument('--compaction-smoke',action='store_true')
    args=parser.parse_args()
    if args.live and os.environ.get('RODER_RESPONSES_PARITY_LIVE')!='1': parser.error('live mode requires RODER_RESPONSES_PARITY_LIVE=1')
    source=Path(__file__).resolve().parent
    repo=source.parents[2]
    scratch=args.scratch or Path(tempfile.mkdtemp(prefix='roder-parity-eval-'))
    scratch.mkdir(parents=True,exist_ok=True)
    baseline=scratch/'baseline-source'
    if not (baseline/'.baseline-extracted').exists():
        baseline.mkdir(exist_ok=True)
        archive=subprocess.check_output(['git','archive',BASELINE],cwd=repo)
        with tarfile.open(fileobj=io.BytesIO(archive)) as archive:
            archive.extractall(baseline,members=[item for item in archive.getmembers() if not item.issym() and not item.islnk()],filter='data')
        (baseline/'.baseline-extracted').touch()
    print(f'Scratch: {scratch}',flush=True)
    env=dict(os.environ)
    env['RODER_PARITY_TEST_MODEL']=args.model
    # HTTP makes the live comparison control transport. WS has separate wire
    # fixtures; a small model sample cannot isolate transport effects reliably.
    env['RODER_RESPONSES_TRANSPORT']=args.transport
    builds=[('current',repo)] if args.current_only else [('current',repo),('baseline',baseline)]
    for label,root in builds:
        crate=prepare(root,scratch,label,source/'eval.rs')
        (crate/'Cargo.lock').write_bytes((repo/'Cargo.lock').read_bytes())
        command=['mise','exec','--','cargo','run','--manifest-path',str(crate/'Cargo.toml'),'--']
        if args.live: command+=['--live']
        if args.compaction_smoke: command+=['--compact-smoke']
        suffix='-'+args.transport if args.transport!='http' else ''
        if args.compaction_smoke: suffix+='-compaction'
        output=scratch/f'{label}-{ "live" if args.live else "offline" }{suffix}.json'
        build_env=dict(env)
        if label=='baseline': build_env['CARGO_TARGET_DIR']=str(scratch/'baseline-target')
        subprocess.run(command+[str(output)],cwd=repo,env=build_env,check=True)
    print(f'Results: {scratch}',flush=True)

if __name__=='__main__': main()
