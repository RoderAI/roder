#!/usr/bin/env python3
"""Live model -> public Roder app-server -> local Cua -> native Calculator.
Uses an existing Calculator window; never closes it or captures other apps.
"""
import argparse
import base64
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'cua-desktop'))
from smoke_support import Rpc, model_proxy
from calculator_trace import calculation_clicks, calculation_buttons_passed

TOOLS=['cua_get_window_state','cua_click','cua_press_key','cua_bring_to_front']

def window_only(output):
    """Keep only the tested window subtree; Cua also returns global menu rows."""
    for key in ('observation','after_action'):
        observation=output.get(key) or {}
        keep=set()
        elements=[]
        for element in observation.get('elements',[]):
            if element.get('role')=='AXWindow' or element.get('parent_index') in keep:
                keep.add(element.get('element_index'))
                elements.append(element)
        if 'elements' in observation:
            observation['elements']=elements
        if 'tree_markdown' in observation:
            observation['tree_markdown']=observation['tree_markdown'].split('\n- ')[0]
    return output

def run(args):
    args.output.mkdir(parents=True,exist_ok=True)
    report={'timestamp':datetime.now(timezone.utc).isoformat(),'backend':'local-macos',
            'driver_version':'0.34.0','public_transport':'Roder app-server JSON-RPC',
            'model':args.model,'passed':False,'target':{'pid':args.pid,'window_id':args.window_id}}
    records=[]
    proxy=model_proxy(records)
    rpc=None
    try:
        with tempfile.TemporaryDirectory(prefix='roder-cua-mac-eval-') as directory:
            temp=Path(directory); config=temp/'config'; config.mkdir()
            (config/'config.toml').write_text(f'''provider="cua-eval-openai"
model={json.dumps(args.model)}
[providers.cua-eval-openai]
api_key_env="OPENAI_API_KEY"
base_url="http://127.0.0.1:{proxy.server_port}"
[cua]
enabled=true
backend="local-macos"
[tool_search]
mode="explicit"
''')
            rpc=Rpc(args.roder,config,temp/'data')
            rpc.request('initialize',{'clientInfo':{'name':'cua-macos-eval','version':'0.1'}})
            extensions=rpc.request('extensions/list',{})
            report['extension_registered']=any(e['id']=='roder-ext-cua' for e in extensions['extensions'])
            workspace=rpc.request('workspace/create',{'roots':[{'path':str(temp),'name':'mac-calculator-eval'}]})['workspace']
            thread=rpc.request('thread/start',{'workspaceId':workspace['id'],'rootId':workspace['defaultRootId'],
                'modelProvider':'cua-eval-openai','model':args.model,'toolAllowlist':TOOLS})['thread']['id']
            turn=rpc.request('turn/start',{'threadId':thread,'policyMode':'bypass','prompt':
                f'Use only cua tools on the native Calculator window pid={args.pid}, window_id={args.window_id}. '
                'Observe fresh window state, clear using Escape, then compute 6 multiplied by 7 by clicking four buttons in this exact order: 6, Multiply, 7, Equals. '
                'Use at least one pixel click with capture_id and one element_token click among those four buttons. Prefer background delivery for clicks. Do not type operands or press calculation keys. '
                'Set query to null for the full accessibility tree; query is a literal filter, not instructions. Set count=1 for each calculation click. Observe before every input, and visually verify the final display shows 42. Foreground delivery is explicitly permitted for this test; use delivery_mode foreground for keyboard. '
                'The provided process/window is the only allowed target. Do not click menu items or close any app. Do not use shell or programmatic state changes. Never retry uncertain input; stop and report it.'})['turnId']
            report.update(thread_id=thread,turn_id=turn)
            deadline=time.monotonic()+300; final=None
            while time.monotonic()<deadline:
                turns=rpc.request('thread/read',{'threadId':thread,'includeTurns':True}).get('thread',{}).get('turns',[]) or []
                final=next((item for item in turns if item['id']==turn),None)
                if final and final['status'] in ('completed','failed','interrupted'):break
                time.sleep(1)
            if not final or final['status']!='completed':raise RuntimeError('Live Calculator turn did not complete')
            trace=[]
            for item in final['items']:
                if item.get('type')!='toolExecution':continue
                image=(item.get('input') or {}).get('__view_image',{})
                if image.get('image_url'):
                    pixels=base64.b64decode(image['image_url'].split(',',1)[1])
                    (args.output/f'{len(trace):02d}.png').write_bytes(pixels)
                    (args.output/'final.png').write_bytes(pixels)
                output=item.get('output','')
                try:output=json.dumps(window_only(json.loads(output)),ensure_ascii=False)
                except (ValueError,TypeError):pass
                trace.append({'tool':item['toolName'],'status':item['status'],'image_returned':bool(image),
                    'arguments':{k:v for k,v in (item.get('input') or {}).items() if k!='__view_image'},'output':output,'error':item.get('error')})
            report['trace']=trace
            successful=[t for t in trace if t['status']=='completed' and not t['error']]
            grader=subprocess.run([str(args.grader),str(args.output/'final.png')],capture_output=True,text=True,check=True,timeout=30)
            report.update(trace=trace,model_requests=records,independent_grader=json.loads(grader.stdout),
                model_received_images=any(r['image_blocks']>0 for r in records),
                calculation_clicks=calculation_clicks(trace),calculation_buttons_passed=calculation_buttons_passed(trace),
                successful_pixel_click=any(t['tool']=='cua_click' and isinstance(t['arguments'].get('x'),(int,float)) for t in successful),
                successful_element_click=any(t['tool']=='cua_click' and t['arguments'].get('element_token') for t in successful),
                successful_keypress=any(t['tool']=='cua_press_key' for t in successful))
            report['passed']=all(report[k] for k in ('extension_registered','model_received_images','calculation_buttons_passed','successful_pixel_click','successful_element_click','successful_keypress')) and report['independent_grader']['passed']
            if not report['passed']:raise RuntimeError('Native macOS Calculator acceptance failed')
    except Exception as error:
        report['error']=str(error);raise
    finally:
        if rpc:rpc.close()
        proxy.shutdown()
        report['model_requests']=records
        report['app_server_stopped']=rpc is None or rpc.child.poll() is not None
        (args.output/'report.json').write_text(json.dumps(report,indent=2,ensure_ascii=False)+'\n')
    print(json.dumps({key:report[key] for key in ('passed','model_received_images','calculation_clicks','independent_grader','app_server_stopped')}))

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--roder',type=Path,required=True)
    parser.add_argument('--pid',type=int,required=True)
    parser.add_argument('--window-id',type=int,required=True)
    parser.add_argument('--grader',type=Path,required=True)
    parser.add_argument('--model',default='gpt-5.4-mini')
    parser.add_argument('--output',type=Path,required=True)
    run(parser.parse_args())
