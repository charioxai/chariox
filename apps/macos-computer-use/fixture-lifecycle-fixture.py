"""Headless fixture cell with real launch checks and native warp scripts in Node."""
import copy
from contextlib import redirect_stdout
import importlib.util
import io
import json
from pathlib import Path
import re
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(file))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


drill = load('lifecycle_fixture', 'text-click-drill.py')
VM = load('fixture_caret', 'caret-pointer.test.py').VM


class FixtureLifecycleCase(unittest.TestCase):
    def setUp(self):
        parent = Path.home() / '.codex/evidence/browser-computer-use-macos'
        parent.mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix='fixture-lifecycle-', dir=parent))
        self.window = {'pid': 42, 'windowID': 7, 'layer': 0,
                       'bounds': {'X': 0, 'Y': 0, 'Width': 1000, 'Height': 1000}}
        self.saved = {'x': 2, 'y': 3}
        self.state = {'frontmostPID': 42, 'pointer': self.saved, 'windows': [self.window]}
        self.probes = [('line5-before-r', 20, 125), ('line5-after-r', 40, 126),
                       ('line1', 60, 3), ('line9', 80, 233)]
        self.launches = []
        self.warps = []
        self.pointer_clicks = []
        self.restore_failure = None
        self.helper_failure = None
        self.fail_on = 4
        self.restored_facts = False
        self.run_id = 0

    def tearDown(self):
        # Inventory only this test's public files; no credentials or recursive delete.
        inventory = list(self.root.rglob('*'))
        assert all(not p.is_symlink() and (p.is_dir() or p.suffix in ['.json', '.txt', '.out', '.err'])
                   for p in inventory)
        for p in sorted(inventory, key=lambda p: len(p.parts), reverse=True):
            p.rmdir() if p.is_dir() else p.unlink()
        self.root.rmdir()

    def desktop(self):
        state = copy.deepcopy(self.state)
        state['cgPointer'] = state['pointer']
        return state

    def native(self, args, **kwargs):
        if args[0] == 'ps':
            # Do not signal any real process from this headless cell.
            return SimpleNamespace(stdout='', stderr='', returncode=0)
        if args[0] == 'codesign':
            return SimpleNamespace(stdout='', stderr='CDHash=' + 'a' * 40 + '\n', returncode=0)
        if args[0] == 'open':
            evidence = Path(args[args.index('--stdout') + 1]).parent
            if '-W' in args:
                return self.helper(evidence, Path(args[args.index('--stdout') + 1]).stem,
                                   args[args.index('--args') + 1:], args)
            output = 'fixturePID=42 windowID=7\n' + '\n'.join(
                f'probe={name} clickX={x} clickY=30 expectedLocation={index} referenceLocation={index}'
                for name, x, index in self.probes)
            (evidence / 'fixture.out').write_text(output + '\n')
            return SimpleNamespace(stdout='', stderr='', returncode=0)
        self.assertEqual(args[0], 'osascript')
        points = json.loads(re.search(r'points=(.*?), last=', args[-1])[1])
        failure = self.restore_failure if points == [self.saved] and len(self.pointer_clicks) == self.fail_on else None
        before = copy.deepcopy(self.state)
        if failure == 'focus': self.state['frontmostPID'] = 99
        elif failure == 'pointer': self.state['pointer'] = {'x': 70, 'y': 80}
        elif failure == 'window': self.state['windows'] = []
        elif failure == 'subprocess':
            self.restored_facts = True
            raise subprocess.CalledProcessError(1, args)
        child = subprocess.Popen(['node', '-e', VM], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        output, _ = child.communicate(json.dumps({'script': args[-1], 'state': self.state}))
        self.assertEqual(child.returncode, 0)
        result = json.loads(output)
        self.warps += result['warps']
        self.state['pointer'] = result['point']
        if failure:
            # Native fence saw takeover; facts return before any subsequent probe.
            self.assertEqual(result['warps'], [])
            self.state = before
            self.restored_facts = True
        return SimpleNamespace(stdout=result['stdout'], stderr='', returncode=0)

    def helper(self, evidence, label, arguments, command):
        self.launches.append(label)
        receipt = ''
        if '--click' in arguments:
            at_pointer = '--click-at-pointer' in arguments
            if at_pointer:
                self.pointer_clicks.append(label)
                point = self.state['pointer']
            else:
                offset = arguments.index('--click-at')
                point = {'x': float(arguments[offset + 1]), 'y': float(arguments[offset + 2])}
            index = next(index for name, x, index in self.probes if x == point['x'])
            with (evidence / 'fixture.out').open('a') as output:
                output.write(f'fixture selection location={index} length=0\n')
            receipt = (f'dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location={index} '
                       f'length=0; point={point["x"]},{point["y"]}\n')
            failure = self.helper_failure if at_pointer and len(self.pointer_clicks) == self.fail_on else None
            if failure:
                self.restored_facts = True
                receipt = 'refused: target (not-frontmost)\n'
                (evidence / f'{label}.err').write_text(receipt)
                if failure == 'exit': raise subprocess.CalledProcessError(1, command)
                if failure == 'timeout': raise subprocess.TimeoutExpired(command, 8)
        else:
            failure = None
        identity = {'path': str(drill.HELPER / 'Contents/MacOS/helper'), 'cdhash': 'a' * 40,
                    'identifier': 'ai.chariox.computer-helper', 'signatureValid': True, 'parentPID': 1}
        if failure == 'identity': identity['cdhash'] = 'b' * 40
        (evidence / f'{label}.out').write_text(json.dumps(identity) + '\n' + receipt)
        return SimpleNamespace(stdout='', stderr='', returncode=0)

    def invoke(self):
        self.run_id += 1
        with redirect_stdout(io.StringIO()), \
             patch.object(drill, 'desktop', side_effect=self.desktop), patch.object(drill, 'require_gui'), \
             patch.object(subprocess, 'run', side_effect=self.native), \
             patch.object(Path, 'home', return_value=self.root), \
             patch.object(drill.time, 'strftime', return_value=f'run-{self.run_id}'):
            code = drill.run_cell('text-click')
        result = self.root / '.codex/evidence/browser-computer-use-macos' / f'run-{self.run_id}'
        verdict = (result / 'RESULT.txt').read_text()
        comparisons = {p.stem: json.loads(p.read_text())['passed'] for p in result.glob('line*.json')
                       if not p.stem.endswith('-warp')}
        print(json.dumps({'restoreFailure': self.restore_failure, 'helperFailure': self.helper_failure,
                          'failOn': self.fail_on, 'code': code, 'verdict': verdict.strip(),
                          'launches': self.launches, 'warps': self.warps,
                          'comparisons': comparisons, 'restoredFacts': self.restored_facts}))
        return code, verdict, result
