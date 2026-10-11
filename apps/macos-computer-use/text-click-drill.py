#!/usr/bin/env python3
"""Unattended build-bundle drills; honor the lane GUI_HOLD protocol."""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time
from fence_pointer import PointerLease, run_pointer_script

BUILD = Path(os.environ.get('CUMAC_BUILD_DIR', str(Path.home() / '.chariox/dev/cumac/build')))
HELPER = BUILD / 'Chariox Computer Helper.app'
LANE = Path('/Users/miguel/.chariox/dev/browser-computer-use/resume-20260930/agents/cumac')


GUI_DEADLINE = time.monotonic() + float(os.environ.get('CUMAC_GUI_WAIT_SECONDS', 20 * 60))


def gui_allowed():
    return (not (LANE / 'GUI_HOLD').exists() or (LANE / 'GUI_OK').exists()
            or time.monotonic() >= GUI_DEADLINE)


def require_gui():
    if not gui_allowed():
        raise RuntimeError(f'GUI_HOLD active in {LANE}; GUI run paused')


def wait_for_gui():
    while not gui_allowed():
        print('GUI_HOLD active; polling in 60 seconds.', flush=True)
        time.sleep(min(60, max(0, GUI_DEADLINE - time.monotonic())))


def desktop():
    # Public geometry only. Do not persist window titles or application text.
    script = '''ObjC.import('AppKit'); ObjC.import('CoreGraphics');
var app = $.NSWorkspace.sharedWorkspace.frontmostApplication;
var windows = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo(1, 0)));
var screens = [];
for (var i=0; i<Number($.NSScreen.screens.count); i++) {
 var s=$.NSScreen.screens.objectAtIndex(i);
 screens.push({frame:s.frame, scale:s.backingScaleFactor});
}
JSON.stringify({frontmostPID: Number(app.processIdentifier),
 windows: windows.map(function(w) {return {pid:w.kCGWindowOwnerPID,
 windowID:w.kCGWindowNumber, layer:w.kCGWindowLayer, bounds:w.kCGWindowBounds};}),
 screens: screens, cgPointer: $.CGEventGetLocation($.CGEventCreate(null)),
 cocoaPointer: $.NSEvent.mouseLocation, pointer: {x:$.NSEvent.mouseLocation.x,
 y:screens[0].frame.origin.y + screens[0].frame.size.height - $.NSEvent.mouseLocation.y}});'''
    result = subprocess.run(['osascript', '-l', 'JavaScript', '-e', script],
                            capture_output=True, text=True, check=True, timeout=3)
    return json.loads(result.stdout)


def read(evidence, name):
    path = evidence / name
    return path.read_text() if path.exists() else ''


def verify_launch_receipt(receipt, executable, cdhash):
    try:
        identity = next(json.loads(line) for line in receipt.splitlines() if line.startswith('{'))
        if (Path(identity['path']).resolve() != executable.resolve() or identity['cdhash'] != cdhash
                or identity['identifier'] != 'ai.chariox.computer-helper'
                or identity['signatureValid'] is not True or identity['parentPID'] != 1):
            raise ValueError('identity mismatch')
    except (ValueError, KeyError, StopIteration, TypeError):
        raise RuntimeError('helper-launch-identity-mismatch') from None


def launch_helper(evidence, label, arguments):
    require_gui()
    signing = subprocess.run(['codesign', '-dvvv', str(HELPER)], capture_output=True, text=True, check=True)
    match = re.search(r'^CDHash=([0-9a-f]{40})$', signing.stderr, re.MULTILINE)
    if not match: raise RuntimeError('helper-signature-unavailable')
    subprocess.run(['open', '-n', '-g', '-W', '--stdout', str(evidence / f'{label}.out'),
                    '--stderr', str(evidence / f'{label}.err'),
                    str(HELPER), '--args', *arguments],
                   check=True, timeout=8)
    receipt = read(evidence, f'{label}.out') + read(evidence, f'{label}.err')
    verify_launch_receipt(receipt, HELPER / 'Contents/MacOS/helper', match[1])
    print(receipt, end='')
    return receipt


def selection(evidence):
    rows = re.findall(r'fixture selection location=(\d+) length=(\d+)', read(evidence, 'fixture.out'))
    return rows[-1] if rows else None


