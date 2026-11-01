"""Summarize sweep.jsonl into markdown tables: median of runs [min-max]."""
import json, os, statistics, sys

rows = [json.loads(line) for line in open(sys.argv[1])]
artifacts = sys.argv[2]
order = ['wasm-rust', 'wasm-js', 'wasm-jsaot', 'node-jco', 'node-js']
label = {'wasm-rust': 'wasmtime + Rust', 'wasm-js': 'wasmtime + JS (componentize-js)',
         'wasm-jsaot': 'wasmtime + JS (componentize-js --aot)', 'node-jco': 'Node 24.20 + jco Rust (JSPI)',
         'node-js': 'Node 24.20 plain JS (today)'}

def cells(variant, sandbox, entry='aot'):
    return [r['r'] for r in rows if r['r'] and r['r']['variant'] == variant and r['r']['sandbox'] == sandbox and r['cell']['entry'] == entry]

def fmt(values, digits=1, unit=''):
    values = [v for v in values if v is not None]
    if not values:
        return 'n/a'
    med = statistics.median(values)
    if len(values) == 1:
        return f'{med:.{digits}f}{unit}'
    return f'{med:.{digits}f}{unit} [{min(values):.{digits}f}–{max(values):.{digits}f}]'

def outcome(cell, key):
    hog = cell[key]
    o = hog['outcome']
    if isinstance(o, dict) and 'code' in o:
        text = o['code']
    elif isinstance(o, dict):
        text = f"allocated {o.get('allocatedMb')} MiB (not stopped)"
    else:
        text = f"process died ({hog['exit']})"
    alive = 'worker alive' if hog['workerAliveAfter'] else 'worker gone'
    return f"{text}, {hog['ms']:.0f} ms, {alive}, peak RSS {hog['memory']['hwmKb'] // 1024} MiB" if hog['memory']['hwmKb'] else f"{text}, {hog['ms']:.0f} ms, {alive}"

def spin(cell):
    s = cell['spin']
    if s['stoppedBy'].startswith('worker'):
        return s['errorAfterMs'], s['nextCallMs'], 'epoch trap, same process'
    return None, s['killToReadyAgainMs'], f"SIGKILL at {s['killAtMs']:.0f} ms, reap {s['killToReapMs']:.0f} ms"

def size(variant):
    root = os.path.join(artifacts, variant)
    if variant.startswith('wasm'):
        return f"{os.path.getsize(root + '/app.wasm') / 1024:.0f} KiB .wasm, {os.path.getsize(root + '/app.cwasm') / 1024:.0f} KiB .cwasm"
    total = sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(root) for f in fs)
    return f'{total / 1024:.0f} KiB package (+ Node runtime)'

for sandbox in ['bwrap', 'none']:
    print(f'\n### sandbox: {sandbox} (median of runs [min–max])\n')
    print('| metric | ' + ' | '.join(label[v] for v in order) + ' |')
    print('|---' * (len(order) + 1) + '|')
    data = {v: cells(v, sandbox) for v in order}
    def line(name, getter, digits=1, unit=''):
        print(f'| {name} | ' + ' | '.join(fmt([getter(c) for c in data[v]], digits, unit) for v in order) + ' |')
    print('| runs | ' + ' | '.join(str(len(data[v])) for v in order) + ' |')
    line('cold start to ready, ms (AOT where wasm)', lambda c: c['coldReadyMs'])
    print('| cold start, compile at start (no AOT), ms | ' + ' | '.join(fmt([c['coldReadyMs'] for c in cells(v, sandbox, 'jit')]) if v.startswith('wasm') else 'n/a' for v in order) + ' |')
    line('warm instantiate, ms', lambda c: (c['warmInstantiate'] or {}).get('medianMs'), 3)
    line('RSS after ready, MiB', lambda c: c['rssAfterReady']['rssKb'] / 1024)
    line('private (anon) RSS after ready, MiB', lambda c: c['rssAfterReady']['anonKb'] / 1024)
    line('RSS after 1,000 calls + CPU tools, MiB', lambda c: c['rssAfterWork']['rssKb'] / 1024)
    line('tool call p50, ms', lambda c: c['echo']['p50'], 3)
    line('tool call p95, ms', lambda c: c['echo']['p95'], 3)
    line('tool call p99, ms', lambda c: c['echo']['p99'], 3)
    line('JSON transform (10k records x5), ms', lambda c: c['transform']['medianMs'])
    line('JS-heavy compute (1M Map/string ops), ms', lambda c: c['compute']['medianMs'])
    line('runaway loop: error after, ms', lambda c: spin(c)[0], 0)
    line('runaway loop: next call answered after, ms', lambda c: spin(c)[1], 1)
    print('| runaway loop: how | ' + ' | '.join((spin(data[v][0])[2] if data[v] else 'n/a') for v in order) + ' |')
    print('| 256 MiB byte buffers (limit 128) | ' + ' | '.join((outcome(data[v][0], 'hog_bytes') if data[v] else 'n/a') for v in order) + ' |')
    print('| 256 MiB small objects (limit 128) | ' + ' | '.join((outcome(data[v][0], 'hog_objects') if data[v] else 'n/a') for v in order) + ' |')
    print('| state + broker HTTP correct | ' + ' | '.join(('yes' if data[v] and all(c['counter'][2].get('count') == 3 and c['fetch'].get('status') == 200 and c['fetchDenied'].get('code') == 'APP_HTTP_DENIED' for c in data[v]) else 'NO') for v in order) + ' |')
    print('| load average during cells | ' + ' | '.join(fmt([r['cell']['load1'] for r in rows if r['r'] and r['r']['variant'] == v and r['r']['sandbox'] == sandbox and r['cell']['entry'] == 'aot'], 1) for v in order) + ' |')
print('\n### artifact size\n')
for v in order:
    print(f'- {label[v]}: {size(v)}')
