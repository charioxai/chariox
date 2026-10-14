#!/usr/bin/env python3
"""Unattended, file-bound real TextEdit caret comparison.

The granted helper only captures the runner-owned TextEdit window and clicks;
the independent AppKit reference runs in the unprivileged textedit-reference
tool, never inside the granted helper process."""
import importlib.util
import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time

spec = importlib.util.spec_from_file_location('fixture_drill', Path(__file__).with_name('text-click-drill.py'))
drill = importlib.util.module_from_spec(spec)
spec.loader.exec_module(drill)
PUBLIC_TEXT = '\n'.join(f'line {index:02d} public caret probe' for index in range(1, 13)) + '\n'
TEXTEDIT = '/System/Applications/TextEdit.app/Contents/MacOS/TextEdit'
REFERENCE = Path(os.environ.get('CUMAC_TEXTEDIT_REFERENCE', str(drill.BUILD / 'textedit-reference')))


def owned_helper(evidence, label, arguments, lease):
    try:
        receipt = drill.launch_helper(evidence, label, arguments)
        if 'refused:' in receipt:
            raise RuntimeError('TextEdit helper refused: ' + receipt.splitlines()[-1])
        return receipt
    except Exception:
        # Refusals and launch/identity failures cannot be undone by later desktop facts.
        if lease:
            lease.retired = True
        raise


def run_reference(frame, bounds):
    result = subprocess.run([str(REFERENCE), str(frame), *(str(bounds[k]) for k in ['X', 'Y', 'Width', 'Height'])],
                            capture_output=True, text=True, timeout=30)
    if result.returncode != 0:
        raise RuntimeError('TextEdit independent reference unavailable: ' + (result.stderr.strip() or 'no output').splitlines()[-1])
    return json.loads(result.stdout)


