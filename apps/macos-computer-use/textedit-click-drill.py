#!/usr/bin/env python3
"""Unattended, file-bound real TextEdit caret comparison using the build helper."""
import importlib.util
import argparse
import json
from pathlib import Path
import re
import subprocess
import time

spec = importlib.util.spec_from_file_location('fixture_drill', Path(__file__).with_name('text-click-drill.py'))
drill = importlib.util.module_from_spec(spec)
spec.loader.exec_module(drill)
PUBLIC_TEXT = '\n'.join(f'line {index:02d} public caret probe' for index in range(1, 13)) + '\n'


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


def reference(evidence, label, scope, document, close=False, lease=None):
    arguments = scope + ['--ax-read', '--public-text-drill-document', str(document)]
    if close:
        arguments += ['--close-public-text-drill']
    receipt = owned_helper(evidence, label, arguments, lease)
    if close:
        if 'closed public TextEdit drill window=' not in receipt:
            raise RuntimeError('TextEdit cleanup not proven: ' + receipt.splitlines()[-1])
        return None
    rows = [row.removeprefix('publicTextDrillReference ') for row in receipt.splitlines()
            if row.startswith('publicTextDrillReference ')]
    if len(rows) != 1:
        raise RuntimeError('TextEdit independent reference unavailable: ' + receipt.splitlines()[-1])
    result = json.loads(rows[0])
    probes = result['probes']
    if (len(probes) != 2 or {p['name'] for p in probes} != {'line5-before-r', 'line5-after-r'}
            or any(p['expectedLocation'] != p['referenceLocation']
                   or p['expectedLocation'] != {'line5-before-r': 125, 'line5-after-r': 126}[p['name']]
                   for p in probes)):
        raise RuntimeError('TextEdit independent reference mismatch')
    return result


def caret_matches(receipt, expected, wanted, actual):
    observed = re.search(r'path=AXSelectedTextRange; observed AXSelectedTextRange location=(\d+) length=0; point=([\d.-]+),([\d.-]+)', receipt)
    return bool(observed and int(observed[1]) == expected
                and actual == {'location': expected, 'length': 0}
                and all(abs(float(observed[i + 2]) - wanted[key]) < 0.01
                        for i, key in enumerate(['x', 'y'])))


def run():
    drill.wait_for_gui()
    evidence = Path.home() / '.codex/evidence/browser-computer-use-macos' / time.strftime('ax-textedit-%Y%m%d-%H%M%S')
    evidence.mkdir(parents=True, exist_ok=False)
    document = evidence / 'caret-probe-r4.txt'
    scope = None
    pid = None
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
        document.write_text(PUBLIC_TEXT)
        drill.require_gui()
        subprocess.run(['open', '-a', '/System/Applications/TextEdit.app', str(document)], check=True, timeout=5)
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            drill.require_gui()
            state = drill.desktop()
            windows = [w for w in state['windows'] if w['windowID'] not in existing and w['layer'] == 0
                       and w['pid'] == state['frontmostPID'] and w['bounds']['Width'] > 200
                       and w['bounds']['Height'] > 100]
            if len(windows) == 1:
                pid = windows[0]['pid']
                scope = ['--enable-owner-window', '--owner-pid', str(pid), '--window-id', str(windows[0]['windowID']), '--target', 'focused']
                break
            time.sleep(0.1)
        else:
            raise RuntimeError('new TextEdit window not unambiguously frontmost; no mutation attempted')
        (evidence / 'desktop.json').write_text(json.dumps(state, indent=2) + '\n')
        lease = drill.PointerLease(drill.desktop, drill.run_pointer_script, saved)
        lease.bind(pid, int(scope[4]))
        initial = reference(evidence, 'reference', scope, document, lease=lease)
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
                lease.require()
                actual = reference(evidence, label + '-actual', scope, document, lease=lease)
                lease.require()
                # Layout must stay unchanged during this comparison.
                passed = (actual['probes'] == initial['probes']
                          and caret_matches(receipt, probe['expectedLocation'], wanted, actual['selectedRange'])
                          and lease.require()['frontmostPID'] == pid)
                checks.append(passed)
                (evidence / f'{label}.json').write_text(json.dumps({
                    'probe': probe, 'sampledPoint': wanted, 'actual': actual, 'passed': passed}, indent=2) + '\n')
        code = 0 if len(checks) == 4 and all(checks) else 1
        verdict = 'PASS: all four real TextEdit caret comparisons agree with AX bounds and AppKit.' if code == 0 else verdict
    except Exception as error:
        code = 1
        verdict = f'FAIL: {error}'
    finally:
        if lease and warped is not None:
            lease.restore()
        if scope and lease and lease.retired:
            code = 1
            verdict = f'FAIL: cleanup incomplete: pointer lease retired; {verdict}'
        elif scope:
            try:
                reference(evidence, 'close', scope, document, close=True, lease=lease)
                deadline = time.monotonic() + 2
                while any(w['windowID'] == int(scope[4]) for w in drill.desktop()['windows']) and time.monotonic() < deadline:
                    time.sleep(0.05)
                if any(w['windowID'] == int(scope[4]) for w in drill.desktop()['windows']):
                    raise RuntimeError('TextEdit window remains visible')
            except Exception as error:
                code = 1
                verdict = f'FAIL: cleanup incomplete: {error}'
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
