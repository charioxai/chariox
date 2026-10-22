"""Spike Python App on the async world (componentize-py). One instance
serves every call at once; module globals are shared by all callers."""
import json

from componentize_py_types import Err
from wit_world import exports
from wit_world.imports import events, http, log, registry, state
from wit_world.imports.types import AppError

TOOLS = ["echo", "counter", "fetch", "transform", "compute", "spin", "hog", "shared", "stream", "subscribe", "wait"]

# Shared by every concurrent caller: a counter, a cache and one "connection".
SHARED = {"counter": 0, "cache": {}, "connection": None}


def fail(code, message):
    return Err(AppError(code=code, message=message, retryable=None))


def request(url, connection=None):
    return http.Request(url=url, method="GET", headers=[("accept", "text/plain")], body=None,
                        connection_id=connection, operation_id=None)


async def counter():
    current = await state.get("count")
    count = json.loads(current.value) if current else 0
    committed = await state.transact(state.Transaction(
        schema_version=0,
        checks=[state.Check(key="count", version=current.version if current else None)],
        writes=[state.Write_Put(("count", json.dumps(count + 1)))],
    ))
    return {"count": count + 1, "revision": committed.revision}


async def fetch(url):
    response = await http.send(request(url))
    body = response.body
    return {"status": response.head.status, "bytes": len(body), "head": body[:16].decode()}


async def shared(url):
    SHARED["counter"] += 1
    n = SHARED["counter"]
    if SHARED["connection"] is None:
        SHARED["connection"] = {"id": f"conn-{n}", "requests": 0}
    connection = SHARED["connection"]
    connection["requests"] += 1
    sequence = connection["requests"]
    response = await http.send(request(url, connection["id"]))
    SHARED["cache"][n] = len(response.body)
    return {"n": n, "sequence": sequence, "connection": connection["id"], "cached": len(SHARED["cache"])}


async def stream(url, work):
    head, body = await http.fetch(request(url))
    total, chunks, digest = 0, 0, 0
    with body:
        while not body.writer_dropped:
            chunk = await body.read(65536)
            total += len(chunk)
            chunks += 1 if chunk else 0
            for _ in range(work):
                for byte in chunk:
                    digest = (digest * 31 + byte) & 0xFFFFFFFF
    return {"status": head.status, "bytes": total, "chunks": chunks, "hash": digest}


async def subscribe(name, count, work):
    subscription = await events.subscribe(name)
    received, in_order, last, digest = 0, True, 0, 0
    with subscription:
        while received < count and not subscription.writer_dropped:
            for event in await subscription.read(1):
                seq = json.loads(event.data)["seq"]
                in_order = in_order and seq == last + 1
                last = seq
                received += 1
                for i in range(work):
                    digest = (digest * 31 + i) & 0xFFFFFFFF
    return {"received": received, "inOrder": in_order, "last": last, "hash": digest}


def transform(n, rounds):
    records = [{"id": i, "name": f"item-{i}", "score": (i * 7919) % 100, "tags": ["a", "b", "c"]} for i in range(n)]
    text = json.dumps(records, separators=(",", ":"))
    total, out_bytes = 0, 0
    for _ in range(rounds):
        picked = []
        for record in json.loads(text):
            if record["score"] > 50:
                total += record["score"]
                picked.append({"id": record["id"], "name": record["name"].upper()})
        out_bytes = len(json.dumps(picked, separators=(",", ":")))
    return {"sum": total, "inBytes": len(text), "outBytes": out_bytes}


def compute(n):
    h, counts = 0, {}
    for i in range(n):
        key = "k" + str(i % 1000)
        counts[key] = counts.get(key, 0) + 1
        h = (h * 31 + i) & 0xFFFFFFFF
    if h >= 1 << 31:
        h -= 1 << 32
    return {"h": h, "keys": len(counts)}


def spin():
    while True:
        pass


def hog(mb, kind):
    if kind == "objects":
        objects = ["x" * 64 + str(i) for i in range(mb * 10000)]
        return {"allocatedMb": mb, "objects": len(objects)}
    chunks = [bytearray([i % 251]) * (1 << 20) for i in range(mb)]
    return {"allocatedMb": len(chunks)}


async def call(name, value):
    if name == "echo":
        return value
    if name == "counter":
        return await counter()
    if name in ("fetch", "wait"):
        return await fetch(value.get("url", ""))
    if name == "transform":
        return transform(value.get("n", 1000), value.get("rounds", 1))
    if name == "compute":
        return compute(value.get("n", 1000000))
    if name == "spin":
        return spin()
    if name == "hog":
        return hog(value.get("mb", 64), value.get("kind", "bytes"))
    if name == "shared":
        return await shared(value.get("url", ""))
    if name == "stream":
        return await stream(value.get("url", ""), value.get("work", 0))
    if name == "subscribe":
        return await subscribe(value.get("name", "tick"), value.get("count", 1), value.get("work", 0))
    raise fail("TOOL_NOT_FOUND", "unknown tool")


class Handlers(exports.Handlers):
    def register(self) -> None:
        for name in TOOLS:
            registry.tool(name)
        log.write(log.Level.INFO, "registered", json.dumps({"tools": len(TOOLS)}))

    async def call_tool(self, name, input, context, cancel) -> str:
        # componentize-py 0.25.1 cannot cancel a pending future read (the
        # instance traps with "resource has children"), so this App does not
        # race `cancel`; it closes it and runs to completion.
        with cancel:
            pass
        try:
            return json.dumps(await call(name, json.loads(input)), separators=(",", ":"))
        except Err:
            raise
        except MemoryError:
            raise fail("APP_MEMORY_LIMIT", "out of memory")
        except Exception as error:
            raise fail("HANDLER_FAILED", str(error)[:4096])
