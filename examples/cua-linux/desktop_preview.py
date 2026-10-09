#!/usr/bin/env python3
"""Read-only, loopback-only live view of an explicitly selected Blaxel desktop.

Screen reads use GDK independently of Cua, so watching never replaces the
agent's grounding captures. No desktop input or cloud credentials reach the
browser. Keep this process running while the owned sandbox is in use.
"""
import argparse
import base64
import binascii
import http.server
import json
import os
from pathlib import Path
import subprocess
import threading
import time
from fixture import BlaxelTransport

READER = '''import gi,base64,json
gi.require_version('Gdk','3.0')
from gi.repository import Gdk,GdkPixbuf
root=Gdk.get_default_root_window()
if root is None:raise RuntimeError('X11 root window unavailable')
pixels=Gdk.pixbuf_get_from_window(root,0,0,root.get_width(),root.get_height())
for maximum,quality in [(1280,65),(1024,55),(800,45)]:
 width=min(root.get_width(),maximum)
 height=round(root.get_height()*width/root.get_width())
 scaled=pixels.scale_simple(width,height,GdkPixbuf.InterpType.BILINEAR)
 ok,data=scaled.save_to_bufferv('jpeg',['quality'],[str(quality)])
 if not ok:raise RuntimeError('JPEG encoding failed')
 encoded=base64.b64encode(data).decode()
 if len(encoded)<55000:break
else:raise RuntimeError('Preview exceeds process output limit')
_,cursor_x,cursor_y=Gdk.Display.get_default().get_default_seat().get_pointer().get_position()
print(json.dumps({'jpeg':encoded,'cursor_x':cursor_x,'cursor_y':cursor_y,
 'width':root.get_width(),'height':root.get_height()}))
'''
CAPTURE = ("import pathlib,json,subprocess\n"
           "prefix=json.loads(pathlib.Path('/opt/roder-cua/session.json').read_text())\n"
           f"p=subprocess.run(prefix+['/usr/bin/python3','-c',{READER!r}],"
           "capture_output=True,text=True,timeout=10)\n"
           "print(p.stdout);raise SystemExit(p.returncode)\n")


def decode_frame(process):
    if process.get('exitCode') != 0:
        raise ValueError('Desktop capture failed')
    value = json.loads(process['stdout'])
    if not isinstance(value.get('jpeg'), str) or len(value['jpeg']) > 2_000_000:
        raise ValueError('Invalid screenshot size')
    pixels = base64.b64decode(value['jpeg'], validate=True)
    if not pixels.startswith(b'\xff\xd8') or not pixels.endswith(b'\xff\xd9'):
        raise ValueError('Invalid JPEG screenshot')
    for key in ('width', 'height'):
        if type(value.get(key)) is not int or not 0 < value[key] <= 16384:
            raise ValueError('Invalid desktop dimensions')
    dimensions = {key: value[key] for key in ('width', 'height')}
    for key, edge in [('cursor_x', 'width'), ('cursor_y', 'height')]:
        if type(value.get(key)) is int and 0 <= value[key] < dimensions[edge]:
            dimensions[key] = value[key]
    return pixels, dimensions


class Preview:
    def __init__(self, sandbox, workspace, duration=3600):
        self.transport = BlaxelTransport(sandbox, workspace)
        self.lock = threading.Lock()
        self.stopped = threading.Event()
        self.frame = None
        self.duration = duration
        self.status = {'sandbox': sandbox, 'backend': 'XFCE / X11',
                       'updated_at': None, 'sequence': 0, 'state': 'connecting'}

    def capture(self):
        deadline = time.monotonic() + self.duration
        while not self.stopped.is_set() and time.monotonic() < deadline:
            try:
                pixels, dimensions = decode_frame(self.transport.run_script(CAPTURE, timeout=15))
                with self.lock:
                    self.frame = pixels
                    self.status.update(dimensions, updated_at=time.time(),
                                       sequence=self.status['sequence'] + 1, state='live')
            except (RuntimeError, ValueError, KeyError, binascii.Error, OSError,
                    subprocess.TimeoutExpired):
                # Retain the last real frame, but make its age and loss of live
                # connectivity explicit. Never retry or dispatch desktop input.
                with self.lock:
                    self.status['state'] = 'capture unavailable'
            self.stopped.wait(2)
        with self.lock:
            self.status['state'] = 'preview ended'

    def snapshot(self):
        with self.lock:
            return self.frame, dict(self.status)


def handler(preview):
    html = Path(__file__).with_suffix('.html').read_bytes()

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            if self.headers.get('Host') != f'127.0.0.1:{self.server.server_port}':
                self.send_error(403)
                return
            frame, status = preview.snapshot()
            path = self.path.split('?', 1)[0]
            if path == '/':
                payload, mime = html, 'text/html; charset=utf-8'
            elif path == '/status':
                payload, mime = json.dumps(status).encode(), 'application/json'
            elif path == '/frame' and frame:
                payload, mime = frame, 'image/jpeg'
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Type', mime)
            self.send_header('Content-Length', str(len(payload)))
            self.send_header('Cache-Control', 'no-store')
            self.send_header('X-Content-Type-Options', 'nosniff')
            self.send_header('Content-Security-Policy',
                             "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'")
            self.end_headers()
            try:
                self.wfile.write(payload)
            except (BrokenPipeError, ConnectionResetError):
                pass

    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sandbox', required=True, help='Explicitly owned sandbox')
    parser.add_argument('--workspace', required=True)
    parser.add_argument('--port', type=int, default=0)
    parser.add_argument('--duration', type=int, default=3600, help='Seconds of live capture; default 1 hour, then serve the last frame without cloud requests')
    parser.add_argument('--receipt', type=Path)
    args = parser.parse_args()
    if not 0 < args.duration <= 43200:
        parser.error('--duration must be between 1 and 43200 seconds')
    preview = Preview(args.sandbox, args.workspace, args.duration)
    server = http.server.ThreadingHTTPServer(('127.0.0.1', args.port), handler(preview))
    url = f'http://127.0.0.1:{server.server_port}/'
    receipt = {'url': url, 'sandbox': args.sandbox, 'read_only': True,
               'bind': '127.0.0.1', 'pid': os.getpid()}
    if args.receipt:
        args.receipt.write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt), flush=True)
    thread = threading.Thread(target=preview.capture, daemon=True)
    thread.start()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        preview.stopped.set()
        server.server_close()
        thread.join(timeout=25)


if __name__ == '__main__':
    main()
