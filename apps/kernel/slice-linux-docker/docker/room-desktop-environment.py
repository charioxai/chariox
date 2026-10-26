"""MP-08/MP-10/MP-11: execute on the existing private Room desktop bus."""
import os
from pathlib import Path
import re
import stat
import sys
import time


def desktop_environment():
    private = os.environ.get('CHARIOX_SLICE_PRIVATE_ROOT')
    runtime = Path(private) / 'runtime' if private else Path(os.environ.get('CHARIOX_SLICE_ROOT', '/opt/chariox-slice')) / 'private'
    fd = os.open(runtime / 'desktop-session-address', os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        metadata = os.fstat(fd)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid() or metadata.st_mode & 0o077 or metadata.st_size > 512:
            raise ValueError('private desktop binding required')
        address = os.read(fd, 513).decode('utf-8')
        if not re.fullmatch(r'unix:(?:path|abstract)=[^\x00-\x20]{1,480}', address):
            raise ValueError('invalid desktop binding')
    finally:
        os.close(fd)
    return {**os.environ, 'DBUS_SESSION_BUS_ADDRESS': address, 'NO_AT_BRIDGE': '0',
            'GTK_A11Y': 'always', 'ACCESSIBILITY_ENABLED': '1'}


if __name__ == '__main__':
    try:
        if len(sys.argv) < 2:
            raise ValueError('desktop command required')
        deadline = time.monotonic() + 5
        while True:
            try:
                environment = desktop_environment()
                break
            except FileNotFoundError:
                # Openbox publishes its bus during supervised desktop startup.
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.05)
        os.execvpe(sys.argv[1], sys.argv[1:], environment)
    except Exception:
        # Never log command arguments, the bus address, or input contents.
        sys.stderr.write('MP-11: Room desktop environment unavailable\n')
        sys.exit(1)
