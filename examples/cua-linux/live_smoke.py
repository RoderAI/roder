#!/usr/bin/env python3
"""Run the built Roder against an owned, provisioned Blaxel desktop; delete it.
Requires OPENAI_API_KEY and BLAXEL_API_KEY or BL_API_KEY in the environment.
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
from fixture import BlaxelTransport, grade
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "cua-desktop"))
from smoke_support import Rpc, model_proxy
from calculator_trace import calculation_buttons_passed, calculation_clicks

TOOLS = ['cua_list_windows','cua_get_window_state','cua_get_desktop_state','cua_click',
         'cua_press_key','cua_type_text','cua_set_value','cua_drag','cua_scroll','cua_move_cursor',
         'cua_bring_to_front','cua_set_window_frame']

def run(args):
    args.output.mkdir(parents=True,exist_ok=True)
    report = {'timestamp':datetime.now(timezone.utc).isoformat(),'sandbox':args.sandbox,
              'workspace':args.workspace,'backend':'Linux XFCE/X11','driver_version':'0.34.0',
              'public_transport':'Roder app-server JSON-RPC','model':args.model,
              'live_model_exercised':False,'native_wayland_exercised':False,'passed':False}
    records = []
    proxy = model_proxy(records)
    rpc = None
    try:
        with tempfile.TemporaryDirectory(prefix='roder-cua-eval-') as directory:
            temp = Path(directory)
            config = temp / 'config'
            config.mkdir()
            (config/'config.toml').write_text(f'''provider = "cua-eval-openai"
model = {json.dumps(args.model)}
[providers.cua-eval-openai]
api_key_env = "OPENAI_API_KEY"
base_url = "http://127.0.0.1:{proxy.server_port}"
[cua]
enabled = true
[tool_search]
mode = "explicit"
''')
            rpc = Rpc(args.roder,config,temp/'data')
            rpc.request('initialize',{'clientInfo':{'name':'cua-live-eval','version':'0.1'}})
            extensions = rpc.request('extensions/list',{})
            report['extension_registered'] = any(e['id']=='roder-ext-cua' for e in extensions['extensions'])
            workspace = rpc.request('workspace/create',{'roots':[{'path':str(temp),'name':'calculator-eval'}]})['workspace']
            thread = rpc.request('thread/start',{'workspaceId':workspace['id'],'rootId':workspace['defaultRootId'],
                'modelProvider':'cua-eval-openai','model':args.model,'toolAllowlist':TOOLS,
                'runner':{'providerId':'blaxel','workspace':'/home/cua/roder',
                    'config':{'sandbox_name':args.sandbox,'workspace':args.workspace,'image':'blaxel/cua-xfce:latest','cleanup':'detach-on-close'}}})['thread']['id']
            turn = rpc.request('turn/start',{'threadId':thread,'policyMode':'bypass',
                'prompt':"Use only the cua tools to find the native Galculator window. Clear it using the Escape keypress, then compute 6 multiplied by 7 by clicking each of the four buttons in this exact order: 6, *, 7, =. Do not type the operands or use keys for those four calculation steps. Use at least one pixel click with capture_id and one element_token click among those buttons. Observe fresh state before each input and visually verify the result is 42. This is an owned disposable desktop: if background input explicitly refuses, foreground delivery is permitted. Do not use shell, programmatic calculator state changes, or retry uncertain input. Finish once you have verified 42."})['turnId']
            report.update(thread_id=thread,turn_id=turn)
            deadline = time.monotonic()+300
            final = None
            while time.monotonic()<deadline:
                read = rpc.request('thread/read',{'threadId':thread,'includeTurns':True})
                turns = read.get('thread',{}).get('turns',[]) or []
                final = next((turn_record for turn_record in turns if turn_record['id']==turn),None)
                if final and final['status'] in ('completed','failed','interrupted'):
                    break
                time.sleep(1)
            if not final or final['status']!='completed':
                raise RuntimeError('Live Roder turn did not complete')
            trace = []
            for item in final['items']:
                if item.get('type')=='toolExecution':
                    image = (item.get('input') or {}).get('__view_image',{})
                    if image.get('image_url'):
                        pixels = base64.b64decode(image['image_url'].split(',',1)[1])
                        (args.output/f'{len(trace):02d}.png').write_bytes(pixels)
                        (args.output/'final.png').write_bytes(pixels)
                    trace.append({'tool':item['toolName'],'status':item['status'],
                                  'image_returned':bool(image),'arguments':{k:v for k,v in (item.get('input') or {}).items() if k!='__view_image'},'output':item.get('output',''),'error':item.get('error')})
            report.update(trace=trace,model_requests=records,
                live_model_exercised=any(r.get('http_status')==200 for r in records),
                model_received_images=any(r['image_blocks']>0 for r in records),
                independent_grader=grade(BlaxelTransport(args.sandbox,args.workspace)))
            successful = [t for t in trace if t['status']=='completed' and not t['error']]
            report['successful_pixel_click'] = any(t['tool']=='cua_click' and isinstance(t['arguments'].get('x'), (int,float)) for t in successful)
            report['successful_element_click'] = any(t['tool']=='cua_click' and t['arguments'].get('element_token') for t in successful)
            report['successful_keypress'] = any(t['tool']=='cua_press_key' for t in successful)
            report['calculation_clicks'] = calculation_clicks(trace)
            report['calculation_buttons_passed'] = calculation_buttons_passed(trace)
            report['passed'] = (report['successful_pixel_click'] and report['successful_element_click'] and report['successful_keypress']
                and any(t['image_returned'] for t in successful) and report['extension_registered'] and report['live_model_exercised']
                and report['model_received_images'] and report['independent_grader']['passed'] and report['calculation_buttons_passed'])
            if not report['passed']:
                raise RuntimeError('Native model/calculator acceptance failed')
    except Exception as error:
        report['error'] = str(error)
        raise
    finally:
        report['model_requests'] = records
        if rpc:
            rpc.close()
        proxy.shutdown()
        deletion = subprocess.run(['bl','delete','sandbox',args.sandbox,'--workspace',args.workspace],
                                  capture_output=True,timeout=60)
        report['delete_accepted'] = deletion.returncode==0
        for _ in range(20):
            probe = subprocess.run(['bl','get','sandbox',args.sandbox,'--workspace',args.workspace,'-o','json'],
                                   capture_output=True,text=True,timeout=30)
            try:
                status = json.loads(probe.stdout)[0]['status']
                report['cleanup_status'] = status if isinstance(status,str) else status.get('status',status)
            except (ValueError,KeyError,IndexError,TypeError):
                report['cleanup_status'] = 'unavailable'
            if report['cleanup_status']=='TERMINATED':
                break
            time.sleep(1)
        report['passed'] = report['passed'] and report['delete_accepted'] and report['cleanup_status']=='TERMINATED'
        (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    if not report['passed']:
        raise RuntimeError('Native acceptance or sandbox cleanup failed; inspect report.json')
    print(json.dumps({key:report[key] for key in ['passed','model_received_images','independent_grader','cleanup_status']}))

if __name__=='__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--roder',type=Path,required=True)
    parser.add_argument('--sandbox',required=True,help='Owned disposable sandbox; it will be deleted')
    parser.add_argument('--workspace',required=True)
    parser.add_argument('--model',default='gpt-5.4-mini')
    parser.add_argument('--output',type=Path,required=True)
    run(parser.parse_args())