def reference(evidence, label, scope, lease):
    state = lease.require()
    bounds = next(w['bounds'] for w in state['windows'] if w['windowID'] == int(scope[4]))
    frame = evidence / f'{label}.png'
    receipt = owned_helper(evidence, label, scope + ['--capture', str(frame)], lease)
    if not re.search(rf'^frame \d+x\d+ window={scope[4]}$', receipt, re.MULTILINE):
        raise RuntimeError('TextEdit capture not proven')
    result = run_reference(frame, bounds)
    probes = result['probes']
    if (len(probes) != 2 or {p['name'] for p in probes} != {'line5-before-r', 'line5-after-r'}
            or any(p['expectedLocation'] != p['referenceLocation']
                   or p['expectedLocation'] != {'line5-before-r': 125, 'line5-after-r': 126}[p['name']]
                   for p in probes)):
        raise RuntimeError('TextEdit independent reference mismatch')
    (evidence / f'{label}.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def process_identity(pid):
    return subprocess.run(['ps', '-p', str(pid), '-o', 'lstart=,comm='], capture_output=True, text=True).stdout.strip()


def started_since(identity, launched):
    # `ps` lstart is local time in whole seconds, e.g. 'Wed Oct 14 11:24:07 2026'.
    _, month, day, clock, year = identity.split()[:5]
    hour, minute, second = map(int, clock.split(':'))
    months = 'Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec'.split()
    return time.mktime((int(year), months.index(month) + 1, int(day), hour, minute, second, 0, 0, -1)) >= int(launched)


def terminate_owned(pid, identity, window):
    # Only the runner's own unedited TextEdit instance, identified by start time and path.
    if process_identity(pid) != identity:
        raise RuntimeError('owned TextEdit identity changed; not signalled')
    os.kill(pid, signal.SIGTERM)
    deadline = time.monotonic() + 3
    while any(w['windowID'] == window for w in drill.desktop()['windows']) and time.monotonic() < deadline:
        time.sleep(0.05)
    if any(w['windowID'] == window for w in drill.desktop()['windows']):
        raise RuntimeError('TextEdit window remains visible')


def settled_bounds(pid, window):
    # A new TextEdit window animates into place; bind only stable geometry.
    last = None
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        drill.require_gui()
        bounds = [w['bounds'] for w in drill.desktop()['windows'] if w['pid'] == pid and w['windowID'] == window]
        if len(bounds) == 1 and bounds[0] == last:
            return last
        last = bounds[0] if len(bounds) == 1 else None
        time.sleep(0.5)
    raise RuntimeError('new TextEdit window geometry not stable; no mutation attempted')


def caret_matches(receipt, expected, wanted):
    # The helper reads AXSelectedTextRange back after setting it.
    observed = re.search(r'path=AXSelectedTextRange; observed AXSelectedTextRange location=(\d+) length=0; point=([\d.-]+),([\d.-]+)', receipt)
    return bool(observed and int(observed[1]) == expected
                and all(abs(float(observed[i + 2]) - wanted[key]) < 0.01
                        for i, key in enumerate(['x', 'y'])))


def run():
    drill.wait_for_gui()
    evidence = Path.home() / '.codex/evidence/browser-computer-use-macos' / time.strftime('ax-textedit-%Y%m%d-%H%M%S')
    evidence.mkdir(parents=True, exist_ok=False)
    document = evidence / 'caret-probe-r4.txt'
    scope = None
    pid = None
    identity = None
    saved = None
    warped = None
    lease = None
    code = 1
    verdict = 'FAIL: TextEdit completion not proven.'
    try:
        # Refuse before opening a window if this helper has not been re-granted.
        preflight = drill.launch_helper(evidence, 'preflight', ['--drill-permissions'])
        if 'refused: permission' in preflight:
            code = 2
            verdict = 'BLOCKED: helper Accessibility re-grant required; no TextEdit document opened.'
            return code
        if 'drillPermission accessibility=granted' not in preflight:
            raise RuntimeError('helper does not support public TextEdit diagnostic')
        before = drill.desktop()
        saved = before['pointer']
        existing = {w['windowID'] for w in before['windows']}
        owners = {w['pid'] for w in before['windows']}
        document.write_text(PUBLIC_TEXT)
        drill.require_gui()
        launched = time.time()
        # A new, runner-owned instance: an existing TextEdit and its documents stay untouched.
        subprocess.run(['open', '-n', '-a', '/System/Applications/TextEdit.app', str(document),
                        '--args', '-ApplePersistenceIgnoreState', 'YES'], check=True, timeout=5)
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            drill.require_gui()
            state = drill.desktop()
            windows = [w for w in state['windows'] if w['windowID'] not in existing and w['layer'] == 0
                       and w['pid'] == state['frontmostPID'] and w['pid'] not in owners
                       and w['bounds']['Width'] > 200 and w['bounds']['Height'] > 100]
            if len(windows) == 1:
                pid = windows[0]['pid']
                identity = process_identity(pid)
                if not identity.endswith(' ' + TEXTEDIT) or not started_since(identity, launched):
                    raise RuntimeError('new window is not a runner-owned TextEdit; no mutation attempted')
                scope = ['--enable-owner-window', '--owner-pid', str(pid), '--window-id', str(windows[0]['windowID']), '--target', 'focused']
                break
            time.sleep(0.1)
        else:
            raise RuntimeError('new TextEdit window not unambiguously frontmost; no mutation attempted')
        settled_bounds(pid, int(scope[4]))
        (evidence / 'desktop.json').write_text(json.dumps(drill.desktop(), indent=2) + '\n')
        lease = drill.PointerLease(drill.desktop, drill.run_pointer_script, saved)
        lease.bind(pid, int(scope[4]))
        initial = reference(evidence, 'reference', scope, lease)
        checks = []
        for at_pointer in [False, True]:
            for probe in initial['probes']:
                drill.require_gui()
                lease.require()
                point = probe['point']
                label = probe['name'] + ('-pointer' if at_pointer else '-explicit')
                if at_pointer:
                    lease.warp(point)
                    warped = lease.last
                    state = lease.require()
                    warped = state['pointer']
                    (evidence / f'{label}-warp.json').write_text(json.dumps(state, indent=2) + '\n')
                    if (not all(abs(warped[k] - point[k]) < 1 for k in ['x', 'y'])
                            or not all(abs(warped[k] - state['cgPointer'][k]) < 0.01 for k in ['x', 'y'])):
                        raise RuntimeError('pointer-warp-coordinate-mismatch')
                    arguments = ['--click-at-pointer', '--click']
                    wanted = warped
                else:
                    arguments = ['--click-at', str(point['x']), str(point['y']), '--click']
                    wanted = point
                receipt = owned_helper(evidence, label, scope + arguments, lease)
                drill.require_caret_receipt(receipt, lease)
                passed = (caret_matches(receipt, probe['expectedLocation'], wanted)
                          and lease.require()['frontmostPID'] == pid)
                checks.append(passed)
                (evidence / f'{label}.json').write_text(json.dumps({
                    'probe': probe, 'sampledPoint': wanted, 'receipt': receipt.splitlines()[-1], 'passed': passed}, indent=2) + '\n')
        # Layout must stay unchanged across all four comparisons.
        unchanged = reference(evidence, 'reference-after', scope, lease)['probes'] == initial['probes']
        code = 0 if len(checks) == 4 and all(checks) and unchanged else 1
        verdict = 'PASS: all four real TextEdit caret clicks agree with the independent AppKit reference.' if code == 0 else verdict
    except Exception as error:
        code = 1
        verdict = f'FAIL: {error}'
    finally:
        if lease and warped is not None:
            lease.restore()
        if scope and lease and lease.retired:
            code = 1
            verdict = f'FAIL: pointer lease retired; {verdict}'
            try:
                # Public geometry only: which ownership fact changed.
                now = drill.desktop()
                (evidence / 'lease-retired.json').write_text(json.dumps({
                    'frontmostPID': now['frontmostPID'], 'pid': pid, 'pointer': now['pointer'], 'last': lease.last,
                    'bounds': lease.bounds, 'window': [w['bounds'] for w in now['windows'] if w['windowID'] == int(scope[4])]},
                    indent=2) + '\n')
            except Exception:
                pass
        if scope:
            # Ending the runner's own unedited instance is not a desktop mutation.
            try:
                terminate_owned(pid, identity, int(scope[4]))
            except Exception as error:
                code = 1
                verdict = f'FAIL: cleanup incomplete: {error}; {verdict}'
        (evidence / 'RESULT.txt').write_text(verdict + '\n')
        print(verdict, flush=True)
        print(f'Evidence: {evidence}', flush=True)
    return code


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--recheck-bundle', action='store_true', help='use the separately built helper and preserve the granted helper')
    if parser.parse_args().recheck_bundle:
        drill.HELPER = drill.BUILD / 'Chariox Computer Helper Recheck.app'
    raise SystemExit(run())