def require_caret_receipt(receipt, lease=None):
    # Helper observations also permanently retire ownership, even if focus returns
    # before the runner can sample the desktop. Abort the enclosing probe loop.
    if 'refused:' in receipt:
        if lease:
            lease.retired = True
        raise RuntimeError('helper-caret-refused; no further mutation attempted')


def verify_caret(evidence, label, scope, pid, x, y, expected, at_pointer):
    require_gui()
    before = desktop()
    if before['frontmostPID'] != pid:
        raise RuntimeError('fixture-lost-frontmost; no helper mutation attempted')
    requested = {'x': float(x), 'y': float(y)}
    saved_pointer = before['pointer']
    lease = None
    warped = None
    comparison = None
    try:
        if at_pointer:
            lease = PointerLease(desktop, run_pointer_script, saved_pointer)
            lease.bind(pid, int(scope[scope.index('--window-id') + 1]))
            lease.warp(requested)
            warped = lease.last
            warped_state = lease.require()
            (evidence / f'{label}-warp.json').write_text(json.dumps(warped_state, indent=2) + '\n')
            warped = warped_state['pointer']
            # CGWarpMouseCursorPosition can quantize fractional screen points
            # to whole points. Allow less than one point per axis, but require
            # CG and converted AppKit samples to agree. The helper receipt must
            # still match the sampled point below, not the fractional request.
            if (not all(abs(warped[k] - requested[k]) < 1 for k in ['x', 'y'])
                    or not all(abs(warped_state['cgPointer'][k] - warped[k]) < 0.01
                               for k in ['x', 'y'])):
                raise RuntimeError('pointer-warp-coordinate-mismatch')
            arguments = ['--click-at-pointer', '--click']
        else:
            arguments = ['--click-at', x, y, '--click']
        if lease:
            lease.require()
        elif desktop()['frontmostPID'] != pid:
            raise RuntimeError('fixture-lost-frontmost; no helper mutation attempted')
        try:
            receipt = launch_helper(evidence, label, scope + ['--target', 'text', *arguments])
            require_caret_receipt(receipt, lease)
        except Exception:
            # A failed or unverified launch cannot authorize pointer restoration,
            # even when desktop facts have returned before cleanup.
            if lease:
                lease.retired = True
            raise
        deadline = time.monotonic() + 1
        while time.monotonic() < deadline and selection(evidence) != (expected, '0'):
            time.sleep(0.05)
        observed = re.search(r'path=AXSelectedTextRange; observed AXSelectedTextRange location=(\d+) length=0; point=([\d.-]+),([\d.-]+)', receipt)
        wanted = warped if at_pointer else requested
        point_matches = (observed and max(abs(float(observed[i + 2]) - wanted[k])
                                        for i, k in enumerate(['x', 'y'])) < 0.01)
        after = lease.require() if lease else desktop()
        passed = bool(observed and observed[1] == expected and point_matches
                      and selection(evidence) == (expected, '0') and after['frontmostPID'] == pid)
        comparison = {
            'requestedPoint': requested, 'pointerPoint': warped, 'expectedLocation': int(expected),
            'fixtureSelection': selection(evidence), 'passed': passed,
            'before': before, 'after': after}
    finally:
        # Do not undo a person's pointer movement if they took over mid-drill.
        if lease and warped is not None:
            lease.restore()
        retired = bool(lease and lease.retired)
        if comparison is not None:
            comparison['passed'] = passed and not retired
            comparison['pointerLeaseRetired'] = retired
            (evidence / f'{label}.json').write_text(json.dumps(comparison, indent=2) + '\n')
    if retired:
        # Abort the cell before another probe can acquire fresh pointer ownership.
        raise RuntimeError('fixture-pointer-lease-retired; no further probe mutation attempted')
    return passed, receipt


def fixture_probes(output):
    probes = re.findall(r'probe=([\w-]+) clickX=([\d.-]+) clickY=([\d.-]+) expectedLocation=(\d+) referenceLocation=(\d+)', output)
    expected = {'line5-before-r': '125', 'line5-after-r': '126', 'line1': '3', 'line9': '233'}
    if (len(probes) != len(expected) or {probe[0] for probe in probes} != set(expected)
            or any(index != expected[name] or reference != index for name, x, y, index, reference in probes)):
        raise RuntimeError('fixture-independent-reference-mismatch; no helper mutation attempted')
    return probes


