from fence_cleanup import finish_cleanup
from fence_pointer import PointerLease, run_pointer_script
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time

E = Path.home() / '.codex/evidence/browser-computer-use-macos' / time.strftime('r6-fences-%Y%m%d-%H%M%S')
E.mkdir(parents=True, exist_ok=False)
spec = importlib.util.spec_from_file_location('d', str(Path(__file__).with_name('text-click-drill.py')))
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)
pid = None
cover = None
cover_pid = None
cover_window_id = None
cover_command = None
cover_attempted = False
cover_verified = False
jitter = None
report = {'status': 'FAIL', 'checks': []}
pointer = None
fixture_arguments = ['--unattended', '--text-drill', '--cover-panel']
fixture_command = ' '.join([str(d.BUILD / 'Chariox Computer Fixture.app/Contents/MacOS/fixture')] + fixture_arguments)


def fence_helper(label, arguments, expected=None):
    try:
        receipt = d.launch_helper(E, label, arguments)
        if expected is not None:
            refusals = re.findall(r'^refused:.*$', receipt, re.MULTILINE)
            if refusals != [f'refused: target ({expected})'] or 'dispatched' in receipt:
                raise RuntimeError(f'unexpected-fence-receipt: {label}')
        return receipt
    except Exception:
        pointer.retired = True
        raise


def toggle_panel():
    # SIGUSR1 opens or closes the fixture's own non-key panel; only signal a process that is provably this fixture.
    command = subprocess.run(['ps', '-ww', '-p', str(pid), '-o', 'command='], capture_output=True, text=True).stdout.strip()
    if command != fixture_command:
        raise RuntimeError('fixture-process-identity-changed')
    marker = re.compile(r'^coverPanel(?:PID=.*| closed)$', re.MULTILINE)
    seen = len(marker.findall(d.read(E, 'fixture.out')))
    os.kill(pid, signal.SIGUSR1)
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        lines = marker.findall(d.read(E, 'fixture.out'))
        if len(lines) > seen:
            return lines[-1]
        time.sleep(.05)
    raise RuntimeError('cover-panel-signal-unanswered')


def stop_cover():
    global cover_verified
    if cover_pid is None:
        return
    command = subprocess.run(['ps', '-ww', '-p', str(cover_pid), '-o', 'command='], capture_output=True, text=True).stdout.strip()
    if command and command != cover_command:
        cover_verified = False
        raise RuntimeError('cover-process-identity-changed')
    if command:
        try:
            os.kill(cover_pid, signal.SIGTERM)
        except ProcessLookupError:
            pass


