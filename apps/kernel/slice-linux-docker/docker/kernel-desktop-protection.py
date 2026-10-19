"""MP-08/MP-11: desktop protection oracle for the native desktop capture.
One persistent process per desktop source. Each request line is answered by one
field-only AT-SPI/CDP snapshot: the masks to apply, or none (retry),
plus a digest of the whole snapshot. The kernel binds a readback only between
two equal snapshots taken before and after it. Never reads pixels.
"""
import hashlib
import importlib.util
import json
import sys
import time
from pathlib import Path


def scope(request):
    if set(request) - {'values', 'fill'} != {'processes', 'browser_processes', 'browser_protection'}:
        raise ValueError('desktop protection request')
    # Owner 2026-10-09: only live Vault-filled plain fields contribute masks.
    values = request.get('values', [])
    if not isinstance(values, list) or len(values) > 256 or not all(isinstance(v, str) and 0 < len(v) <= 4096 for v in values):
        raise ValueError('desktop protection values')
    if not isinstance(request.get('fill', False), bool):
        raise ValueError('desktop fill flag')
    for field in ('processes', 'browser_processes'):
        if not isinstance(request[field], list) or len(request[field]) > 256:
            raise ValueError('desktop process scope')
    if request['browser_protection'] is not None and not isinstance(request['browser_protection'], dict):
        raise ValueError('desktop browser protection')
    return request


UNPLACED_GRACE = 0.4  # seconds an unplaced filled window waits for its field mask before it is covered whole
unplaced_since = None


def answer(request, snapshot):
    global unplaced_since
    try:
        tree = snapshot(request['processes'], request['browser_processes'], request['browser_protection'], request.get('values', []), request.get('fill', False))
    except ValueError:
        # A filled window that cannot be placed yet is uncertain, not fatal: no masks, so the kernel withholds the frame and asks again.
        tree = {'available': False, 'complete': False}
    # A window whose filled field cannot be placed is covered whole, after a short grace for the placement to settle.
    if tree.get('unplaced'):
        unplaced_since = unplaced_since if unplaced_since is not None else time.monotonic()
        if time.monotonic() - unplaced_since < UNPLACED_GRACE:
            tree = {'available': False, 'complete': False}
    else:
        unplaced_since = None
    digest = hashlib.sha256(json.dumps(tree, sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode()).hexdigest()
    # Native password widgets already paint dots. Preserve the desktop and
    # apply only the filled-field rectangles supplied by the snapshot.
    usable = tree.get('available') and tree.get('complete')
    # Fixed-label diagnostics only: why masks are absent, never page data.
    state = [int(bool(tree.get(key))) for key in ('available', 'complete', 'protected')] + [int(tree.get('browser_withheld', 0) or 0)]
    return {'digest': digest, 'masks': tree.get('masks', tree.get('uncovered', [])) if usable else None, 'state': state}


def main():
    spec = importlib.util.spec_from_file_location('native_accessibility', Path(__file__).with_name('native-accessibility.py'))
    accessibility = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(accessibility)
    for line in sys.stdin.buffer:
        if len(line) > 1 << 20:
            raise ValueError('desktop protection request bound')
        sys.stdout.write(json.dumps(answer(scope(json.loads(line)), accessibility.capture_snapshot)) + '\n')
        sys.stdout.flush()


if __name__ == '__main__':
    try:
        main()
    except Exception:
        sys.stderr.write('MD-DISPLAY: desktop protection stage request\n')
        sys.exit(1)
