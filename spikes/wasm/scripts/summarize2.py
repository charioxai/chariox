"""Markdown tables from sweep-async.sh output: median [min-max] of 3 runs.
Usage: python3 summarize2.py <sweep2.jsonl> <artifacts-dir>"""
import json
import os
import statistics
import sys
from collections import defaultdict

rows = [json.loads(line) for line in open(sys.argv[1]) if line.strip()]
artifacts = sys.argv[2]
VARIANTS = ["wasm-rust-async", "wasm-qjs", "wasm-py", "wasm-rust", "wasm-js", "node-js"]
NAMES = {
    "wasm-rust-async": "Rust (async)", "wasm-qjs": "JS QuickJS (async)", "wasm-py": "Python (async)",
    "wasm-rust": "Rust (r1 sync)", "wasm-js": "JS StarlingMonkey (r1 sync)", "node-js": "Node 24.20 (today)",
}


def cells(predicate):
    found = defaultdict(list)
    for row in rows:
        if row["r"] and predicate(row["cell"], row["r"]):
            found[row["r"]["variant"]].append(row["r"])
    return found


def stat(values, digits=1):
    values = [v for v in values if isinstance(v, (int, float))]
    if not values:
        return "n/a"
    fmt = f"{{:.{digits}f}}"
    return f"{fmt.format(statistics.median(values))} [{fmt.format(min(values))}–{fmt.format(max(values))}]"


def table(title, variants, metrics, found):
    print(f"\n### {title}\n")
    print("| metric | " + " | ".join(NAMES[v] for v in variants) + " |")
    print("|---|" + "---|" * len(variants))
    print("| runs | " + " | ".join(str(len(found.get(v, []))) for v in variants) + " |")
    for label, get, digits in metrics:
        out = []
        for v in variants:
            values = []
            for r in found.get(v, []):
                try:
                    values.append(get(r))
                except (KeyError, TypeError, IndexError):
                    pass
            if values and all(isinstance(x, str) for x in values):
                out.append(" / ".join(sorted(set(values))))
            else:
                out.append(stat(values, digits))
        print(f"| {label} | " + " | ".join(out) + " |")


mib = lambda kb: kb / 1024
base = cells(lambda c, r: c["args"].endswith("bwrap 1") and c["entry"] == "app.cwasm")
bare = cells(lambda c, r: c["args"].endswith("none 1"))
for v, found in bare.items():
    for r, b in zip(base.get(v, []), found):
        r["bareColdMs"] = b["coldReadyMs"]
jit = cells(lambda c, r: c["args"].endswith("bwrap 1") and c["entry"] == "app.wasm")
for v, found in jit.items():
    for r, j in zip(base.get(v, []), found):
        r["jitColdMs"] = j["coldReadyMs"]


def hog(kind):
    def get(r):
        h = r[f"hog_{kind}"]
        o = h["outcome"]
        outcome = (o.get("code") or f"allocated {o.get('allocatedMb')} MiB (not stopped)") if isinstance(o, dict) else str(o)
        return f"{outcome}, worker {'alive' if h['workerAliveAfter'] else 'gone'}"
    return get


def spin_how(r):
    s = r["spin"]
    return s["stoppedBy"]


table("r1 cells (bubblewrap, AOT unless noted)", VARIANTS, [
    ("cold start to ready, ms", lambda r: r["coldReadyMs"], 1),
    ("cold start without bubblewrap, ms", lambda r: r["bareColdMs"], 1),
    ("cold start, compile at start (no AOT), ms", lambda r: r["jitColdMs"], 1),
    ("warm instantiate, ms", lambda r: r["warmInstantiate"]["medianMs"], 3),
    ("RSS after ready, MiB", lambda r: mib(r["rssAfterReady"]["rssKb"]), 1),
    ("private (anon) RSS after ready, MiB", lambda r: mib(r["rssAfterReady"]["anonKb"]), 1),
    ("RSS after 1,000 calls + CPU tools, MiB", lambda r: mib(r["rssAfterWork"]["rssKb"]), 1),
    ("tool call p50, ms", lambda r: r["echo"]["p50"], 3),
    ("tool call p95, ms", lambda r: r["echo"]["p95"], 3),
    ("tool call p99, ms", lambda r: r["echo"]["p99"], 3),
    ("JSON transform (10k records x5), ms", lambda r: r["transform"]["medianMs"], 1),
    ("JS-heavy compute (1M Map/string ops), ms", lambda r: r["compute"]["medianMs"], 1),
    ("runaway loop: error after, ms", lambda r: r["spin"]["errorAfterMs"], 0),
    ("runaway loop: next call answered after, ms", lambda r: r["spin"].get("nextCallMs", r["spin"].get("killToReadyAgainMs")), 1),
    ("runaway loop: how", spin_how, 0),
    ("256 MiB byte buffers (limit 128)", hog("bytes"), 0),
    ("256 MiB byte buffers: peak RSS, MiB", lambda r: mib(r["hog_bytes"]["memory"]["hwmKb"]), 0),
    ("256 MiB small objects (limit 128)", hog("objects"), 0),
    ("256 MiB small objects: peak RSS, MiB", lambda r: mib(r["hog_objects"]["memory"]["hwmKb"]), 0),
    ("state + broker HTTP correct", lambda r: str(r["counter"][2].get("count") == 3 and r["fetch"].get("status") == 200), 0),
], base)

