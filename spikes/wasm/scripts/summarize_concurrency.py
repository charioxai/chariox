"""Concurrency cells → markdown table: median total time of 16 waiting calls."""
import collections, json, statistics, sys

label = {('wasm-rust', 1): 'wasmtime + Rust, 1 instance', ('wasm-rust', 16): 'wasmtime + Rust, pool of 16',
         ('wasm-js', 1): 'wasmtime + JS, 1 instance', ('wasm-js', 16): 'wasmtime + JS, pool of 16',
         ('node-jco', None): 'Node 24.20 + jco Rust', ('node-js', None): 'Node 24.20 plain JS (today)'}
groups = collections.defaultdict(list)
for line in open(sys.argv[1]):
    r = json.loads(line)
    groups[(r['variant'], r['pool'])].append(r)
print('| variant | 16 calls done in, ms [min–max] | calls ok | RSS after, MiB |')
print('|---|---|---|---|')
for key, name in label.items():
    rows = groups.get(key, [])
    if not rows:
        continue
    times = [r['totalMs'] for r in rows]
    rss = statistics.median(r['memory']['rssKb'] / 1024 for r in rows) if 'memory' in rows[0] else float('nan')
    print(f"| {name} | {statistics.median(times):.0f} [{min(times):.0f}–{max(times):.0f}] | {min(r['ok'] for r in rows)}/16 | {rss:.0f} |")
