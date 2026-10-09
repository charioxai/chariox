"""MP-08/MP-11: desktop protection oracle for the native desktop capture.
One persistent process per desktop source. Each request line is answered by one
complete AT-SPI/X11 snapshot: the masks to apply, or none (mask everything),
plus a digest of the whole snapshot. The kernel binds a readback only between
two equal snapshots taken before and after it. Never reads pixels.
"""
import hashlib
import importlib.util
import json
import sys
from pathlib import Path


def scope(request):
    if set(request) - {'values'} != {'processes', 'browser_processes', 'browser_protection'}:
        raise ValueError('desktop protection request')
    # Owner 2026-10-09: registered Vault values, masked best effort by text box.
    values = request.get('values', [])
    if not isinstance(values, list) or len(values) > 256 or not all(isinstance(v, str) and 0 < len(v) <= 4096 for v in values):
        raise ValueError('desktop protection values')
    for field in ('processes', 'browser_processes'):
        if not isinstance(request[field], list) or len(request[field]) > 256:
            raise ValueError('desktop process scope')
    if request['browser_protection'] is not None and not isinstance(request['browser_protection'], dict):
        raise ValueError('desktop browser protection')
    return request


def answer(request, snapshot):
    tree = snapshot(request['processes'], request['browser_processes'], request['browser_protection'], request.get('values', []))
    digest = hashlib.sha256(json.dumps(tree, sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode()).hexdigest()
    usable = tree.get('available') and tree.get('complete') and not tree.get('protected')
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
        sys.stdout.write(json.dumps(answer(scope(json.loads(line)), accessibility.snapshot)) + '\n')
        sys.stdout.flush()


if __name__ == '__main__':
    try:
        main()
    except Exception:
        sys.stderr.write('MD-DISPLAY: desktop protection stage request\n')
        sys.exit(1)
