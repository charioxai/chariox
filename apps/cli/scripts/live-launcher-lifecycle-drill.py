#!/usr/bin/env python3
"""MP-08/MP-11: real Rust launcher + Bun TUI lifecycle in controlling PTYs.
Run with explicit --launcher/--kernel binaries built from this checkout, --evidence
outside the repository, and TMPDIR in lane-owned disk scratch. No provider runs.
"""
import argparse
import ctypes
from datetime import datetime, timezone
import fcntl
import hashlib
import html
import json
import os
from pathlib import Path
import pty
import re
import select
import shutil
import signal
import socket
import struct
import sys
import subprocess
import tempfile
from types import SimpleNamespace
import termios
import time
import threading


ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "apps/kernel/slice-linux-docker"))
from owned_process_signals import OwnedProcesses


def processes():
    rows = subprocess.check_output(['ps', '-axo', 'pid=,ppid=,pgid='], text=True)
    return [tuple(map(int, row.split())) for row in rows.splitlines() if row.strip()]


def alive(pid):
    # Reap adopted children on Linux; never mistake a zombie for a live TUI.
    try:
        if os.waitpid(pid, os.WNOHANG)[0] == pid:
            return False
    except ChildProcessError:
        pass
    return any(row[0] == pid for row in processes())


def send(pid, sig, owned, guard):
    if not isinstance(pid, int) or pid <= 1 or pid not in owned:
        raise RuntimeError('refusing unowned or unsafe signal target')
    if alive(pid):
        guard.pid(owned[pid], sig)