try:
    d.wait_for_gui()
    d.require_gui()
    pointer = PointerLease(d.desktop, run_pointer_script, d.desktop()["pointer"])
    permission = fence_helper('preflight', ['--drill-permissions'])
    if 'accessibility=granted' not in permission:
        report['status'] = 'BLOCKED'; raise RuntimeError('helper-regrant-required')
    subprocess.run(['open', '-n', '--stdout', str(E / 'fixture.out'), '--stderr', str(E / 'fixture.err'),
                    str(d.BUILD / 'Chariox Computer Fixture.app'), '--args', *fixture_arguments], check=True)
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        output = d.read(E, 'fixture.out')
        identity = re.search(r'fixturePID=(\d+) windowID=(\d+)', output)
        if identity and d.desktop()['frontmostPID'] == int(identity[1]):
            pid = int(identity[1]); break
        time.sleep(.1)
    if pid is None:
        raise RuntimeError('fixture not ready')
    # A fresh window animates into place; bind the lease only to stable geometry.
    last = None
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        bounds = [w['bounds'] for w in d.desktop()['windows'] if w['pid'] == pid and w['windowID'] == int(identity[2])]
        if len(bounds) == 1 and bounds[0] == last:
            break
        last = bounds[0] if len(bounds) == 1 else None
        time.sleep(.3)
    else:
        raise RuntimeError('fixture-geometry-not-stable')
    pointer.bind(pid, int(identity[2]))
    probes = d.fixture_probes(output)
    name, x, y, expected, reference = probes[1]
    scope = ['--enable-fixture', '--fixture-pid', str(pid), '--window-id', identity[2], '--target', 'text']
    # A foreign opaque panel covers the probe without activating or changing AX focus.
    with (E / 'cover.out').open('w') as out, (E / 'cover.err').open('w') as err:
        top = d.desktop()['screens'][0]['frame']['origin']['y'] + d.desktop()['screens'][0]['frame']['size']['height']
        cover_arguments = [str(float(x) - 60), str(top - float(y) - 60), '120', '120']
        cover_command = ' '.join([str(d.BUILD / 'Chariox Computer Cover.app/Contents/MacOS/cover')] + cover_arguments)
        cover_attempted = True
        cover = subprocess.Popen(['open', '-n', '-g', '--stdout', str(E / 'cover.out'), '--stderr', str(E / 'cover.err'), str(d.BUILD / 'Chariox Computer Cover.app'), '--args'] + cover_arguments, stdout=out, stderr=err)
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline and 'coverPID=' not in d.read(E, 'cover.out'):
        time.sleep(.05)
    match = re.search(r'coverPID=(\d+) windowID=(\d+)', d.read(E, 'cover.out'))
    if not match or int(match[1]) <= 0 or int(match[2]) <= 0:
        raise RuntimeError('cover-identity-missing')
    command = subprocess.run(['ps', '-ww', '-p', match[1], '-o', 'command='], capture_output=True, text=True).stdout.strip()
    if command != cover_command:
        raise RuntimeError('cover-process-identity-unverified')
    cover_pid = int(match[1]); cover_window_id = int(match[2])
    time.sleep(.15)
    state = d.desktop()
    foreign_window = next((w for w in state['windows'] if w['pid'] == cover_pid and w['windowID'] == cover_window_id), None)
    if foreign_window is None:
        raise RuntimeError('cover-window-identity-unverified')
    cover_verified = True
    (E / 'covered-desktop.json').write_text(json.dumps(state, indent=2) + '\n')
    selected_before = d.selection(E)
    receipt = fence_helper('nonactivating-cover-click', scope + ['--click-at', x, y, '--click'], 'foreign-window-cover')
    time.sleep(.2)
    selected_after = d.selection(E)
    covers = bool(foreign_window and foreign_window['bounds']['X'] <= float(x) <= foreign_window['bounds']['X'] + foreign_window['bounds']['Width']
                  and foreign_window['bounds']['Y'] <= float(y) <= foreign_window['bounds']['Y'] + foreign_window['bounds']['Height'])
    report['checks'].append({'name': 'nonactivating-foreign-cover-refusal', 'passed': 'refused: target (foreign-window-cover)' in receipt and selected_after == selected_before and covers and state['frontmostPID'] == pid,
                             'frontmostUnchanged': state['frontmostPID'] == pid, 'foreignWindowCoversPoint': covers,
                             'beforeSelection': selected_before, 'afterSelection': selected_after, 'receipt': receipt})
    pointer.warp({'x': float(x), 'y': float(y)})
    receipt = fence_helper('nonactivating-cover-pointer', scope + ['--click-at-pointer', '--click'], 'foreign-window-cover')
    time.sleep(.1)
    report['checks'].append({'name': 'pointer-foreign-cover-refusal',
        'passed': 'refused: target (foreign-window-cover)' in receipt and d.selection(E) == selected_before,
        'receipt': receipt})
    stop_cover()
    time.sleep(.2)
    if any(w['pid'] == cover_pid or w['windowID'] == cover_window_id for w in d.desktop()['windows']):
        raise RuntimeError('cover-window-still-open')
    stale_before = d.selection(E)
    # Move the real pointer between the helper's one-time snapshot and its later fence.
    point_a = {'x': float(x), 'y': float(y)}
    point_b = {'x': float(x) + 10, 'y': float(y)}
    script = pointer.jitter_script([point_a, point_b])
    with (E / 'jitter.out').open('w') as out, (E / 'jitter.err').open('w') as err:
        jitter = subprocess.Popen(['osascript', '-l', 'JavaScript', '-e', script], stdout=out, stderr=err)
    time.sleep(.1)
    receipt = fence_helper('stale-pointer', scope + ['--click-at-pointer', '--click'], 'stale-pointer')
    jitter.wait(timeout=4)
    pointer.finish_jitter(d.read(E, "jitter.out"))
    report['checks'].append({'name': 'live-stale-pointer-tag', 'passed': 'refused: target (stale-pointer)' in receipt and d.selection(E) == stale_before, 'receipt': receipt,
                             'fixtureSelection': d.selection(E)})
    # Same-process cover: a non-key panel of the fixture's own process sits over the still-focused
    # target, so the focus fence stays silent and only the WindowServer stack fence can refuse.
    main_id = int(identity[2])
    line = toggle_panel()
    opened = re.fullmatch(r'coverPanelPID=(\d+) windowID=(\d+) mainWindowID=(\d+) frame=.*', line)
    if not opened or int(opened[1]) != pid or int(opened[3]) != main_id:
        raise RuntimeError('cover-panel-identity-missing')
    panel_id = int(opened[2])
    time.sleep(.15)
    state = d.desktop()
    (E / 'same-process-desktop.json').write_text(json.dumps(state, indent=2) + '\n')
    order = [w['windowID'] for w in state['windows']]
    panel = next((w for w in state['windows'] if w['pid'] == pid and w['windowID'] == panel_id), None)
    if panel is None or main_id not in order or order.index(panel_id) > order.index(main_id):
        raise RuntimeError('cover-panel-not-above-target')
    box = panel['bounds']
    inside = (box['X'] + box['Width'] / 2, box['Y'] + box['Height'] / 2)
    covered_selection = d.selection(E)

    def outside(point):
        return not (box['X'] <= float(point[0]) <= box['X'] + box['Width'] and box['Y'] <= float(point[1]) <= box['Y'] + box['Height'])

    receipt = fence_helper('same-process-cover-click', scope + ['--click-at', str(inside[0]), str(inside[1]), '--click'], 'foreign-window-cover')
    time.sleep(.2)
    report['checks'].append({'name': 'same-process-cover-click-refusal', 'passed': d.selection(E) == covered_selection and d.desktop()['frontmostPID'] == pid,
                             'panelAboveTarget': True, 'receipt': receipt})
    pointer.warp({'x': inside[0], 'y': inside[1]})
    receipt = fence_helper('same-process-cover-pointer', scope + ['--click-at-pointer', '--click'], 'foreign-window-cover')
    time.sleep(.1)
    report['checks'].append({'name': 'same-process-cover-pointer-refusal', 'passed': d.selection(E) == covered_selection, 'receipt': receipt})
    denied = E / 'same-process-denied.png'
    receipt = fence_helper('same-process-cover-capture', scope + ['--capture', str(denied)], 'foreign-window-cover')
    report['checks'].append({'name': 'same-process-cover-capture-refusal', 'passed': not denied.exists(), 'receipt': receipt})
    # Outside the overlap the same fixture and cover still accept the same operations.
    for label, probe, at_pointer in [('same-process-outside-click', probes[0], False), ('same-process-outside-pointer', probes[2], True)]:
        name, x, y, expected, reference = probe
        if not outside((x, y)):
            raise RuntimeError('cover-panel-overlaps-probe')
        if at_pointer:
            pointer.warp({'x': float(x), 'y': float(y)})
        receipt = fence_helper(label, scope + (['--click-at-pointer', '--click'] if at_pointer else ['--click-at', x, y, '--click']))
        deadline = time.monotonic() + 1
        while time.monotonic() < deadline and d.selection(E) != (expected, '0'):
            time.sleep(.05)
        report['checks'].append({'name': label, 'passed': 'refused:' not in receipt and f'observed AXSelectedTextRange location={expected}' in receipt
                                 and d.selection(E) == (expected, '0'), 'receipt': receipt, 'fixtureSelection': d.selection(E)})
    if 'closed' not in toggle_panel():
        raise RuntimeError('cover-panel-close-unconfirmed')
    time.sleep(.15)
    if any(w['windowID'] == panel_id for w in d.desktop()['windows']):
        raise RuntimeError('cover-panel-still-open')
    allowed = E / 'same-process-uncovered.png'
    receipt = fence_helper('same-process-uncovered-capture', scope + ['--capture', str(allowed)])
    report['checks'].append({'name': 'same-process-uncovered-capture', 'passed': 'refused:' not in receipt and re.search(r'frame \d+x\d+ window=', receipt) is not None
                             and allowed.exists() and allowed.stat().st_size > 0, 'receipt': receipt})
    report['status'] = 'PASS' if all(c['passed'] for c in report['checks']) else 'FAIL'
