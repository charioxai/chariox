"""MP-08 / MP-11: private Room binding for the shared native protection checks."""
import importlib.util
import os
from pathlib import Path
import re
import stat


class RoomInputDenied(ValueError):
    pass


def binding():
    # Same non-root slice UID and private bus binding as RoomNativeAccessibility.
    uid = os.getuid()
    if uid == 0 or not os.environ.get('CHARIOX_SLICE_ID'):
        raise ValueError('native Room slice binding unavailable')
    root = Path(os.environ['CHARIOX_SLICE_PRIVATE_ROOT']) / 'runtime' if os.environ.get('CHARIOX_SLICE_PRIVATE_ROOT') else Path(os.environ.get('CHARIOX_SLICE_ROOT', '/opt/chariox-slice')) / 'private'
    fd = os.open(root / 'desktop-session-address', os.O_RDONLY | os.O_NOFOLLOW)
    try:
        metadata = os.fstat(fd)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != uid or metadata.st_mode & 0o077 or metadata.st_size > 512:
            raise ValueError('native Room bus binding unavailable')
        address = os.read(fd, 513).decode('utf-8')
        if not re.fullmatch(r'unix:(?:path|abstract)=[^\x00-\x20]{1,480}', address):
            raise ValueError('invalid native Room bus binding')
    finally:
        os.close(fd)
    os.environ['DBUS_SESSION_BUS_ADDRESS'] = address
    processes = []
    for path in Path('/proc').iterdir():
        if not path.name.isdigit() or int(path.name) <= 1:
            continue
        try:
            if path.stat().st_uid != uid:
                continue
            text = (path / 'stat').read_text()
            processes.append({'pid': int(path.name), 'started': text[text.rindex(')') + 2:].split()[19]})
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    return processes


def accessibility():
    spec = importlib.util.spec_from_file_location('native_accessibility', Path(__file__).with_name('native-accessibility.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def snapshot():
    processes = binding()
    return accessibility().snapshot(processes)


def input_guard():
    try:
        processes = binding()
        return accessibility().input_guard(processes)
    except Exception as error:
        # A failed private binding/admission never becomes ordinary input.
        raise RoomInputDenied('native Room focus unavailable') from error
