#!/usr/bin/env python3
"""Loopback-only browser fixture. Fake identity, real HTTP cookie and form state."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from http.cookies import SimpleCookie
import html
import json
from pathlib import Path
from urllib.parse import parse_qs

STATE = Path('/tmp/roder-cua-browser-oracle.json')
PROFILE = json.loads(Path('/opt/roder-cua/browser-profile.json').read_text())
oracle = {'events': [], 'submission': None, 'marker': PROFILE['marker']}

def persist():
    STATE.write_text(json.dumps(oracle, ensure_ascii=False, indent=2))

class Site(BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def signed_in(self):
        cookie = SimpleCookie(self.headers.get('Cookie', ''))
        return cookie.get('roder_test_session') is not None and cookie['roder_test_session'].value == PROFILE['session_cookie']
    def send(self, body, cookie=None):
        payload = ('<!doctype html><html lang="en"><meta charset="utf-8"><title>Roder Browser Test</title>'
            '<style>body{font:20px system-ui;background:#f5f7fb;color:#1b2435;max-width:840px;margin:65px auto}'
            'button,input,textarea{font:inherit;padding:12px;margin:10px 0}label{display:block}a{margin-right:25px}'
            'article{padding:30px;background:white;border:1px solid #ccc;border-radius:12px}</style>'
            '<h1>Roder X11 browser test</h1><article>'+body+'</article></html>').encode()
        self.send_response(200)
        if cookie:self.send_header('Set-Cookie', cookie)
        self.send_header('Content-Type','text/html; charset=utf-8');self.send_header('Content-Length',str(len(payload)))
        self.end_headers();self.wfile.write(payload)
    def do_GET(self):
        authenticated = self.signed_in()
        oracle['events'].append({'method':'GET','path':self.path,'signed_in':authenticated});persist()
        if not authenticated:
            self.send('<h2>Welcome</h2><p>Use this disposable test account.</p><form method="post" action="/login"><button>Continue as Roder Test User</button></form>');return
        links='<p>Signed in as Roder Test User</p><nav><a href="/dashboard">Dashboard</a><a href="/form">Create report</a><a href="/receipt">Saved report</a></nav>'
        if self.path == '/form':
            body='<h2>Create browser report</h2><form method="post" action="/submit"><label>Report title<input name="title" autocomplete="off" required></label><label>Report notes<textarea name="notes" rows="4" cols="45" required></textarea></label><button>Save browser report</button></form>'
        elif self.path == '/receipt':
            submission=oracle['submission']
            body=('<h2>Saved browser report</h2><p>Report saved successfully</p><h3>'+html.escape(submission['title'])+'</h3><p>'+html.escape(submission['notes'])+'</p>') if submission else '<h2>No saved report yet</h2>'
        else:body='<h2>Dashboard</h2><p>Your existing browser session is active.</p><p>Open Create report to continue.</p>'
        self.send(links+body)
    def do_POST(self):
        length=int(self.headers.get('Content-Length','0'))
        if not 0 <= length <= 32768:self.send_error(413);return
        data=parse_qs(self.rfile.read(length).decode())
        authenticated=self.signed_in()
        oracle['events'].append({'method':'POST','path':self.path,'signed_in':authenticated})
        cookie=None
        if self.path == '/login':cookie=f"roder_test_session={PROFILE['session_cookie']}; HttpOnly; SameSite=Strict; Path=/"
        elif self.path == '/submit' and authenticated:
            oracle['submission']={'title':data.get('title',[''])[0],'notes':data.get('notes',[''])[0]}
        else:self.send_error(403);persist();return
        persist();self.send_response(303)
        self.send_header('Location','/dashboard' if self.path == '/login' else '/receipt')
        if cookie:self.send_header('Set-Cookie',cookie)
        self.send_header('Content-Length','0');self.end_headers()

persist()
ThreadingHTTPServer(('127.0.0.1',8765), Site).serve_forever()
