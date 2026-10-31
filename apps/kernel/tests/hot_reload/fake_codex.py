# A Codex app-server shaped like the real one (codex-cli 0.159.3) where a hot
# reload meets it: a running turn keeps running whether or not a client is
# connected, `thread/resume` on a new connection rejoins it, and what was
# streamed while no client was connected is not replayed. The turn streams one
# item of 450 parts, one every 20 ms. The test writes `swap` into the record
# when it starts the reload and `go` once it is connected again: part 300 waits
# for `go`, so the item still streams after the swap. With
# CHARIOX_FAKE_CODEX_FINISH_IN_GAP the turn ends as soon as `swap` exists.
# A JSON-RPC body the test writes as `mcp-<name>` is sent to the kernel's tools
# as this agent, as the real Codex calls them; the answer goes to `mcp-<name>.out`.
import base64, hashlib, json, os, socket, struct, sys, threading, time, urllib.request

record = os.environ.get("CHARIOX_FAKE_CODEX_RECORD", "/dev/null/none")
finish_in_gap = os.environ.get("CHARIOX_FAKE_CODEX_FINISH_IN_GAP") == "1"
THREAD, TURN, ITEM = "thread-1", "turn-1", "msg-1"
lock = threading.Condition()
subscriber = None
turn = {"status": None, "text": ""}
tools = {}


def note(name, line):
    with open(os.path.join(record, name), "a") as f:
        f.write(line + "\n")


def send(conn, message):
    data = json.dumps(message).encode()
    header = bytes([0x81])
    if len(data) < 126:
        header += bytes([len(data)])
    elif len(data) < 65536:
        header += bytes([126]) + struct.pack(">H", len(data))
    else:
        header += bytes([127]) + struct.pack(">Q", len(data))
    try:
        conn.sendall(header + data)
    except OSError:
        pass


def notify(method, params):
    with lock:
        conn = subscriber
    if conn is not None:
        send(conn, {"jsonrpc": "2.0", "method": method, "params": params})


def read_exact(conn, n):
    data = b""
    while len(data) < n:
        chunk = conn.recv(n - len(data))
        if not chunk:
            raise EOFError
        data += chunk
    return data


def frames(conn):
    while True:
        b0, b1 = read_exact(conn, 2)
        length = b1 & 0x7F
        if length == 126:
            length = struct.unpack(">H", read_exact(conn, 2))[0]
        elif length == 127:
            length = struct.unpack(">Q", read_exact(conn, 8))[0]
        mask = read_exact(conn, 4) if b1 & 0x80 else b"\0\0\0\0"
        payload = bytes(c ^ mask[i % 4] for i, c in enumerate(read_exact(conn, length)))
        opcode = b0 & 0x0F
        if opcode == 8:
            return
        if opcode == 9:
            conn.sendall(bytes([0x8A, len(payload)]) + payload)
        elif opcode == 1:
            yield json.loads(payload)


def turn_view():
    items = [{"type": "userMessage", "id": "user-1", "content": [{"type": "text", "text": "prompt"}]}]
    if turn["status"] == "completed":
        items.append({"type": "agentMessage", "id": ITEM, "text": turn["text"]})
    return {"id": TURN, "status": turn["status"], "items": items}


PARTS = 450


def stream(n):
    delta = f"turn-1-part-{n} \n"
    turn["text"] += delta
    notify("item/agentMessage/delta", {"threadId": THREAD, "turnId": TURN, "itemId": ITEM, "delta": delta})
    note("parts", f"part {n}")


def wait_for(name):
    while not os.path.exists(os.path.join(record, name)):
        time.sleep(0.01)


def run_turn():
    notify("turn/started", {"threadId": THREAD, "turn": {"id": TURN, "status": "inProgress", "items": []}})
    notify("item/started", {"threadId": THREAD, "turnId": TURN, "item": {"type": "agentMessage", "id": ITEM, "text": ""}})
    for n in range(1, PARTS + 1):
        swapping = os.path.exists(os.path.join(record, "swap"))
        if finish_in_gap and swapping:
            stream(n)
            continue
        if n == 300:
            if finish_in_gap:
                wait_for("swap")
            else:
                wait_for("go")
        stream(n)
        time.sleep(0.02)
    notify("item/completed", {"threadId": THREAD, "turnId": TURN, "item": {"type": "agentMessage", "id": ITEM, "text": turn["text"]}})
    turn["status"] = "completed"
    notify("turn/completed", {"threadId": THREAD, "turn": {"id": TURN, "status": "completed", "items": []}})
    note("parts", "result 1")


