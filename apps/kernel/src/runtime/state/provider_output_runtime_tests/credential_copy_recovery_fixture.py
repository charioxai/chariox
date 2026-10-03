#!/usr/bin/env python3
# MP-08/MP-10/MP-11: fake OAuth refresh_token_reused and official Codex app-server protocol fixture.
# Native conversations survive process replacement in this fixture's private state.
import base64
import hashlib
import json
import os
from pathlib import Path
import socketserver
import struct
import sys
import threading
from urllib.parse import urlparse

if '--version' in sys.argv:
    print('codex-cli 0.159.3')
    sys.exit(0)
# Fixture servers die with their test process, including assertion failures.
if sys.platform.startswith('linux'):
    import ctypes, signal
    ctypes.CDLL(None).prctl(1, signal.SIGTERM)
home = Path(os.environ['CODEX_HOME'])
(Path(__file__).parent / ('synthetic-home-' + str(os.getpid()))).write_text(str(home))
state_path = home / 'synthetic-turns.json'
login_marker = home / 'synthetic-login-complete'
import http.server
import urllib.request
import urllib.error

class OAuth(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_POST(self):
        self.send_response(400)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(b'{"error":"refresh_token_reused"}')
    def do_GET(self):
        if self.path == '/consent':
            login_marker.write_text('synthetic-login')
            (home / 'auth.json').write_text('{"tokens":{"refresh_token":"synthetic-worker-login"}}')
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'Synthetic provider login')

oauth = http.server.ThreadingHTTPServer(('127.0.0.1', 0), OAuth)
threading.Thread(target=oauth.serve_forever, daemon=True).start()
oauth_url = 'http://127.0.0.1:' + str(oauth.server_port)

lock = threading.Lock()

def read_exact(stream, count):
    data = stream.read(count)
    if len(data) != count:
        raise EOFError()
    return data

class Handler(socketserver.StreamRequestHandler):
    def send(self, value, opcode=1):
        body = json.dumps(value).encode() if opcode == 1 else value
        length = len(body)
        head = bytes([128 | opcode, length]) if length < 126 else bytes([128 | opcode, 126]) + struct.pack('!H', length)
        self.wfile.write(head + body)
        self.wfile.flush()

    def handle(self):
        try:
            request_line = self.rfile.readline()
            if request_line.startswith(b'GET /readyz '):
                self.wfile.write(b'HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok')
                self.wfile.flush()
                return
            headers = {}
            while True:
                line = self.rfile.readline().strip()
                if not line:
                    break
                name, value = line.decode().split(':', 1)
                headers[name.lower()] = value.strip()
            key = headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11'
            accept = base64.b64encode(hashlib.sha1(key.encode()).digest()).decode()
            self.wfile.write(('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' + accept + '\r\n\r\n').encode())
            self.wfile.flush()
            while True:
                opcode, size = read_exact(self.rfile, 2)
                mask, size = size & 128, size & 127
                if size == 126:
                    size = struct.unpack('!H', read_exact(self.rfile, 2))[0]
                elif size == 127:
                    size = struct.unpack('!Q', read_exact(self.rfile, 8))[0]
                if size > 1024 * 1024:
                    raise ValueError('oversized fixture request')
                key = read_exact(self.rfile, 4) if mask else b''
                body = read_exact(self.rfile, size)
                if mask:
                    body = bytes(b ^ key[i % 4] for i, b in enumerate(body))
                opcode &= 15
                if opcode == 8:
                    return
                if opcode == 9:
                    self.send(body, 10)
                    continue
                request = json.loads(body)
                if 'id' not in request:
                    continue
                method, params = request['method'], request.get('params', {})
                with (Path(__file__).parent / ('synthetic-methods-' + str(os.getpid()))).open('a') as trace:
                    trace.write(method + '\n')
                result = {}
                with lock:
                    state = json.loads(state_path.read_text()) if state_path.exists() else {'threads': {}, 'resumes': []}
                    notification = None
                    if method == 'initialize':
                        if login_marker.exists(): (home / 'synthetic-ready').write_text('initialize')
                    elif method == 'account/read':
                        result = {'account': {'email':'fixture@chariox.test'} if login_marker.exists() else None, 'requiresOpenaiAuth': True}
                    elif method == 'account/login/start':
                        (home / 'synthetic-official-login-invoked').write_text('account/login/start')
                        result = {'type':'chatgptDeviceCode', 'loginId':'synthetic-login', 'verificationUrl':oauth_url + '/device', 'userCode':'SYNTHETIC'}
                    elif method == 'thread/start':
                        if login_marker.exists(): (home / 'synthetic-ready').write_text('thread/start')
                        tid = 'native-thread-' + str(len(state['threads']) + 1)
                        state['threads'][tid] = []
                        result = {'thread': {'id': tid}, 'model': 'fixture-model'}
                    elif method == 'thread/resume':
                        tid = params['threadId']
                        assert tid in state['threads']
                        state['resumes'].append(tid)
                        result = {'thread': {'id': tid}, 'model': 'fixture-model'}
                    elif method == 'turn/start':
                        turns = state['threads'][params['threadId']]
                        turns.append({'input': params['input'], 'project_label': os.environ.get('APP_LABEL'), 'previous': list(turns)})
                        turn_id = 'turn-' + str(len(turns))
                        result = {'turn': {'id': turn_id}}
                        if not login_marker.exists():
                            try:
                                urllib.request.urlopen(urllib.request.Request(oauth_url + '/token', data=b'synthetic')).read()
                            except urllib.error.HTTPError as error:
                                failure = json.loads(error.read())['error']
                            notification = {'method':'turn/completed', 'params':{'threadId':params['threadId'], 'turn':{'id':turn_id,'status':'failed','error':{'message':failure}}}}
                        else:
                            (home / 'synthetic-resumed').write_text(turn_id)
                            notification = {'method':'turn/completed', 'params':{'threadId':params['threadId'], 'turn':{'id':turn_id,'status':'completed'}}}
                    state_path.write_text(json.dumps(state))
                self.send({'id': request['id'], 'result': result})
                if notification: self.send(notification)
        except (EOFError, BrokenPipeError, ConnectionResetError):
            return

class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

endpoint = urlparse(sys.argv[sys.argv.index('--listen') + 1])
with Server((endpoint.hostname, endpoint.port), Handler) as server:
    server.serve_forever()
