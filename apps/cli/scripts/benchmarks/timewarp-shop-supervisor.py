"""MP-08/MP-10/MP-11: stop exactly our Shop child on stdin EOF, positive PIDs only."""
import subprocess
import sys
import threading
import signal
from round2.owned_processes import OwnedProcessTree

def main():
    child = subprocess.Popen(sys.argv[1:], start_new_session=True)
    owned = OwnedProcessTree(child.pid)
    closed = threading.Event()
    def read_input():
        sys.stdin.buffer.read()
        closed.set()
    threading.Thread(target=read_input, daemon=True).start()
    try:
        while child.poll() is None and not closed.wait(0.1):
            pass
    finally:
        owned.signal(signal.SIGTERM)
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            owned.signal(signal.SIGKILL)
            child.wait(timeout=5)
    sys.exit(child.returncode if not closed.is_set() else 0)

if __name__ == '__main__':
    main()
