"""Non-secret app-server client and image-counting live Responses proxy."""
import http.server
import json
import os
import queue
import subprocess
import threading
import urllib.error
import urllib.request

class Rpc:
    def __init__(self, binary, config, data):
        self.child = subprocess.Popen([str(binary), 'app-server'], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            env={**os.environ, 'RODER_CONFIG_DIR': str(config), 'RODER_DATA_DIR': str(data)})
        self.messages = queue.Queue()
        self.identifier = 0
        def read():
            for line in self.child.stdout:
                try:
                    self.messages.put(json.loads(line))
                except ValueError:
                    pass
        threading.Thread(target=read, daemon=True).start()
        threading.Thread(target=lambda: list(self.child.stderr), daemon=True).start()

    def request(self, method, params):
        self.identifier += 1
        self.child.stdin.write(json.dumps({'jsonrpc':'2.0', 'id':self.identifier,
            'method':method, 'params':params}) + '\n')
        self.child.stdin.flush()
        while True:
            message = self.messages.get(timeout=60)
            if message.get('id') != self.identifier:
                continue
            if 'error' in message:
                raise RuntimeError(f'Roder RPC {method} failed: {message["error"]["code"]}')
            return message['result']

    def close(self):
        self.child.stdin.close()
        try:
            self.child.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.child.terminate()
            self.child.wait(timeout=10)

def model_proxy(records):
    class Proxy(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass
        def do_POST(self):
            body = self.rfile.read(int(self.headers['Content-Length']))
            parsed = json.loads(body)
            def images(value):
                if isinstance(value, dict):
                    return int(value.get('type') == 'input_image') + sum(images(v) for v in value.values())
                if isinstance(value, list):
                    return sum(images(v) for v in value)
                return 0
            record = {'model':parsed.get('model'), 'image_blocks':images(parsed)}
            records.append(record)
            request = urllib.request.Request('https://api.openai.com/v1' + self.path, data=body,
                headers={'Authorization':self.headers['Authorization'], 'Content-Type':'application/json'})
            try:
                with urllib.request.urlopen(request, timeout=120) as response:
                    payload = response.read()
                    record['http_status'] = response.status
                    self.send_response(response.status)
                    self.send_header('Content-Type', response.headers.get('Content-Type','text/event-stream'))
                    self.send_header('Content-Length', str(len(payload)))
                    self.end_headers()
                    self.wfile.write(payload)
            except urllib.error.HTTPError as error:
                record['http_status'] = error.code
                self.send_response(error.code)
                self.end_headers()
                self.wfile.write(b'{"error":{"message":"Live model request failed; inspect account access"}}')
    server = http.server.ThreadingHTTPServer(('127.0.0.1',0), Proxy)
    threading.Thread(target=server.serve_forever,daemon=True).start()
    return server
