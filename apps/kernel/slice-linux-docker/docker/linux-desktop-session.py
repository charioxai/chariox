"""MP-08 / MP-11: retain detached GUI children inside an owned process tree."""
import ctypes
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


def valid_pid(pid):
    return isinstance(pid, int) and 1 < pid <= 2147483647


def identity(pid):
    if not valid_pid(pid):
        return None
    try:
        fields = Path('/proc/'+str(pid)+'/stat').read_text().rsplit(') ', 1)[1].split()
        return {'pid': pid, 'parent': int(fields[1]), 'started': fields[19]}
    except (OSError, ValueError, IndexError):
        return None


def signal_child(item, value):
    if not valid_pid(item.get('pid')):
        raise ValueError('invalid owned child PID')
    current = identity(item['pid'])
    if not current or current['parent'] != os.getpid() or current['started'] != item['started']:
        return False
    try:
        os.kill(item['pid'], value)
    except ProcessLookupError:
        return False
    return True


def children():
    values = Path('/proc/self/task/'+str(os.getpid())+'/children').read_text().split()
    return [item for value in values if (item := identity(int(value))) and item['parent'] == os.getpid()]


def reap(main_pid=None):
    code = None
    while True:
        try:
            pid, status = os.waitpid(-1, os.WNOHANG)
        except ChildProcessError:
            break
        if not pid:
            break
        if pid == main_pid:
            code = os.waitstatus_to_exitcode(status)
    return code


def run(command):
    stopping = False

    def stop(_signal, _frame):
        nonlocal stopping
        stopping = True

    for value in [signal.SIGTERM, signal.SIGINT]:
        signal.signal(value, stop)
    parent = os.getppid()
    libc = ctypes.CDLL(None, use_errno=True)
    libc.prctl.argtypes = [ctypes.c_int] + [ctypes.c_ulong] * 4
    for option, value in [(36, 1), (1, signal.SIGTERM)]:  # CHILD_SUBREAPER / PDEATHSIG
        if libc.prctl(option, value, 0, 0, 0):
            raise OSError(ctypes.get_errno())
    if os.getppid() != parent:
        return 128 + signal.SIGTERM
    # Forward only the existing readiness/CDP pipes; never inherit arbitrary FDs.
    descriptors = []
    for fd in [3, 4]:
        try:
            os.fstat(fd)
            descriptors.append(fd)
        except OSError:
            pass
    child = subprocess.Popen(command, pass_fds=descriptors)
    for fd in descriptors:
        os.close(fd)
    code = None
    try:
        while code is None and not stopping:
            code = reap(child.pid)
            time.sleep(.02)
    finally:
        deadline = time.monotonic() + 1
        while children():
            value = signal.SIGTERM if time.monotonic() < deadline else signal.SIGKILL
            for item in children():
                signal_child(item, value)
            reap(child.pid)
            time.sleep(.02)
    child.returncode = code if code is not None else -signal.SIGTERM
    return code if code is not None and code >= 0 else 128 - child.returncode


if __name__ == '__main__':
    try:
        sys.exit(run(sys.argv[1:]))
    except Exception as error:
        print('MP-08 / MP-11: desktop supervision '+type(error).__name__, file=sys.stderr)
        sys.exit(1)