def fixture_arguments(mode):
    return ['--unattended'] + (['--text-drill'] if mode == 'text-click' else [])


def run_cell(mode):
    require_gui()
    evidence = Path.home() / '.codex/evidence/browser-computer-use-macos' / time.strftime(f'ax-{mode}-%Y%m%d-%H%M%S')
    evidence.mkdir(parents=True, exist_ok=False)
    fixture = BUILD / 'Chariox Computer Fixture.app'
    arguments = fixture_arguments(mode)
    pid = None
    code = 1
    try:
        subprocess.run(['open', '-n', '--stdout', str(evidence / 'fixture.out'), '--stderr',
                        str(evidence / 'fixture.err'), str(fixture), '--args', *arguments], check=True)
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            require_gui()
            output = read(evidence, 'fixture.out')
            identity = re.search(r'fixturePID=(\d+) windowID=(\d+)', output)
            point = re.search(r'clickX=([\d.-]+) clickY=([\d.-]+) expectedLocation=(\d+)', output)
            if identity:
                pid = int(identity[1])
            if identity and (mode != 'text-click' or point):
                state = desktop()
                (evidence / 'desktop.json').write_text(json.dumps(state, indent=2) + '\n')
                if state['frontmostPID'] == pid:
                    break
            time.sleep(0.1)
        else:
            raise RuntimeError('fixture-not-ready-or-not-frontmost; no helper mutation attempted')
        scope = ['--enable-fixture', '--fixture-pid', str(pid), '--window-id', identity[2]]
        # The current binary can report its identity and TextEdit window choices.
        launch_helper(evidence, 'identity', ['--identity'])
        launch_helper(evidence, 'owner-windows', ['--enable-owner-window', '--list-windows'])
        if mode == 'text-click':
            launch_helper(evidence, 'text-metadata', scope + ['--target', 'text', '--ax-read'])
            probes = fixture_probes(output)
            checks = []
            receipts = []
            for at_pointer in [False, True]:
                for name, x, y, expected, reference in probes:
                    if expected != reference:
                        raise RuntimeError('fixture-reference-index-mismatch')
                    label = f'{name}-' + ('pointer' if at_pointer else 'explicit')
                    passed, result = verify_caret(evidence, label, scope, pid, x, y, expected, at_pointer)
                    checks.append(passed)
                    receipts.append(result)
                    if 'refused: permission' in result:
                        break
                if any('refused: permission' in result for result in receipts):
                    break
            receipt = ''.join(receipts)
            passed = all(checks)
        else:
            button = launch_helper(evidence, 'button', scope + ['--target', 'button', '--click'])
            scroll = launch_helper(evidence, 'scroll', scope + ['--target', 'scroll', '--scroll'])
            receipt = button + scroll
            passed = ('path=AXPress; observed fixture counter increment' in button
                      and 'path=AXScrollValue; observed vertical scroll position increased' in scroll)
        (evidence / 'desktop-after.json').write_text(json.dumps(desktop(), indent=2) + '\n')
        code = 2 if 'refused: permission' in receipt else (0 if passed else 1)
        verdict = 'PASS: public receipts and fixture checks agree.' if code == 0 else ('BLOCKED: helper permission refused.' if code == 2 else 'FAIL: application completion not proven.')
    except Exception as error:
        verdict = f'FAIL: {error}'
    finally:
        if pid:
            command = subprocess.run(['ps', '-p', str(pid), '-o', 'command='], capture_output=True, text=True).stdout.strip()
            expected_command = str(fixture / 'Contents/MacOS/fixture') + ' ' + ' '.join(arguments)
            if command == expected_command:
                try:
                    os.kill(pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
        (evidence / 'RESULT.txt').write_text(verdict + '\n')
        print(verdict)
        print(f'Evidence: {evidence}')
    return code


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--suite', action='store_true', help='also run fixture button and scroll')
    parser.add_argument('--recheck-bundle', action='store_true', help='use the separately built helper')
    options = parser.parse_args()
    if options.recheck_bundle:
        HELPER = BUILD / 'Chariox Computer Helper Recheck.app'
    wait_for_gui()
    results = [run_cell('text-click')]
    if options.suite:
        results.append(run_cell('fixture'))
    raise SystemExit(max(results))
