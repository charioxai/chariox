#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11: one real TUI -> disposable kernel -> official provider turn.

The runtime bundle and product-linked account are explicit inputs. This runner
never copies credentials, enrolls accounts or uses a provider SDK. Accounting
integration is currently blocked on the coordinator protocol allocation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import shutil
import shlex
import signal
import socket
import struct
import subprocess
import tempfile
import termios
import time
import fcntl

from contract import safe_pid, completed_answer, quota_exhausted, admit_placement, leased_target


def file_hash(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def preflight(runtime_root, source_commit, kernel_sha256, local_protocol):
    root = Path(runtime_root)
    if not root.is_absolute() or not re.fullmatch(r'[0-9a-f]{40}', source_commit):
        raise ValueError('MP-11: absolute runtime and exact source commit required')
    if not re.fullmatch(r'[0-9a-f]{64}', kernel_sha256):
        raise ValueError('MP-11: exact kernel SHA256 required')
    kernel = root / 'bin/chariox-kernel'
    if not kernel.resolve().is_relative_to(root.resolve()):
        raise ValueError('MP-11: kernel artifact escapes bundle')
    if file_hash(kernel) != kernel_sha256:
        raise ValueError('MP-11: kernel artifact digest mismatch')
    actual = subprocess.check_output([str(kernel), '--print-local-daemon-protocol-version'], text=True).strip()
    if actual != str(local_protocol):
        raise ValueError('MP-11: kernel protocol mismatch')
    manifest = json.loads((root / 'eval-runtime.json').read_text())
    if manifest.get('source_commit') != source_commit or manifest.get('kernel_sha256') != kernel_sha256:
        raise ValueError('MP-11: runtime source provenance mismatch')
    # No unmanifested files (including credentials) may hitchhike in a bundle.
    actual_files = {str(p.relative_to(root)) for p in root.rglob('*') if p.is_file()}
    if actual_files != set(manifest['files']) | {'eval-runtime.json'}:
        raise ValueError('MP-11: unmanifested runtime bundle file')
    for p in root.rglob('*'):
        if p.is_symlink() and not p.resolve().is_relative_to(root.resolve()):
            raise ValueError('MP-11: runtime symlink escapes bundle')
    # Manifest hashes bind the real app entry and all shipped runtime files.
    for relative, digest in manifest['files'].items():
        p = root / relative
        if not p.resolve().is_relative_to(root.resolve()) or not p.is_file():
            raise ValueError('MP-11: runtime file escapes bundle')
        if file_hash(p) != digest:
            raise ValueError('MP-11: runtime file digest mismatch')
    required = ['bin/chariox-kernel', 'bin/bun', 'apps/cli/dist/index.js', 'packages/kernel-client/dist/ipc.js']
    if any(p not in manifest['files'] for p in required):
        raise ValueError('MP-11: incomplete real-client runtime manifest')
    return root


class Automation:
    def __init__(self, path):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(30)
        self.socket.connect(str(path))
        self.file = self.socket.makefile('rb')
        self.sequence = 0

    def send(self, action, **fields):
        self.sequence += 1
        self.socket.sendall((json.dumps({'id': self.sequence, 'action': action, **fields}) + '\n').encode())
        response = json.loads(self.file.readline())
        if response.get('id') != self.sequence or not response.get('ok'):
            # Do not emit provider response bodies/errors into the harness output.
            raise RuntimeError('MP-08 / MP-10: TUI action rejected')
        return response.get('data')

    def close(self):
        self.file.close()
        self.socket.close()


def stop_owned(process):
    if process.poll() is None:
        safe_pid(process.pid)
        process.send_signal(signal.SIGTERM)
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            # Positive child PID only. No process group or name-based signal.
            os.kill(safe_pid(process.pid), signal.SIGKILL)
            process.wait(timeout=5)


def wait_until(predicate, deadline, tick=None):
    while time.monotonic() < deadline:
        if tick:
            tick()
        value = predicate()
        if value:
            return value
        time.sleep(0.2)
    raise TimeoutError('MP-08 / MP-10: runtime operation timed out')


def process_identity(pid):
    safe_pid(pid)
    try:
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(') ', 1)[1].split()
        return fields[19] if fields[0] != 'Z' else None
    except (OSError, IndexError):
        return None


def descendant_identities(pid):
    found = {}
    try:
        children = Path(f'/proc/{safe_pid(pid)}/task/{pid}/children').read_text().split()
    except OSError:
        return found
    for child in children:
        child = safe_pid(int(child))
        identity = process_identity(child)
        if identity is not None:
            found[child] = identity
            found.update(descendant_identities(child))
    return found


def signal_owned_descendant(pid, born, sig):
    safe_pid(pid)
    try:
        handle = os.pidfd_open(pid)
    except ProcessLookupError:
        return
    try:
        if process_identity(pid) == born:
            try:
                signal.pidfd_send_signal(handle, sig)
            except ProcessLookupError:
                pass
    finally:
        os.close(handle)


def resource_sample():
    fields = dict(line.split(':', 1) for line in Path('/proc/meminfo').read_text().splitlines())
    available = int(fields['MemAvailable'].split()[0]) * 1024
    disk = shutil.disk_usage('/').free
    if available < 9 * 1024**3 or disk < 10 * 1024**3:
        raise RuntimeError('MP-11: resource floor reached; settling only this run')
    return {'mem_available_bytes': available, 'root_free_bytes': disk}


def run(request, evidence):
    root = preflight(request['runtime_root'], request['source_commit'], request['kernel_sha256'], request['local_protocol'])
    placement = admit_placement(request)
    profile = request.get('profile_path')
    provider = request.get('provider', 'codex')
    if provider not in ['codex', 'claude', 'opencode']:
        raise ValueError('MP-08 / MP-10: official provider harness required')
    resources_before = resource_sample()
    evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
    state = Path(tempfile.mkdtemp(prefix='chariox-evals-'))
    (state / 'home').mkdir(mode=0o700)
    env = {k: os.environ[k] for k in ['PATH', 'LANG', 'LC_ALL'] if k in os.environ}
    env['PATH'] = str(root / 'bin') + ':' + env.get('PATH', '/usr/bin:/bin')
    env.update(HOME=str(state / 'home'), CHARIOX_HOME=str(state / 'kernel'),
               CHARIOX_LOG_DIR=str(state / 'logs'), TERM='xterm-256color',
               CHARIOX_PROVIDER_PROCESS_ORPHAN_TTL_MS=str(2**64 - 1))
    reservations = []
    for key in (['CHARIOX_KERNEL_PORT', 'CHARIOX_MCP_PORT', 'CHARIOX_CODEX_PORT', 'CHARIOX_OPENCODE_PORT'] if placement == 'local' else []):
        s = socket.socket(); s.bind(('127.0.0.1', 0)); reservations.append(s)
        env[key] = str(s.getsockname()[1])
    endpoint = 'ws://127.0.0.1:' + env['CHARIOX_KERNEL_PORT'] if placement == 'local' else request['home_kernel_url']
    if placement == 'leased' and request.get('home_auth_file'):
        # Locator only; the real product client consumes its runtime access grant.
        env['CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE'] = request['home_auth_file']
    processes, connections, descriptors, captures = [], [], [], []
    descendants = {}
    start = time.monotonic()
    measurement = {'mp_items': ['MP-08', 'MP-10', 'MP-11'], 'source_commit': request['source_commit'],
                   'placement': placement, 'kernel_sha256': request['kernel_sha256'], 'status': 'failed', 'usage': None,
                   'resources_before': resources_before,
                   'usage_unavailable_reason': 'kernel per-turn accounting awaits coordinator protocol allocation'}

    def tick():
        resource_sample()
        for process in processes:
            if process.poll() is None:
                descendants.update(descendant_identities(process.pid))

    def wait_owned(predicate, deadline):
        return wait_until(predicate, deadline, tick=tick)

    def launch_cli(extra, suffix):
        automation = state / (suffix + '.sock')
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 160, 0, 0))
        descriptors.append(master)
        capture = (evidence / (suffix + '.terminal.log')).open('wb'); captures.append(capture)
        cli = subprocess.Popen([str(root / 'bin/bun'), str(root / 'apps/cli/dist/index.js'),
                                '--kernel-url', endpoint, '--automation-socket', str(automation), *extra],
                               env=env, stdin=slave, stdout=slave, stderr=slave)
        os.close(slave); processes.append(cli)
        # Drain the PTY continuously so real rendering cannot block the client.
        import threading
        def drain():
            try:
                while data := os.read(master, 65536):
                    capture.write(data); capture.flush()
            except OSError:
                pass
        thread = threading.Thread(target=drain, daemon=True); thread.start()
        wait_owned(lambda: automation.exists(), time.monotonic() + 30)
        connection = Automation(automation); connections.append(connection)
        return connection

    try:
        for s in reservations: s.close()
        if placement == 'local':
            kernel_log = (evidence / 'kernel.log').open('wb'); captures.append(kernel_log)
            kernel = subprocess.Popen([str(root / 'bin/chariox-kernel')], env=env,
                                      stdout=kernel_log, stderr=kernel_log)
            processes.append(kernel)
            setup = launch_cli(['--detached'], 'setup')
            setup.send('wait_for', daemonDisconnected=False, timeoutMs=30000)
            setup.send('submit_prompt', prompt=f'/provider accounts link {provider} Evals {shlex.quote(profile)}')
            status_command = [str(root / 'bin/bun'), str(Path(__file__).with_name('product_status.mjs')), str(root), endpoint, 'Evals', provider]
            def status():
                result = subprocess.run(status_command, env=env, capture_output=True, text=True, timeout=30)
                return json.loads(result.stdout) if result.returncode == 0 else None
            account = wait_owned(status, time.monotonic() + 45)
            if account['auth_state'] != 'authenticated':
                raise RuntimeError('MP-08 / MP-10: linked account needs owner login')
            if quota_exhausted(account['meters'], int(time.time() * 1000)):
                measurement['status'] = 'quota_exhausted'
                raise RuntimeError('MP-08 / MP-10: proven rolling plan quota exhaustion')
            setup.send('exit')
            client = launch_cli(['--create-session', '--workspace', request['workspace'], '--provider', provider,
                                 '--model', request['model'], '--account-profile', account['profile_id']], 'task')
            snap = wait_owned(lambda: client.send('snapshot'), time.monotonic() + 30)
            snap = wait_owned(lambda: (s if (s := client.send('snapshot')).get('session', {}).get('agents') else None), time.monotonic() + 60)
            session_id = snap['session']['id']; agent_id = snap['session']['focusedAgentId']
        else:
            client = launch_cli(['--session', request['home_session_ref']], 'task')
            snap = wait_owned(lambda: (s if (s := client.send('snapshot')).get('session', {}).get('agents') else None), time.monotonic() + 60)
            target = leased_target(snap, request)
            client.send('submit_prompt', prompt='/agent focus ' + target['id'])
            snap = wait_owned(lambda: (s if (s := client.send('snapshot')).get('session', {}).get('focusedAgentId') == target['id'] else None), time.monotonic() + 30)
            session_id = snap['session']['id']; agent_id = target['id']
            measurement['worker_kernel_id'] = request['worker_kernel_id']
            measurement['worker_lifecycle_owner'] = 'coordinator'
        # Normal kernel-owned permission policy, scoped to this benchmark agent.
        client.send('submit_prompt', prompt='/agent permissions ' + agent_id + ' yolo')
        snap = client.send('snapshot')
        prior_prompt_ids = {e.get('promptId') for e in snap.get('transcript', {}).get('entries', []) if e.get('promptId')}
        client.send('submit_prompt', prompt=request['instruction'])
        deadline = time.monotonic() + request.get('timeout_seconds', 1800)
        def submitted():
            s = client.send('snapshot')
            prompts = [e for e in s.get('transcript', {}).get('entries', []) if e.get('role') == 'user' and e.get('promptId') and e['promptId'] not in prior_prompt_ids]
            return prompts[-1]['promptId'] if prompts else None
        prompt_id = wait_owned(submitted, min(deadline, time.monotonic() + 60))
        measurement.update(session_id=session_id, agent_id=agent_id, prompt_id=prompt_id)
        def settled():
            s = client.send('snapshot')
            answer = completed_answer(s, session_id, agent_id, prompt_id)
            if answer is not None:
                safe_snapshot = {'session_id': session_id, 'agent_id': agent_id, 'prompt_id': prompt_id,
                                 'transcript': [e for e in s.get('transcript', {}).get('entries', []) if e.get('promptId') == prompt_id]}
                (evidence / 'final-snapshot.json').write_text(json.dumps(safe_snapshot, indent=2))
                return {'answer': answer}
            return None
        final = wait_owned(settled, deadline)
        measurement.update(status='completed', answer=final['answer'])
        client.send('exit')
    except Exception as error:
        measurement['failure_kind'] = type(error).__name__
    finally:
        for process in processes:
            if process.poll() is None:
                descendants.update(descendant_identities(process.pid))
        for connection in connections:
            connection.close()
        for process in reversed(processes):
            stop_owned(process)
        # Identity-bound signals to descendants observed under our own children.
        # Never signal a group, unrelated PID, PID 0/1/-1 or a reused PID.
        remaining = {pid: born for pid, born in descendants.items() if process_identity(pid) == born}
        for pid, born in remaining.items():
            if process_identity(pid) == born:
                signal_owned_descendant(pid, born, signal.SIGTERM)
        until = time.monotonic() + 5
        while remaining and time.monotonic() < until:
            remaining = {pid: born for pid, born in remaining.items() if process_identity(pid) == born}
            time.sleep(0.1)
        for pid, born in remaining.items():
            if process_identity(pid) == born:
                signal_owned_descendant(pid, born, signal.SIGKILL)
        for fd in descriptors:
            os.close(fd)
        for capture in captures:
            capture.close()
        measurement['wall_time_seconds'] = time.monotonic() - start
        # Never delete a runtime directory containing a provider credential.
        protected = any(p.name in {'auth.json', '.credentials.json'} for p in state.rglob('*'))
        if protected:
            measurement['retained_state'] = str(state)
            measurement['cleanup_complete'] = False
        else:
            shutil.rmtree(state)
            measurement['cleanup_complete'] = not any(process_identity(pid) == born for pid, born in descendants.items())
    return measurement


def main():
    parser = argparse.ArgumentParser(description='MP-08 / MP-10 / MP-11: real Chariox task turn')
    parser.add_argument('--input', type=Path)
    parser.add_argument('--result', type=Path)
    parser.add_argument('--preflight', action='store_true')
    parser.add_argument('--runtime-root')
    parser.add_argument('--source-commit')
    parser.add_argument('--kernel-sha256')
    parser.add_argument('--local-protocol', type=int)
    args = parser.parse_args()
    if args.preflight:
        preflight(args.runtime_root, args.source_commit, args.kernel_sha256, args.local_protocol)
        return 0
    if not args.input or not args.result:
        parser.error('--input and --result required')
    request = json.loads(args.input.read_text())
    result = run(request, args.result.with_suffix('.evidence'))
    with args.result.open('x') as out:
        json.dump(result, out, indent=2); out.write('\n')
    return 0 if result['status'] == 'completed' and result['cleanup_complete'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
