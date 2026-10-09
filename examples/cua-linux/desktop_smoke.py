#!/usr/bin/env python3
"""Live model -> public Roder -> native XFCE desktop workflow.

Only desktop tools are advertised. Provisioning and read-only file grading
are separate. Deletes only the explicitly supplied owned sandbox by default.
"""
import argparse
import base64
from datetime import datetime, timezone
import json
from pathlib import Path
import sys
import tempfile
import time
from fixture import BlaxelTransport
from desktop_fixture import grade_desktop
from input_smoke import cleanup
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'cua-desktop'))
from smoke_support import Rpc, model_proxy

TOOLS = ['cua_list_apps', 'cua_list_windows', 'cua_get_window_state',
         'cua_get_desktop_state', 'cua_click', 'cua_drag', 'cua_scroll',
         'cua_press_key', 'cua_type_text', 'cua_set_value', 'cua_move_cursor',
         'cua_bring_to_front', 'cua_set_window_frame']


def observations(trace):
    for row in trace:
        try:
            output = json.loads(row['output'])
        except (ValueError, TypeError):
            continue
        for field in ('observation', 'after_action'):
            value = output.get(field)
            if isinstance(value, dict):
                yield value


def document_reopened(trace):
    """Prove close -> File Manager open -> fresh Writer observation in order.

    LibreOffice can keep its native window/PID when switching to Start Center
    and opening a document again. A different XID alone is neither necessary
    nor sufficient evidence of reopening the saved file.
    """
    saved = False
    closed = False
    file_open = False
    windows = {}
    for row in trace:
        try:
            result = json.loads(row['output'])
        except (ValueError, TypeError):
            continue
        args = row.get('arguments', {})
        target = (args.get('pid'), args.get('window_id'))
        previous = windows.get(target, {})
        if (closed and previous.get('app_name') == 'Thunar'
                and previous.get('window_title') == 'Documents'
                and row['status'] == 'completed' and not row.get('error')
                and ((row['tool'] == 'cua_click' and args.get('count') == 2)
                     or (row['tool'] == 'cua_press_key'
                         and str(args.get('key', '')).lower() in ('enter', 'return')
                         and not args.get('modifiers')))):
            file_open = True
        for field in ('observation', 'after_action'):
            state = result.get(field, {})
            if not isinstance(state, dict):
                continue
            title = str(state.get('window_title', ''))
            is_report = 'Reliability report' in title and 'LibreOffice Writer' in title
            if is_report and closed and file_open:
                return True
            if is_report:
                saved = True
                closed = False
                file_open = False
            if saved and title == 'LibreOffice':
                closed = True  # The document was closed to Start Center.
            if saved and row['tool'] == 'cua_list_windows' and isinstance(state.get('windows'), list):
                closed |= not any('Reliability report' in str(window.get('title', ''))
                                  and 'LibreOffice Writer' in str(window.get('title', ''))
                                  for window in state['windows'])
            if state.get('window_id') is not None:
                windows[(state.get('pid'), state['window_id'])] = state
    return False


def trace_checks(trace):
    states = list(observations(trace))
    titles = [str(state.get('window_title', '')) for state in states]
    successful = [row for row in trace if row['status'] == 'completed' and not row['error']]
    return {
        'gui_launcher_observed': any('Application Finder' in title for title in titles),
        'file_manager_observed': any(state.get('app_name') == 'Thunar' for state in states),
        'archive_navigated': any(state.get('app_name') == 'Thunar' and state.get('window_title') == 'Archive' for state in states),
        'writer_observed': any('LibreOffice Writer' in title for title in titles),
        'file_drag_dispatched': any(row['tool'] == 'cua_drag' for row in successful),
        'document_saved_and_reopened': document_reopened(trace),
        'desktop_observed': any(row['tool'] == 'cua_get_desktop_state' and row['image_returned'] for row in successful),
        'window_images_returned': any(row['tool'] == 'cua_get_window_state' and row['image_returned'] for row in successful),
    }


