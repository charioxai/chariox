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
                    str(d.BUILD / 'Chariox Computer Fixture.app'), '--args', '--unattended', '--text-drill'], check=True)
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        output = d.read(E, 'fixture.out')
        identity = re.search(r'fixturePID=(\d+) windowID=(\d+)', output)
        if identity and d.desktop()['frontmostPID'] == int(identity[1]):
            pid = int(identity[1]); break
        time.sleep(.1)
    if pid is None:
        raise RuntimeError('fixture not ready')
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
        pointer.restore()
        report['pointerLeaseRetired'] = pointer.retired
        if pointer.retired: report['status'] = 'FAIL'
    if pid:
        command = subprocess.run(['ps', '-p', str(pid), '-o', 'command='], capture_output=True, text=True).stdout.strip()
        if command == str(d.BUILD / 'Chariox Computer Fixture.app/Contents/MacOS/fixture') + ' --unattended --text-drill':
            os.kill(pid, signal.SIGTERM)
    report['coverIdentityVerified'] = cover_verified
    finish_cleanup(report, lambda: (not cover_attempted or cover_verified) and not any(
        w['pid'] == pid or w['pid'] == cover_pid or w['windowID'] == cover_window_id for w in d.desktop()['windows']))
    (E / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
raise SystemExit(0 if report['status'] == 'PASS' else 1)