print("\n### Artifact sizes and install-time precompile\n")
print("| App | .wasm KiB | .cwasm KiB |")
print("|---|---|---|")
for v in VARIANTS[:5]:
    d = os.path.join(artifacts, v)
    sizes = [os.path.getsize(os.path.join(d, f)) // 1024 if os.path.exists(os.path.join(d, f)) else "n/a" for f in ("app.wasm", "app.cwasm")]
    print(f"| {NAMES[v]} | {sizes[0]} | {sizes[1]} |")

ASYNC = ["wasm-rust-async", "wasm-qjs", "wasm-py"]
for calls, wait in ((16, 0), (64, 0), (64, 50)):
    found = cells(lambda c, r, calls=calls, wait=wait: r.get("mode") == "shared" and r["calls"] == calls and r.get("waitMs", 0) == wait)
    what = "50 ms broker wait" if wait == 0 else "50 ms timer wait (WASI 0.3 wait-for; Node setTimeout), no broker call"
    table(f"Many agents, one instance: {calls} concurrent callers, shared counter/cache/connection, {what}", ASYNC + ["node-js"], [
        ("calls ok", lambda r: r["ok"], 0),
        ("all done in, ms", lambda r: r["totalMs"], 1),
        ("per-call latency p50, ms", lambda r: r["latency"]["p50"], 1),
        ("per-call latency p95, ms", lambda r: r["latency"]["p95"], 1),
        ("no lost updates / one connection", lambda r: f"{r['noLostUpdates']} / {r['oneConnection']}", 0),
        ("first error", lambda r: (r["errors"][0]["code"] if r["errors"] else "none"), 0),
        ("RSS after, MiB", lambda r: mib(r["memory"]["rssKb"]), 1),
    ], found)

for work in (0, 20):
    found = cells(lambda c, r, work=work: r.get("mode") == "stream" and r["work"] == work)
    table(f"Streaming a 50 MiB body through the broker ({'fast reader' if work == 0 else 'slow reader: 20 hash rounds per byte'})", ASYNC + (["node-js"] if work == 0 else []), [
        ("result", lambda r: r["result"].get("error", {}).get("code") if "error" in r["result"] else f"{r['result']['bytes'] >> 20} MiB in {r['result']['chunks']} chunks", 0),
        ("time, ms", lambda r: r["ms"], 0),
        ("throughput, MiB/s", lambda r: r["mbPerS"], 0),
        ("worker RSS before → peak, MiB", lambda r: f"{mib(r['memoryBefore']['rssKb']):.1f} → {mib(r['memoryAfter']['hwmKb']):.1f}", 0),
        ("max chunks waiting for the guest (cap 4)", lambda r: r["stats"]["chunksBufferedMax"] if r["stats"] else "n/a (SDK Fetch)", 0),
        ("broker streams left open", lambda r: r["brokerStreamsOpen"], 0),
    ], found)

for work in (0, 200000):
    found = cells(lambda c, r, work=work: r.get("mode") == "subscribe" and r["work"] == work)
    table(f"Subscription: kernel delivers events one by one ({'fast reader' if work == 0 else 'slow reader: 200k ops per event'})", ASYNC, [
        ("received / in order", lambda r: f"{r['result'].get('received')} / {r['result'].get('inOrder')}", 0),
        ("events per second", lambda r: r["eventsPerS"], 0),
        ("delivery ack p50, ms", lambda r: r["ack"]["p50"], 3),
        ("delivery ack p95, ms", lambda r: r["ack"]["p95"], 3),
        ("RSS after, MiB", lambda r: mib(r["memory"]["rssKb"]), 1),
    ], found)

found = cells(lambda c, r: r.get("mode") == "cancel")
table("Cancellation: cancel a waiting call and a streaming call while 8 others run", ASYNC, [
    ("other calls ok", lambda r: f"{r['othersOk']}/{r['othersTotal']}", 0),
    ("all calls answered and broker streams released after, ms", lambda r: r["allReleasedMs"], 1),
    ("broker streams still open right after", lambda r: r["brokerStreamsOpen"], 0),
    ("every broker stream released after, ms", lambda r: r["releasedAfterMs"], 0),
    ("next call answered after, ms", lambda r: r["nextCallMs"], 2),
], found)

for culprit in ("spin", "bytes", "objects"):
    found = cells(lambda c, r, culprit=culprit: r.get("mode") == "blast" and r["culprit"] == culprit if isinstance(r.get("culprit"), str) else False)
    found = cells(lambda c, r, culprit=culprit: r.get("mode") == "blast" and c["args"].endswith(culprit))
    title = {"spin": "runaway loop", "bytes": "256 MiB byte buffers", "objects": "256 MiB small objects"}[culprit]
    table(f"One bad call among 9 in flight ({title}; limit 128 MiB, CPU slice 1 s)", ASYNC, [
        ("bad call gets", lambda r: r["culprit"]["outcome"]["error"]["code"] if "error" in r["culprit"]["outcome"] else "ok", 0),
        ("bad call answered after, ms", lambda r: r["culprit"]["ms"], 0),
        ("the 8 other calls get", lambda r: " ".join(f"{n}x {o}" for o, n in sorted(__import__('collections').Counter(x["outcome"] for x in r["others"]).items())), 0),
        ("shared counter after (1 = in-memory state lost)", lambda r: r["sharedCounterAfter"], 0),
        ("worker RSS after, MiB", lambda r: mib(r["memory"]["rssKb"]), 1),
    ], found)

found = cells(lambda c, r: r.get("mode") == "hol")
table("One CPU-heavy call (3M-iteration compute) and an echo on the same instance", ASYNC + ["node-js"], [
    ("compute, ms", lambda r: r["computeMs"], 0),
    ("first echo during compute, ms", lambda r: r["echoDuringComputeMs"][0], 0),
    ("echo after compute, ms", lambda r: r["echoDuringComputeMs"][1], 2),
], found)

loads = [row["cell"]["load1"] for row in rows]
print(f"\nLoad average during the sweep: {stat(loads)}; cells: {len(rows)}; empty cells: {sum(1 for row in rows if not row['r'])}")