def record_trace(items, output):
    trace = []
    # App-server transcripts include updates to the same execution. Keep its
    # latest status rather than counting an in-progress row as another action.
    latest = {item['id']: item for item in items if item.get('type') == 'toolExecution'}
    for item in latest.values():
        if item.get('status') == 'inProgress':
            continue
        image = (item.get('input') or {}).get('__view_image', {})
        if image.get('image_url'):
            pixels = base64.b64decode(image['image_url'].split(',', 1)[1])
            (output / f'{len(trace):03d}.png').write_bytes(pixels)
            (output / 'final.png').write_bytes(pixels)
        trace.append({'tool': item['toolName'], 'status': item['status'],
            'image_returned': bool(image),
            'arguments': {k: v for k, v in (item.get('input') or {}).items() if k != '__view_image'},
            'output': item.get('output', ''), 'error': item.get('error')})
    (output / 'trace.json').write_text(json.dumps(trace, indent=2) + '\n')
    return trace


def run(args):
    args.output.mkdir(parents=True, exist_ok=True)
    report = {'timestamp': datetime.now(timezone.utc).isoformat(),
              'sandbox': args.sandbox, 'workspace': args.workspace,
              'backend': 'XFCE/X11', 'driver_version': '0.34.0',
              'model': args.model, 'public_transport': 'Roder app-server JSON-RPC',
              'passed': False, 'tools_advertised': TOOLS}
    rpc = None
    records = []
    proxy = model_proxy(records)
    transport = BlaxelTransport(args.sandbox, args.workspace)
    try:
        baseline = grade_desktop(transport)
        report['baseline'] = baseline
        if (baseline['checks']['source_moved'] or baseline['checks']['destination_exists']
                or baseline['checks']['document_exists']):
            raise RuntimeError('Requires a fresh seeded desktop; do not replay a partial workflow')
        profile = baseline['profile']
        with tempfile.TemporaryDirectory(prefix='roder-cua-desktop-eval-') as directory:
            temp = Path(directory); config = temp / 'config'; config.mkdir()
            (config / 'config.toml').write_text(f'''provider="cua-eval-openai"
model={json.dumps(args.model)}
[providers.cua-eval-openai]
api_key_env="OPENAI_API_KEY"
base_url="http://127.0.0.1:{proxy.server_port}"
[cua]
enabled=true
backend="runner"
max_image_dimension=1280
[tool_search]
mode="explicit"
''')
            rpc = Rpc(args.roder, config, temp / 'data')
            rpc.request('initialize', {'clientInfo': {'name': 'cua-desktop-eval', 'version': '0.1'}})
            extensions = rpc.request('extensions/list', {})
            report['extension_registered'] = any(e['id'] == 'roder-ext-cua' for e in extensions['extensions'])
            workspace = rpc.request('workspace/create', {'roots': [{'path': str(temp), 'name': 'desktop-eval'}]})['workspace']
            thread = rpc.request('thread/start', {'workspaceId': workspace['id'],
                'rootId': workspace['defaultRootId'], 'modelProvider': 'cua-eval-openai',
                'model': args.model, 'toolAllowlist': TOOLS,
                'runner': {'providerId': 'blaxel', 'workspace': '/home/cua/roder',
                    'config': {'sandbox_name': args.sandbox, 'workspace': args.workspace,
                        'image': 'blaxel/cua-xfce:latest', 'cleanup': 'detach-on-close'}}})['thread']['id']
            prompt = f'''Test this owned disposable full XFCE/X11 desktop using ONLY cua tools.
1. Observe the full desktop. Use the existing Application Finder GUI to launch the standard Thunar File Manager. Search by application name and launch the visible app; never use shell commands or a terminal. Super+d opens Application Finder from any foreground app if needed.
2. In File Manager navigate to {profile['root']} using its GUI location field. Observe its icon view. First click Source note.txt to select it, observe the selection, then drag the selected icon onto Archive with cua_drag and modifier ["shift"] to explicitly request a move; never use Ctrl, which requests a copy. Do not copy/cut/paste, menus, rename, delete or otherwise move the file. After a dispatched drag, observe the folder again: asynchronous file operations may not appear in the immediate after-action capture. If a completed gesture visibly had no effect, inspect the selection and geometry before attempting a freshly grounded correction. Open Archive and visibly verify the file is there, then navigate back up.
3. Open Application Finder with Super+d, search for LibreOffice Writer and launch it through its GUI. Create a report with heading 'Roder desktop reliability report', a paragraph containing the exact marker {profile['marker']}, and a paragraph explaining that you opened apps, navigated folders and dragged a file using GUI input through Roder and Cua.
4. Save the document as an OpenDocument Text (.odt) at {profile['document']} using Writer's Save dialog. Use cua_set_value for exact editable filename/path fields in a multi-window dialog. Never type via a terminal or manipulate files through an API.
5. Close Writer via its GUI (Alt+F4 is allowed). In File Manager navigate to the Documents folder inside {profile['root']} and double-click Reliability report.odt to reopen it. Observe Writer's reopened window and verify the heading and marker. Capture the full desktop again, then finish.
All input is authorized in foreground on this owned desktop. Bring an addressed app to the front when using pointer gestures, keys or focus. Observe each exact target before input and use fresh capture_id/element_token handles. PNG coordinates belong to that window image. Query is a literal accessibility filter: omit/null for a full tree. Do not reuse tokens across targets.
Input is never automatically retried. If an app launch, Save or Alt+F4 closes the addressed launcher/dialog/window, after-action capture can fail even when the action completed. Discover and observe current windows before deciding what to do next; never repeat that action blindly. A tool reporting effect=unverifiable means you must verify the app through fresh observation; it does not by itself mean transport failure. A timeout/disconnection with uncertain dispatch and no observable completed effect must stop. Do not run shell, browser automation, document injection, clipboard-based file moves, or non-cua tools.'''
            turn = rpc.request('turn/start', {'threadId': thread, 'policyMode': 'bypass', 'prompt': prompt})['turnId']
            report.update(thread_id=thread, turn_id=turn)
            deadline = time.monotonic() + args.timeout
            final = None
            printed = set()
            while time.monotonic() < deadline:
                turns = rpc.request('thread/read', {'threadId': thread, 'includeTurns': True}).get('thread', {}).get('turns', []) or []
                final = next((t for t in turns if t['id'] == turn), None)
                if final:
                    for item in final['items']:
                        if item.get('type') == 'toolExecution' and item.get('status') in ('completed', 'failed') and item['id'] not in printed:
                            printed.add(item['id'])
                            print(json.dumps({'tool': item['toolName'], 'status': item['status']}), flush=True)
                            record_trace(final['items'], args.output)
                    if final['status'] in ('completed', 'failed', 'interrupted'):
                        break
                time.sleep(1)
            if not final or final['status'] not in ('completed', 'failed', 'interrupted'):
                # End the public turn before tearing down its transport. An
                # already-dispatched atomic input still finishes under Cua's
                # fence; this never replays it or invents a passing result.
                rpc.request('turn/interrupt', {'threadId': thread, 'turnId': turn})
                report['deadline_exceeded'] = True
            trace = record_trace((final or {}).get('items', []), args.output)
            report['assistant_messages'] = [item.get('text', '') for item in (final or {}).get('items', []) if item.get('type') == 'agentMessage']
            report.update(trace=trace, turn_status=(final or {}).get('status'),
                          trace_checks=trace_checks(trace), independent_grader=grade_desktop(transport),
                          model_received_images=any(r['image_blocks'] > 0 for r in records))
            report['passed'] = (report['turn_status'] == 'completed'
                and report['extension_registered'] and report['model_received_images']
                and all(report['trace_checks'].values()) and report['independent_grader']['passed'])
    except Exception as error:
        report['error'] = str(error)
        raise
    finally:
        if rpc:
            rpc.close()
        proxy.shutdown()
        report['model_requests'] = records
        report['app_server_stopped'] = rpc is None or rpc.child.poll() is not None
        if not args.retain_sandbox:
            report['cleanup'] = cleanup(args.sandbox, args.workspace)
            report['passed'] = report['passed'] and report['cleanup']['delete_accepted'] and report['cleanup']['status'] == 'TERMINATED'
        else:
            report['sandbox_retained_for_evaluation'] = True
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report.get(k) for k in ('passed', 'trace_checks', 'cleanup')}), flush=True)
    if not report['passed']:
        raise RuntimeError('Full desktop acceptance failed; inspect report.json')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--roder', type=Path, required=True)
    parser.add_argument('--sandbox', required=True, help='Owned disposable provisioned desktop; deleted by default')
    parser.add_argument('--workspace', required=True)
    parser.add_argument('--model', default='gpt-5.4')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout', type=int, default=1200)
    parser.add_argument('--retain-sandbox', action='store_true', help='Keep owned sandbox for explicit further evaluation; caller must clean it up')
    run(parser.parse_args())