except Exception as error:
    if pointer: pointer.retired = True
    report['failure'] = str(error)
finally:
    try:
        stop_cover()
    except Exception as error:
        report['coverCleanupFailure'] = str(error)
        report['status'] = 'FAIL'
    for process in [cover, jitter]:
        if process and process.poll() is None:
            process.terminate(); process.wait(timeout=3)
    if pointer:
        if not pointer.retired:
            try:
                # Public geometry only: the facts the lease compares before it restores the pointer.
                now = d.desktop()
                report['leaseFacts'] = {'frontmostPID': now['frontmostPID'], 'pid': pointer.pid, 'pointer': now['pointer'], 'last': pointer.last,
                                        'bounds': pointer.bounds, 'window': [w['bounds'] for w in now['windows'] if w['windowID'] == pointer.window]}
            except Exception:
                pass
        pointer.restore()
        report['pointerLeaseRetired'] = pointer.retired
        if pointer.retired: report['status'] = 'FAIL'
    if pid:
        command = subprocess.run(['ps', '-p', str(pid), '-o', 'command='], capture_output=True, text=True).stdout.strip()
        if command == fixture_command:
            os.kill(pid, signal.SIGTERM)
    report['coverIdentityVerified'] = cover_verified
    finish_cleanup(report, lambda: (not cover_attempted or cover_verified) and not any(
        w['pid'] == pid or w['pid'] == cover_pid or w['windowID'] == cover_window_id for w in d.desktop()['windows']))
    (E / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
raise SystemExit(0 if report['status'] == 'PASS' else 1)
