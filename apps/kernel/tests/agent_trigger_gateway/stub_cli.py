#!/usr/bin/env python3
# Stands in for `chariox-cli`: `serve prepare` succeeds, and `serve agent-trigger`
# makes the credential read the real gateway makes (consumeKernelLocalAuthTokenFromEnv,
# then a kernel handshake with it) before it answers on its port.
import base64, os, socket, stat, sys, threading, urllib.parse

args = sys.argv[1:]
if args[:2] == ["serve", "prepare"]:
    sys.exit(0)
if args[:2] != ["serve", "agent-trigger"]:
    sys.exit(2)
port = int(args[4])
kernel = urllib.parse.urlparse(args[args.index("--kernel-url") + 1])
# The real client binds a credential to an exact, path-free loopback endpoint
# (canonicalLoopbackKernelEndpoint in packages/kernel-client/src/ipc.ts).
if (
    kernel.scheme != "ws"
    or kernel.hostname not in ("127.0.0.1", "[::1]")
    or kernel.path not in ("", "/")
    or kernel.username
    or kernel.password
    or kernel.query
    or kernel.fragment
):
    sys.stderr.write("the kernel credential needs a canonical loopback endpoint\n")
    sys.exit(14)

path = os.environ.get("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE")
if not path:
    sys.stderr.write("no kernel credential reached the gateway\n")
    sys.exit(11)
if os.stat(path).st_mode & 0o077:
    sys.stderr.write("the kernel credential file is not private\n")
    sys.exit(12)
with open(path) as handle:
    token = handle.read().strip()
os.unlink(path)

kernel_socket = socket.create_connection((kernel.hostname, kernel.port))
kernel_socket.sendall(
    (
        f"GET / HTTP/1.1\r\nHost: {kernel.hostname}:{kernel.port}\r\n"
        "Upgrade: websocket\r\nConnection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {base64.b64encode(os.urandom(16)).decode()}\r\n"
        f"Sec-WebSocket-Version: 13\r\nAuthorization: Bearer {token}\r\n\r\n"
    ).encode()
)
if b" 101 " not in kernel_socket.recv(4096).split(b"\r\n")[0]:
    sys.stderr.write("the kernel refused the gateway credential\n")
    sys.exit(13)


def serve():
    listener = socket.socket()
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(("127.0.0.1", port))
    listener.listen()
    while True:
        client, _ = listener.accept()
        request_path = client.recv(4096).split(b" ")[1].decode()
        ingress = request_path.split("/")[1]
        body = ingress.encode()
        client.sendall(
            b"HTTP/1.1 200 OK\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s"
            % (len(body), body)
        )
        client.close()


threading.Thread(target=serve, daemon=True).start()
sys.stdin.buffer.read()