def wait_for(predicate, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(.025)
    return False


def rpc(path, action):
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(2)
        client.connect(str(path))
        client.sendall((json.dumps({'id': 1, 'action': action}) + '\n').encode())
        data = b''
        while b'\n' not in data:
            chunk = client.recv(65536)
            if not chunk:
                raise RuntimeError('automation closed before response')
            data += chunk
        result = json.loads(data.split(b'\n')[0])
        if not result['ok']:
            raise RuntimeError(result.get('error'))
        return result


def ready(path):
    try:
        return rpc(path, 'ping')['ok']
    except (OSError, RuntimeError):
        return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ['launcher', 'kernel', 'evidence']:
        parser.add_argument('--' + option, required=True, type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    evidence = args.evidence.resolve()
    if evidence.is_relative_to(root):
        raise RuntimeError('evidence must be outside repository')
    evidence.mkdir(parents=True, exist_ok=False)
    scratch = Path(tempfile.mkdtemp(prefix='launcher-drill-'))
    if os.uname().sysname == 'Linux':
        # Adopt/reap our launcher grandchildren, without touching other lanes.
        if ctypes.CDLL(None).prctl(36, 1, 0, 0, 0) != 0:
            raise RuntimeError('cannot become child subreaper')
    owned = {}
    pid_signals = OwnedProcesses()
    group_signals = OwnedProcesses()
    children = []
    terminals = []
    buffers = {}
    readers = []
    reading_done = threading.Event()
    rows = []
    resources = []
    def sample(label):
        disk = os.statvfs(scratch)
        memory = None
        if Path('/proc/meminfo').exists():
            for line in Path('/proc/meminfo').read_text().splitlines():
                if line.startswith('MemAvailable:'):
                    memory = int(line.split()[1])
        resources.append(dict(label=label, date=datetime.now(timezone.utc).isoformat(),
            diskAvailableBytes=disk.f_bavail * disk.f_frsize, memAvailableKiB=memory))
    sample('before')
    env = dict(os.environ, CHARIOX_HOME=str(scratch / 'state'),
               CHARIOX_LOG_DIR=str(scratch / 'logs'), TERM='xterm-256color',
               NODE_OPTIONS='--max-old-space-size=4096')
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    env['CHARIOX_KERNEL_PORT'] = str(port)
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        env['CHARIOX_MCP_PORT'] = str(listener.getsockname()[1])
    def start(label, path):
        def socket_identity():
            try:
                info = path.lstat()
                return (info.st_dev, info.st_ino, os.readlink(path) if path.is_symlink() else None)
            except FileNotFoundError:
                return None
        previous_socket = socket_identity()
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 35, 120, 0, 0))
        def setup():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        proc = subprocess.Popen(['sh', '-c', '"$@"; sleep 3600', 'launcher-drill-shell', str(args.launcher.resolve()), '--kernel-url',
            f'ws://127.0.0.1:{port}/kernel', '--automation-socket', str(path)],
            cwd=root, env=env, stdin=slave, stdout=slave, stderr=slave,
            preexec_fn=setup)
        os.close(slave)
        owned[proc.pid] = pid_signals.record(proc.pid, fresh_launch=True, watch=True)
        children.append(proc)
        terminals.append((master, label))
        buffers[master] = bytearray()
        def drain():
            while not reading_done.is_set():
                if select.select([master], [], [], .05)[0]:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError:
                        break
                    if not chunk:
                        break
                    buffers[master].extend(chunk)
        reader = threading.Thread(target=drain, daemon=True)
        readers.append(reader)
        reader.start()
        if not wait_for(lambda: socket_identity() is not None and socket_identity() != previous_socket and ready(path)):
            raise RuntimeError(f'{label}: TUI automation did not start')
        def connected():
            if kernel.poll() is not None:
                raise RuntimeError('real kernel exited before TUI connected')
            snapshot = rpc(path, 'snapshot')['data']
            rows = snapshot.get('waitingRoom', {}).get('rows', [])
            return not snapshot.get('daemonDisconnected') and any(
                row.get('id') == 'launch-kernel' and 'loading' not in row.get('value', '').lower()
                for row in rows)
        if not wait_for(connected):
            snapshot = rpc(path, 'snapshot')['data']
            public_status = dict(daemonDisconnected=snapshot.get('daemonDisconnected'),
                rowIds=[row.get('id') for row in snapshot.get('waitingRoom', {}).get('rows', [])])
            (evidence / (label + '-connection-status.json')).write_text(json.dumps(public_status, indent=2))
            raise RuntimeError(f'{label}: TUI did not load local kernel inventory')
        launcher = [pid for pid, parent, _ in processes() if parent == proc.pid]
        if len(launcher) != 1:
            raise RuntimeError(f'{label}: expected one Rust launcher')
        launcher_pid = launcher[0]
        owned[launcher_pid] = pid_signals.record(launcher_pid, fresh_launch=True)
        tui = [pid for pid, parent, _ in processes() if parent == launcher_pid]
        if len(tui) != 1:
            raise RuntimeError(f'{label}: expected one owned Bun child, got {tui}')
        owned[tui[0]] = pid_signals.record(tui[0], fresh_launch=True)
        capture(master, label)
        def wait_launcher(timeout):
            if not wait_for(lambda: not alive(launcher_pid), timeout):
                raise RuntimeError('launcher did not exit')
        return SimpleNamespace(pid=launcher_pid, group=proc.pid,
            poll=lambda: None if alive(launcher_pid) else 0, wait=wait_launcher), tui[0]
    def capture(master, label):
        output = bytes(buffers[master])
        (evidence / (label + '.pty')).write_bytes(output)
        # A rendered capture of this real PTY, plus raw terminal bytes.
        plain = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07|\x1b\\)', '', output.decode(errors='replace'))
        text = '\n'.join(plain.splitlines()[-35:])
        lines = ''.join(f'<tspan x="15" dy="18">{html.escape(line[:160])}</tspan>' for line in text.splitlines())
        (evidence / (label + '.svg')).write_text('<svg xmlns="http://www.w3.org/2000/svg" width="1400" height="700"><rect width="100%" height="100%" fill="#101216"/><text x="15" y="20" fill="#eee" font-family="monospace" font-size="13">' + lines + '</text></svg>')
    def check(name, ok, **details):
        sample(name)
        rows.append(dict(mpItems=['MP-08', 'MP-11'], scenario=name,
                         status='PASS' if ok else 'FAIL', **details))
    try:
        with (evidence / 'kernel.log').open('wb') as log:
            kernel = subprocess.Popen([str(args.kernel.resolve())], cwd=root, env=env,
                                      stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        owned[kernel.pid] = pid_signals.record(kernel.pid, fresh_launch=True, watch=True)
        children.append(kernel)
        def kernel_listening():
            if kernel.poll() is not None:
                raise RuntimeError('real kernel exited during startup')
            try:
                with socket.create_connection(('127.0.0.1', port), timeout=.2):
                    return True
            except OSError:
                return False
        if not wait_for(kernel_listening):
            raise RuntimeError('real kernel did not listen')
        path = scratch / 'automation.sock'
        first, tui = start('sigterm', path)
        started = time.monotonic()
        send(first.pid, signal.SIGTERM, owned, pid_signals)
        gone = wait_for(lambda: first.poll() is not None and not alive(tui), 2)
        check('launcher-SIGTERM-child-settled', gone,
              elapsedMs=round((time.monotonic()-started)*1000), launcherPid=first.pid, childPid=tui)
        if alive(tui):
            send(tui, signal.SIGTERM, owned, pid_signals)
            wait_for(lambda: not alive(tui), 2)
        first, tui = start('sigkill-old', path)
        started = time.monotonic()
        send(first.pid, signal.SIGKILL, owned, pid_signals)
        parent_death_settled = wait_for(lambda: first.poll() is not None and not alive(tui), 2)
        check('launcher-SIGKILL-parent-death', parent_death_settled,
              elapsedMs=round((time.monotonic()-started)*1000), launcherPid=first.pid, childPid=tui)
        if alive(tui):
            send(tui, signal.SIGTERM, owned, pid_signals)
            wait_for(lambda: not alive(tui), 2)
        replacement, new_tui = start('sigkill-replacement', path)
        # This checks availability after the old child settled. The deterministic
        # automation-server test covers cleanup racing a live replacement socket.
        check('replacement-socket-after-settled-old-exit', parent_death_settled and ready(path), childPid=new_tui)
        os.write(terminals[-1][0], b'\x03')
        check('normal-quit', wait_for(lambda: replacement.poll() is not None and not alive(new_tui), 2))
        interrupt, tui = start('launcher-interrupt', path)
        send(interrupt.pid, signal.SIGINT, owned, pid_signals)
        check('launcher-SIGINT', wait_for(lambda: interrupt.poll() is not None and not alive(tui), 2))
        hup, tui = start('terminal-hup', path)
        hup_launch = group_signals.record(hup.group, expected_start=owned[hup.group].start)
        members = [pid for pid, _, group in processes() if group == hup.group]
        if hup.group <= 1 or not members:
            raise RuntimeError('refusing group signal without exclusive ownership')
        # The guard checks every current member by ancestry and start identity,
        # including short-lived watcher probes created since start().
        group_signals.group(hup_launch, signal.SIGHUP)
        check('terminal-SIGHUP', wait_for(lambda: hup.poll() is not None and not alive(tui), 2))
    finally:
        pid_signals.refresh()
        for pid in list(pid_signals.owned):
            if pid not in owned:
                owned[pid] = pid_signals.record(pid, fresh_launch=True)
        for pid in list(owned):
            send(pid, signal.SIGTERM, owned, pid_signals)
        time.sleep(.2)
        for pid in list(owned):
            send(pid, signal.SIGKILL, owned, pid_signals)
        for proc in children:
            proc.wait(timeout=3)
        for pid in list(owned):
            wait_for(lambda: not alive(pid), 2)
        reading_done.set()
        for reader in readers:
            reader.join(timeout=1)
        for master, label in terminals:
            capture(master, label + '-end')
            os.close(master)
        check('cleanup-owned-processes', not any(alive(pid) for pid in owned))
        pid_signals.close()
        group_signals.close()
        shutil.rmtree(scratch)
        result = dict(mpItems=['MP-08', 'MP-11'], date=datetime.now(timezone.utc).isoformat(),
            platform=os.uname().sysname, resources=resources, head=subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
            launcherSha256=hashlib.sha256(args.launcher.read_bytes()).hexdigest(),
            kernelSha256=hashlib.sha256(args.kernel.read_bytes()).hexdigest(),
            command=os.sys.argv, rows=rows)
        (evidence / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(rows, indent=2))
    return 0 if rows and all(row['status'] == 'PASS' for row in rows) else 1


if __name__ == '__main__':
    raise SystemExit(main())
