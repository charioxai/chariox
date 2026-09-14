#!/usr/bin/env python3
"""Offline Linux regression for the real managed broker namespace entrypoint.

Run as root with unshare, nsenter, mount and ip installed. All mounts and
network changes stay in disposable namespaces; no Docker daemon is required.
"""
import os
from pathlib import Path
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading


def checked(*args):
    subprocess.run(args, check=True, timeout=5, stdout=subprocess.DEVNULL)


def dns_server(root):
    checked("ip", "link", "set", "lo", "up")
    checked("ip", "addr", "add", "10.0.2.3/32", "dev", "lo")
    resolver = Path(root) / "private-resolv.conf"
    resolver.write_text("nameserver 10.0.2.3\noptions timeout:1 attempts:1\n")
    checked("mount", "--bind", str(resolver), os.path.realpath("/etc/resolv.conf"))
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as server:
        server.bind(("10.0.2.3", 53))
        print(os.getpid(), flush=True)
        while True:
            query, address = server.recvfrom(512)
            # One controlled A question. Preserve the question and return an
            # RFC 5737 address; the drill never contacts that address.
            response = query[:2] + struct.pack("!HHHHH", 0x8180, 1, 1, 0, 0)
            response += query[12:]
            response += b"\xc0\x0c" + struct.pack("!HHIH", 1, 1, 0, 4)
            response += socket.inet_aton("203.0.113.7")
            server.sendto(response, address)


def isolated_probe(helper, root):
    # Mask /run only inside this mount namespace, including the helper's fixed
    # PID path. Never write runtime state into the host's /run.
    checked("mount", "-t", "tmpfs", "-o", "size=1m", "tmpfs", "/run")
    state = Path("/run/chariox-docker/dockerd-rootless")
    state.mkdir(parents=True)
    resolver = Path(os.path.realpath("/etc/resolv.conf"))
    if not resolver.exists():
        resolver.parent.mkdir(parents=True, exist_ok=True)
        resolver.touch()
    server = subprocess.Popen(
        ["unshare", "--user", "--map-root-user", "--mount", "--net",
         sys.executable, __file__, "--dns", root],
        stdout=subprocess.PIPE, text=True,
    )
    try:
        import select
        assert select.select([server.stdout], [], [], 5)[0], "DNS fixture startup timed out"
        child_pid = server.stdout.readline().strip()
        assert child_pid.isdecimal(), "DNS fixture failed before readiness"
        (state / "child_pid").write_text(child_pid + "\n")
        # The bootstrap-to-broker connection uses a filesystem Unix socket.
        # It must remain accessible when only the broker joins private net.
        control_path = str(Path(root) / "control.sock")
        control = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        control.bind(control_path)
        control.listen(1)
        control.settimeout(5)

        def reply():
            with control:
                connection, _ = control.accept()
                with connection:
                    connection.sendall(b"broker-ready")

        responder = threading.Thread(target=reply, daemon=True)
        responder.start()
        probe = (
            "import os,socket; "
            "assert socket.gethostbyname('broker-dns.test') == '203.0.113.7'; "
            "assert os.readlink('/proc/self/ns/net') == "
            f"os.readlink('/proc/{child_pid}/ns/net'), 'broker retained host network with private DNS'; "
            "s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); s.settimeout(2); "
            f"s.connect({control_path!r}); assert s.recv(32)==b'broker-ready'; s.close(); "
            "print('private resolver reachable through production broker entrypoint')"
        )
        result = subprocess.run(["sh", helper, sys.executable, "-c", probe], timeout=15)
        assert result.returncode == 0, "managed broker DNS namespace regression"
        responder.join(timeout=5)
        assert not responder.is_alive(), "broker control socket probe did not finish"
        # The entrypoint must preserve command exit status and must not alter
        # this parent namespace as a side effect.
        result = subprocess.run(["sh", helper, "sh", "-c", "exit 17"], timeout=5)
        assert result.returncode == 17, "broker lost command exit status"
        assert os.readlink("/proc/self/ns/net") != os.readlink(f"/proc/{child_pid}/ns/net")
    finally:
        server.terminate()
        try:
            server.wait(timeout=3)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--dns":
        dns_server(sys.argv[2])
        return
    if len(sys.argv) == 4 and sys.argv[1] == "--isolated":
        isolated_probe(sys.argv[2], sys.argv[3])
        return
    assert sys.platform == "linux" and os.geteuid() == 0, "requires Linux root in a disposable test VM"
    assert len(sys.argv) == 2, "usage: managed-rootless-network-drill.py <production entrypoint>"
    helper = str(Path(sys.argv[1]).resolve(strict=True))
    with tempfile.TemporaryDirectory(prefix="chariox-rootless-dns-") as root:
        child = subprocess.Popen(
            ["unshare", "--mount", "--propagation", "private", sys.executable,
             __file__, "--isolated", helper, root], start_new_session=True,
        )
        try:
            result = child.wait(timeout=25)
        finally:
            # Includes descendants on failure, even if an intermediate process
            # exited. This group contains only this disposable drill.
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
        assert result == 0, "rootless DNS drill failed"
    print("rootless DNS namespace drill passed; fixture processes and temporary state removed")


if __name__ == "__main__":
    main()
