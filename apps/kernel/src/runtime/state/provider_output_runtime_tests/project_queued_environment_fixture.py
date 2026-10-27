#!/usr/bin/env python3
# MP-08/MP-10/MP-11: credential-free Codex app-server protocol fixture.
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

state_path = Path(__file__).with_suffix('.json')
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
            self.rfile.readline()
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
                result = {}
                with lock:
                    state = json.loads(state_path.read_text()) if state_path.exists() else {'threads': {}, 'resumes': []}
                    if method == 'thread/start':
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
                        result = {'turn': {'id': 'turn-' + str(len(turns))}}
                    # MP-08/MP-10/MP-11: selection RPCs may finish after the
                    # acknowledged turn. Readers must see a complete snapshot.
                    pending = state_path.with_suffix(f'.{os.getpid()}.pending')
                    pending.write_text(json.dumps(state))
                    pending.replace(state_path)
                self.send({'id': request['id'], 'result': result})
        except (EOFError, BrokenPipeError, ConnectionResetError):
            return

class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

endpoint = urlparse(sys.argv[sys.argv.index('--listen') + 1])
with Server((endpoint.hostname, endpoint.port), Handler) as server:
    server.serve_forever()