def answer(conn, request):
    global subscriber
    method, params = request.get("method"), request.get("params") or {}
    result = {}
    if method == "initialize":
        result = {"userAgent": "fake-codex/0.159.3"}
    elif method == "account/read":
        result = {"account": {"type": "apiKey"}, "requiresOpenaiAuth": False}
    elif method == "model/list":
        result = {"data": [{"id": "gpt-6-luna", "model": "gpt-6-luna", "displayName": "gpt-6-luna", "isDefault": True,
                            "supportedReasoningEfforts": [{"reasoningEffort": "low", "description": "low"}],
                            "defaultReasoningEffort": "low"}], "nextCursor": None}
    elif method in ("thread/start", "thread/resume"):
        note("threads", f"{method} {os.getpid()}")
        config = params.get("config") or {}
        if "mcp_servers.chariox.url" in config:
            with lock:
                tools["url"] = config["mcp_servers.chariox.url"]
                tools["authorization"] = config["mcp_servers.chariox.http_headers.Authorization"]
                lock.notify_all()
        with lock:
            subscriber = conn
            lock.notify_all()
        status = {"type": "active", "activeFlags": []} if turn["status"] == "inProgress" else {"type": "idle"}
        turns = [turn_view()] if turn["status"] else []
        result = {"thread": {"id": THREAD, "status": status, "turns": turns}, "model": "gpt-6-luna", "cwd": params.get("cwd")}
    elif method == "turn/start":
        note("prompts", "prompt 1")
        turn["status"] = "inProgress"
        result = {"turn": {"id": TURN, "status": "inProgress", "items": []}}
        threading.Thread(target=run_turn, daemon=True).start()
    elif method == "thread/turns/list":
        result = {"data": [turn_view()] if turn["status"] else [], "nextCursor": None}
    elif method == "thread/read":
        result = {"thread": {"id": THREAD, "turns": [turn_view()] if turn["status"] else []}}
    if "id" in request:
        send(conn, {"jsonrpc": "2.0", "id": request["id"], "result": result})


def serve(conn):
    global subscriber
    request = b""
    while b"\r\n\r\n" not in request:
        chunk = conn.recv(4096)
        if not chunk:
            return
        request += chunk
    key = [l.split(b":", 1)[1].strip() for l in request.split(b"\r\n") if l.lower().startswith(b"sec-websocket-key:")]
    if not key:
        # A health probe: plain HTTP.
        conn.sendall(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
        conn.close()
        return
    accept = base64.b64encode(hashlib.sha1(key[0] + b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest())
    conn.sendall(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: " + accept + b"\r\n\r\n")
    try:
        for message in frames(conn):
            answer(conn, message)
    except (EOFError, OSError, ValueError):
        pass
    with lock:
        if subscriber is conn:
            subscriber = None
    conn.close()


def call_tools():
    done = set()
    while True:
        time.sleep(0.02)
        for name in sorted(os.listdir(record)):
            if not name.startswith("mcp-") or name.endswith((".out", ".tmp")) or name in done:
                continue
            with lock:
                lock.wait_for(lambda: "url" in tools)
                url, authorization = tools["url"], tools["authorization"]
            done.add(name)
            with open(os.path.join(record, name), "rb") as f:
                body = f.read()
            request = urllib.request.Request(url, data=body, method="POST", headers={
                "authorization": authorization, "content-type": "application/json",
                "accept": "application/json, text/event-stream"})
            try:
                with urllib.request.urlopen(request, timeout=30) as response:
                    answer = response.read().decode()
            except Exception as error:
                answer = json.dumps({"fake_codex_error": str(error)})
            # A streamed answer carries the JSON-RPC message on its data line.
            lines = [line[5:].strip() for line in answer.splitlines() if line.startswith("data:")]
            with open(os.path.join(record, name + ".tmp"), "w") as f:
                f.write(lines[-1] if lines else answer)
            os.rename(os.path.join(record, name + ".tmp"), os.path.join(record, name + ".out"))


def main():
    args = sys.argv[1:]
    if "--version" in args:
        print("codex-cli 0.159.3")
        return
    if "app-server" not in args or "--listen" not in args:
        return
    address = args[args.index("--listen") + 1].split("://", 1)[1]
    host, port = address.rsplit(":", 1)
    server = socket.socket()
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind((host, int(port)))
    server.listen(16)
    note("starts", f"app-server {os.getpid()}")
    threading.Thread(target=call_tools, daemon=True).start()
    while True:
        conn, _ = server.accept()
        threading.Thread(target=serve, args=(conn,), daemon=True).start()


main()
