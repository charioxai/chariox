#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11: executable unscored official-evaluator/CDP fixture.
Creates only its own loopback server, Chrome and profile. No Room/provider/VM.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import types
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PINS = {'webmall-evaluator.py': '999783fbfcffdc776742251d255b0c2e3cbc7e73cf7660f918c06fa6a6726d52',
        'checkpoints.py': '8d36a60d6733d4b52fc1a91177c673d1441047001a3976fb5a9f3261cf1eeef9'}


def load_official(root):
    for name, digest in PINS.items():
        assert hashlib.sha256((root / name).read_bytes()).hexdigest() == digest, 'official source drift'
    pkg = types.ModuleType('qualified_webmall')
    pkg.__path__ = [str(root)]
    sys.modules[pkg.__name__] = pkg
    for name, filename in [('checkpoints', 'checkpoints.py'), ('evaluator', 'webmall-evaluator.py')]:
        spec = importlib.util.spec_from_file_location(f'{pkg.__name__}.{name}', root / filename)
        mod = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = mod
        spec.loader.exec_module(mod)
    return sys.modules['qualified_webmall.evaluator'], sys.modules['qualified_webmall.checkpoints']


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--upstream', required=True, type=Path)
    ap.add_argument('--state-root', required=True, type=Path)
    ap.add_argument('--output', required=True, type=Path)
    ap.add_argument('--chrome', default='/usr/bin/google-chrome')
    args = ap.parse_args()
    from qualification_paths import require_external
    require_external(args.state_root); require_external(args.output)
    from playwright.sync_api import sync_playwright
    from websockets.sync.client import connect
    import requests
    sys.path.insert(0, str(Path(__file__).parent))
    spec = importlib.util.spec_from_file_location('freshness', Path(__file__).with_name('webmall-freshness.py'))
    freshness = importlib.util.module_from_spec(spec); spec.loader.exec_module(freshness)
    FirstDoneGrade = freshness.FirstDoneGrade
    official, checkpoints = load_official(args.upstream)
    rows = []
    synthetic_urls = ['https://shop.fixture/product/alpha', 'https://shop.fixture/product/beta']

    class Fixture(BaseHTTPRequestHandler):
        def do_GET(self):
            body = ('<title>MP-08 MP-10 MP-11 local fixture</title><form onsubmit="event.preventDefault();'
                    'document.querySelector(\'#submittedResult\').textContent=document.querySelector(\'textarea\').value">'
                    '<textarea></textarea><button>Submit synthetic result</button></form><pre id="submittedResult"></pre>')
            self.send_response(200); self.send_header('Content-Type', 'text/html; charset=utf-8'); self.end_headers()
            self.wfile.write(body.encode())
        def log_message(self, *a):
            pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    base = f'http://127.0.0.1:{server.server_port}'
    os.environ['FRONTEND_URL'] = f'{base}/frontend'
    args.state_root.mkdir(parents=True, exist_ok=True)
    try:
        with tempfile.TemporaryDirectory(prefix='webmall-fixture-', dir=args.state_root) as scratch:
            # Root fixture only. This is not a production Chromium launch contract.
            with open(Path(scratch)/'chrome.log', 'wb') as log:
                chrome = subprocess.Popen([args.chrome, '--headless=new', '--no-sandbox', '--disable-dev-shm-usage',
                    '--remote-debugging-port=0', f'--user-data-dir={scratch}/profile', 'about:blank'], stdout=log, stderr=log)
                try:
                    active = Path(scratch)/'profile/DevToolsActivePort'
                    deadline = time.monotonic()+20
                    while not active.exists():
                        assert time.monotonic() < deadline and chrome.poll() is None, 'owned Chrome startup failed'
                        time.sleep(.05)
                    port = int(active.read_text().splitlines()[0]); cdp = f'http://127.0.0.1:{port}'
                    with sync_playwright() as pw:
                        browser = pw.chromium.connect_over_cdp(cdp)
                        context = browser.contexts[0]
                        pages = [context.new_page(), context.new_page()]
                        for p in context.pages:
                            p.goto(f'{base}/shop')
                        for label, text, score, wrong in [
                            ('right', '\n'.join(synthetic_urls), 1., []),
                            ('partial', synthetic_urls[0], .5, []),
                            ('wrong', 'https://shop.fixture/product/wrong', 0., ['https://shop.fixture/product/wrong'])]:
                            for p in pages:
                                p.goto(f'{base}/shop')
                            targets = requests.get(cdp+'/json/list', timeout=3).json()
                            target_sockets = [t['webSocketDebuggerUrl'] for t in targets if t['type']=='page' and t['url']==f'{base}/shop'][:2]
                            assert len(target_sockets)==2
                            sockets = []; submitted_at = []
                            try:
                                for url in target_sockets:
                                    ws = connect(url, open_timeout=5); sockets.append(ws)
                                    call_id = 0
                                    def send(method, params):
                                        nonlocal call_id
                                        call_id += 1; ws.send(json.dumps({'id':call_id,'method':method,'params':params}))
                                        while True:
                                            msg = json.loads(ws.recv(timeout=5))
                                            if msg.get('id')==call_id:
                                                assert 'error' not in msg
                                                return msg
                                    send('Page.enable', {})
                                    send('Page.navigate', {'url':f'{base}/frontend'})
                                    while True:
                                        msg=json.loads(ws.recv(timeout=5))
                                        if msg.get('method')=='Page.loadEventFired': break
                                    # Synthetic external actor submits through the fixture UI.
                                    send('Runtime.evaluate', {'expression':f'document.querySelector("textarea").value={json.dumps(text)}; document.querySelector("button").click()'})
                                    submitted_at.append(time.time_ns())
                                evaluator = official.StringEvaluator()
                                cps = [checkpoints.Checkpoint(str(i), url, 'string', weight=.5) for i,url in enumerate(synthetic_urls)]
                                stale = [(p.url, evaluator.eval('', p, cps)[0]) for p in context.pages]
                                assert not evaluator.done and all(v[1]==0 for v in stale), 'fail-first stale URL seam not reproduced'
                                latch=FirstDoneGrade()
                                sampled_at=time.time_ns()
                                result=latch.sample(context,evaluator,cps)
                                assert evaluator.done and result==(score,wrong), (label,result)
                                # Frozen result cannot accrue a later submission or alter a zero.
                                ws = sockets[-1]
                                send('Runtime.evaluate', {'expression':'document.querySelector("textarea").value="https://shop.fixture/product/late"; document.querySelector("button").click()'})
                                assert latch.sample(context,evaluator,cps) is result
                                rows.append({'case':label,'staleUrlGate':True,'score':score,'done':evaluator.done,
                                    'observedFrontendTabs':sum(p.url==f'{base}/frontend' for p in context.pages),
                                    'submittedAtNs':submitted_at, 'submittedTextSha256':hashlib.sha256(text.encode()).hexdigest(),
                                    'sampledAtNs':sampled_at,'gradedAtNs':time.time_ns(),'firstDoneFrozen':True})
                            finally:
                                for ws in sockets: ws.close()
                        browser.close()
                finally:
                    if not isinstance(chrome.pid, int) or chrome.pid <= 1:
                        raise ValueError('MP-10 invalid owned Chrome cleanup PID')
                    chrome.terminate()
                    try: chrome.wait(timeout=10)
                    except subprocess.TimeoutExpired: chrome.kill(); chrome.wait(timeout=5)
                    assert chrome.poll() is not None
    finally:
        server.shutdown(); server.server_close(); thread.join(timeout=5)
    report={'mpItems':['MP-08','MP-10','MP-11'],'status':'PASS','scoredCampaign':False,
        'officialCommit':'1aaef63d737308144f13b32e37ab5dbc3688d070','officialHashes':PINS,'cases':rows,
        'cleanup':{'ownedChromeSettled':True,'profileRemoved':True,'loopbackServerClosed':True},
        'limits':'First-party synthetic submissions only; no historical scores repaired; no real shop readiness or MP acceptance.'}
    with args.output.open('x') as f: json.dump(report,f,indent=2); f.write('\n')
    print('MP-08 / MP-10 / MP-11 H6 PASS: unchanged evaluator, right/partial/wrong, multiple tabs; cleanup complete')

if __name__=='__main__': main()
