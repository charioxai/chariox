"""Run the real fence entry point and launch verifier without desktop effects."""
import copy
from contextlib import redirect_stdout
import importlib.util
import io
import json
from pathlib import Path
import re
import runpy
import signal
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from fence_cleanup import finish_cleanup

spec = importlib.util.spec_from_file_location('fence_vm', Path(__file__).with_name('caret-pointer.test.py'))
vm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vm)
VM = vm.VM.replace('CGEventCreate:()', 'NSThread:{sleepForTimeInterval(){}},CGEventCreate:()')
REAL_POPEN = subprocess.Popen


class FenceLifecycleCase(unittest.TestCase):
    def setUp(self):
        parent = Path.home() / '.codex/evidence/browser-computer-use-macos'
        parent.mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix='fence-lifecycle-', dir=parent))
        self.window = {'pid': 42, 'windowID': 7, 'bounds': {'X': 0, 'Y': 0, 'Width': 1000, 'Height': 1000}}
        self.saved = {'x': 2, 'y': 3}
        self.state = {'frontmostPID': 42, 'pointer': self.saved, 'windows': [self.window],
                      'screens': [{'frame': {'origin': {'y': 0}, 'size': {'height': 1000}}}]}
        self.launches = []; self.warps = []; self.jitters = 0
        self.fail_label = None; self.helper_failure = None; self.restore_failure = None
        self.restored_facts = False
        self.cover_identity_failure = None; self.cover_close_failure = None
        self.cover_signals = 0
        self.clock = 0; self.cover_process_reads = 0
        self.panel_signals = 0; self.panel_failure = None

    def now(self):
        self.clock += .05
        return self.clock

    def tearDown(self):
        inventory = list(self.root.rglob('*'))
        assert all(not p.is_symlink() and (p.is_dir() or p.suffix in ['.json', '.out', '.err', '.png']) for p in inventory)
        for p in sorted(inventory, key=lambda p: len(p.parts), reverse=True):
            p.rmdir() if p.is_dir() else p.unlink()
        self.root.rmdir()

    def execute(self, script):
        points = json.loads(re.search(r'points=(.*?), last=', script)[1])
        failure = self.restore_failure if points == [self.saved] else None
        before = copy.deepcopy(self.state)
        if failure == 'focus': self.state['frontmostPID'] = 99
        elif failure == 'pointer': self.state['pointer'] = {'x': 70, 'y': 80}
        elif failure == 'window': self.state['windows'] = []
        elif failure == 'subprocess':
            self.restored_facts = True
            raise subprocess.CalledProcessError(1, ['osascript'])
        child = REAL_POPEN(['node', '-e', VM], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        output, _ = child.communicate(json.dumps({'script': script, 'state': self.state}))
        self.assertEqual(child.returncode, 0)
        result = json.loads(output)
        self.warps.extend(result['warps']); self.state['pointer'] = result['point']
        if failure:
            self.assertEqual(result['warps'], [])
            self.state = before; self.restored_facts = True
        return result['stdout']

    def native(self, args, **kwargs):
        if args[0] == 'ps':
            build = Path('/Users/miguel/.chariox/dev/cumac/build')
            if args[args.index('-p') + 1] == '43':
                self.cover_process_reads += 1
                command = str(build / 'Chariox Computer Cover.app/Contents/MacOS/cover') + ' -20.0 910.0 120 120'
                if self.cover_identity_failure == 'process' or (self.cover_identity_failure == 'process-after' and self.cover_process_reads > 1):
                    command = '/unrelated/process'
                return SimpleNamespace(stdout=command, returncode=0)
            return SimpleNamespace(stdout=str(build / 'Chariox Computer Fixture.app/Contents/MacOS/fixture') + ' --unattended --text-drill --cover-panel', returncode=0)
        if args[0] == 'codesign': return SimpleNamespace(stderr='CDHash=' + 'a' * 40 + '\n', stdout='', returncode=0)
        if args[0] == 'osascript':
            output = json.dumps(self.state) if 'var screens = [];' in args[-1] else self.execute(args[-1])
            return SimpleNamespace(stdout=output, stderr='', returncode=0)
        self.assertEqual(args[0], 'open')
        path = Path(args[args.index('--stdout') + 1]); label = path.stem
        if '-W' not in args:
            path.write_text('fixturePID=42 windowID=7\n' + '\n'.join(
                f'probe={name} clickX={x} clickY=30 expectedLocation={index} referenceLocation={index}'
                for name, x, index in [('line5-before-r', 20, 125), ('line5-after-r', 40, 126),
                                       ('line1', 60, 3), ('line9', 80, 233)]) + '\nfixture selection location=0 length=0\n')
            return SimpleNamespace(stdout='', stderr='', returncode=0)
        self.launches.append(label)
        receipt = 'accessibility=granted' if label == 'preflight' else ('refused: target (stale-pointer)' if label == 'stale-pointer' else 'refused: target (foreign-window-cover)')
        fixture_out = path.with_name('fixture.out')
        if label.startswith('same-process-outside'):
            location = 125 if label.endswith('click') else 3
            receipt = f'path=AXSelectedTextRange; observed AXSelectedTextRange location={location} length=0; point=1,1'
            with fixture_out.open('a') as out: out.write(f'fixture selection location={location} length=0\n')
        elif label == 'same-process-uncovered-capture':
            receipt = 'frame 100x100 window=7'
            Path(args[args.index('--capture') + 1]).write_bytes(b'png')
        failure = self.helper_failure if self.fail_label == label else None
        if failure:
            # A helper has observed takeover, but desktop facts have returned.
            self.restored_facts = True
            receipt = 'refused: target (not-frontmost)'
            if failure == 'mixed': receipt = 'refused: target (foreign-window-cover)\n' + receipt
            if failure == 'success': receipt = 'dispatched'
            if failure == 'exit': raise subprocess.CalledProcessError(1, args)
            if failure == 'timeout': raise subprocess.TimeoutExpired(args, 8)
        identity = {'path': str(Path(args[args.index('--args') - 1]) / 'Contents/MacOS/helper'), 'cdhash': 'a' * 40,
                    'identifier': 'ai.chariox.computer-helper', 'signatureValid': True, 'parentPID': 1}
        if failure == 'identity': identity['cdhash'] = 'b' * 40
        path.write_text(json.dumps(identity) + '\n' + receipt + '\n')
        return SimpleNamespace(stdout='', stderr='', returncode=0)

    def popen(self, args, **kwargs):
        if args[0] == 'open':
            receipt = 'coverPID=43 windowID=8\n'
            if self.cover_identity_failure == 'missing': receipt = ''
            elif self.cover_identity_failure == 'malformed': receipt = 'coverPID=invalid windowID=8\n'
            elif self.cover_identity_failure == 'zero-pid': receipt = 'coverPID=0 windowID=8\n'
            elif self.cover_identity_failure == 'zero-window': receipt = 'coverPID=43 windowID=0\n'
            Path(args[args.index('--stdout') + 1]).write_text(receipt)
            self.state['windows'].append({'pid': 99 if self.cover_identity_failure == 'window-pid' else 43,
                'windowID': 9 if self.cover_identity_failure == 'window-id' else 8,
                'bounds': {'X': 0, 'Y': 0, 'Width': 100, 'Height': 100}})
            if self.cover_identity_failure == 'launch': raise RuntimeError('cover-launch-failed')
        else:
            self.assertEqual(args[0], 'osascript')
            self.jitters += 1
            kwargs['stdout'].write(self.execute(args[-1])); kwargs['stdout'].flush()
        return SimpleNamespace(poll=lambda: 0, wait=lambda **kw: 0)

    def kill(self, pid, sig):
        self.assertIn(pid, [42, 43])
        if sig == signal.SIGUSR1:
            self.assertEqual(pid, 42)
            self.panel_signals += 1
            out = self.root / '.codex/evidence/browser-computer-use-macos/case/fixture.out'
            if self.panel_failure == 'silent': return
            panels = [w for w in self.state['windows'] if w['windowID'] == 10]
            if panels:
                if self.panel_failure != 'sticky': self.state['windows'].remove(panels[0])
                line = 'coverPanel closed'
            else:
                panel = {'pid': 42, 'windowID': 10, 'bounds': {'X': 400, 'Y': 400, 'Width': 200, 'Height': 140}}
                if self.panel_failure == 'below': self.state['windows'].append(panel)
                else: self.state['windows'].insert(0, panel)
                line = f'coverPanelPID={99 if self.panel_failure == "foreign-pid" else 42} windowID=10 mainWindowID=7 frame=(400.0, 400.0, 200.0, 140.0)'
            with out.open('a') as handle: handle.write(line + '\n')
            return
        if pid == 43:
            self.cover_signals += 1
            if self.cover_close_failure == 'permission': raise PermissionError('cover-signal-denied')
            if self.cover_close_failure == 'gone': raise ProcessLookupError('cover-process-gone')
            if self.cover_close_failure == 'sticky': return
        self.state['windows'] = [w for w in self.state['windows'] if w['pid'] != pid]

    def invoke(self):
        cleanup = lambda report, closed: finish_cleanup(report, closed, now=self.now, wait=lambda seconds: None)
        with redirect_stdout(io.StringIO()), patch.object(Path, 'home', return_value=self.root), \
             patch.object(subprocess, 'run', side_effect=self.native), patch.object(subprocess, 'Popen', side_effect=self.popen), \
             patch('os.kill', side_effect=self.kill), patch('time.sleep'), patch('time.monotonic', side_effect=self.now), \
             patch('fence_cleanup.finish_cleanup', side_effect=cleanup), patch('time.strftime', return_value='case'):
            # Both GUI guards use desktop(), whose native boundary is injected.
            with patch.dict('os.environ', {'CUMAC_BUILD_DIR': '/Users/miguel/.chariox/dev/cumac/build'}):
                try: runpy.run_path(str(Path(__file__).with_name('fence-drill.py')), run_name='__main__')
                except SystemExit as result: code = result.code
        report = json.loads((self.root / '.codex/evidence/browser-computer-use-macos/case/result.json').read_text())
        print(json.dumps({'helperFailure': self.helper_failure, 'label': self.fail_label, 'restoreFailure': self.restore_failure,
                          'code': code, 'report': report, 'launches': self.launches, 'warpCount': len(self.warps), 'lastWarp': self.warps[-1:] , 'jitters': self.jitters}))
        return code, report
