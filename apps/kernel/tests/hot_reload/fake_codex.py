# A Codex app-server shaped like the real one (codex-cli 0.159.3) where a hot
# reload meets it: a running turn keeps running while no client is connected,
# `thread/resume` on a new connection rejoins it, and what was streamed
# meanwhile is not replayed. The turn streams 400 parts and keeps streaming
# while the kernel prepares the swap; once its client connection drops it
# streams 30 parts with no client (or, with CHARIOX_FAKE_CODEX_FINISH_IN_GAP,
# the rest of the turn), and continues once a connection resumes the thread.
import base64, hashlib, json, os, socket, struct, sys, threading, time

record = os.environ.get("CHARIOX_FAKE_CODEX_RECORD", "/dev/null/none")
finish_in_gap = os.environ.get("CHARIOX_FAKE_CODEX_FINISH_IN_GAP") == "1"
THREAD, TURN, ITEM = "thread-1", "turn-1", "msg-1"
lock = threading.Condition()
subscriber = None
dropped = False
turn = {"status": None, "text": ""}


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


def stream(n):
    delta = f"turn-1-part-{n} \n"
    turn["text"] += delta
    notify("item/agentMessage/delta", {"threadId": THREAD, "turnId": TURN, "itemId": ITEM, "delta": delta})
    note("parts", f"part {n}")
    time.sleep(0.005)


def run_turn():
    global dropped
    notify("turn/started", {"threadId": THREAD, "turn": {"id": TURN, "status": "inProgress", "items": []}})
    notify("item/started", {"threadId": THREAD, "turnId": TURN, "item": {"type": "agentMessage", "id": ITEM, "text": ""}})
    n = 0
    while not dropped:
        n += 1
        if n == 400:
            # Not dropped in time: hold the last part until it is.
            with lock:
                lock.wait_for(lambda: dropped)
        stream(n)
    gap = 400 if finish_in_gap else min(n + 30, 400)
    while n < gap:
        n += 1
        stream(n)
    if not finish_in_gap:
        with lock:
            lock.wait_for(lambda: subscriber is not None)
        while n < 400:
            n += 1
            stream(n)
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
    global subscriber, dropped
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
            dropped = True
            lock.notify_all()
    conn.close()


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
    while True:
        conn, _ = server.accept()
        threading.Thread(target=serve, args=(conn,), daemon=True).start()


main()
