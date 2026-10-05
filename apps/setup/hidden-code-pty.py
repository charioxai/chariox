#!/usr/bin/env python3
"""MP-11: real Setup hidden code prompt, with script stdin already exhausted."""
import fcntl
import os
import pty
import select
import signal
import subprocess
import sys
import termios
import time
import tempfile

binary = sys.argv[1]
master, slave = pty.openpty()

def session():
    os.setsid()
    fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

# Deliberately unsupported version causes local refusal immediately after code reading.
# No remote requests, credentials, files or provider processes are needed.
scratch = tempfile.TemporaryDirectory(prefix="setup-hidden-code-", dir=os.environ.get("CHARIOX_BYOM_TEST_STATE"))
child = subprocess.Popen([binary, '--enroll', '--release-version', 'invalid'],
                         stdin=subprocess.DEVNULL, stdout=slave, stderr=slave,
                         preexec_fn=session, close_fds=True, env={"HOME":scratch.name,"PATH":"/usr/bin:/bin"})
os.close(slave)
output = bytearray()
code = b'synthetic-hidden-single-use-code'
sent = False
try:
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if select.select([master], [], [], 0.1)[0]:
            try:
                output.extend(os.read(master, 4096))
            except OSError:
                break
            if not sent and b'Single-use enrollment code:' in output:
                os.write(master, code + b'\n')
                sent = True
        if child.poll() is not None:
            break
    assert sent, 'MP-11: Setup never opened the controlling-terminal prompt'
    assert code not in output, 'MP-11: code was echoed by the hidden prompt'
    assert child.wait(timeout=2) == 1, 'MP-07: invalid version must fail locally'
    print('MP-11: controlling-terminal enrollment code prompt never echoed its code')
finally:
    if child.poll() is None:
        if not isinstance(child.pid, int) or child.pid <= 1:
            raise RuntimeError('MP-11: unsafe fixture process signal refused')
        os.kill(child.pid, signal.SIGKILL)
        child.wait()
    os.close(master)
    scratch.cleanup()
