"""Headless real TextEdit runner fixture; native warp scripts execute in Node."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(file))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


textedit = load('lifecycle_textedit', 'textedit-click-drill.py')
VM = load('lifecycle_caret', 'caret-pointer.test.py').VM


class TextEditLifecycleCase(unittest.TestCase):
    def setUp(self):
        parent = Path.home() / '.codex/evidence/browser-computer-use-macos'
        parent.mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix='textedit-lifecycle-', dir=parent))
        self.window = {'pid': 42, 'windowID': 7, 'layer': 0,
                       'bounds': {'X': 0, 'Y': 0, 'Width': 1000, 'Height': 1000}}
        self.state = {'frontmostPID': 42, 'pointer': {'x': 2, 'y': 3}, 'windows': []}
        self.probes = [{'name': name, 'point': {'x': x, 'y': 30},
                        'expectedLocation': location, 'referenceLocation': location}
                       for name, x, location in [('line5-before-r', 20, 125), ('line5-after-r', 40, 126)]]
        self.selected = {'location': 125, 'length': 0}
        self.launches = []
        self.warps = []
        self.restore_failure = None
        self.reference_failure = None
        self.reference_error = False
        self.kills = []
        self.human_input = False
        self.extra_window_at = None
        self.run_id = 0

    def tearDown(self):
        inventory = list(self.root.rglob('*'))
        assert all(not p.is_symlink() and (p.is_dir() or p.suffix in ['.json', '.txt']) for p in inventory)
        for p in sorted(inventory, key=lambda p: len(p.parts), reverse=True):
            p.rmdir() if p.is_dir() else p.unlink()
        self.root.rmdir()

    def desktop(self):
        state = copy.deepcopy(self.state)
        state['cgPointer'] = state['pointer']
        return state

    def native(self, args, **kwargs):
        if args[0] == 'open':
            self.state['windows'] = [self.window]
            return SimpleNamespace(stdout='', stderr='', returncode=0)
        self.assertEqual(args[0], 'osascript')
        # Both pointer probes have finished; the next warp is final restoration.
        if len(self.warps) == 2 and self.restore_failure:
            if self.restore_failure == 'focus': self.state['frontmostPID'] = 99
            elif self.restore_failure == 'pointer': self.state['pointer'] = {'x': 70, 'y': 80}
            else: raise subprocess.CalledProcessError(1, args)
        child = subprocess.Popen(['node', '-e', VM], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        output, _ = child.communicate(json.dumps({'script': args[-1], 'state': self.state}))
        self.assertEqual(child.returncode, 0)
        result = json.loads(output)
        self.warps += result['warps']
        self.state['pointer'] = result['point']
        return SimpleNamespace(stdout=result['stdout'], stderr='', returncode=0)

    def helper(self, evidence, label, arguments):
        self.launches.append(label)
        if '--drill-permissions' in arguments: return 'drillPermission accessibility=granted'
        if label == self.reference_failure:
            # The helper saw takeover, but desktop facts are already restored.
            if self.reference_error: raise RuntimeError('helper-launch-identity-mismatch')
            return 'refused: target (not-frontmost)'
        if label == self.extra_window_at:
            # A user opens another document in the runner's instance.
            self.state['windows'] = [*self.state['windows'], {**self.window, 'windowID': 8}]
        if '--capture' in arguments:
            return 'frame 2000x2000 window=7'
        if '--click-at-pointer' in arguments: point = self.state['pointer']
        else:
            index = arguments.index('--click-at')
            point = {'x': float(arguments[index + 1]), 'y': float(arguments[index + 2])}
        self.selected = {'location': 125 if point['x'] == 20 else 126, 'length': 0}
        return (f'dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location={self.selected["location"]} '
                f'length=0; point={point["x"]},{point["y"]}')

    def kill(self, pid, sig):
        self.kills.append(pid)
        self.state['windows'] = []

    def invoke(self):
        self.run_id += 1
        with patch.object(textedit.drill, 'desktop', side_effect=self.desktop), \
             patch.object(textedit.drill, 'require_gui'), patch.object(textedit.drill, 'wait_for_gui'), \
             patch.object(textedit.drill, 'launch_helper', side_effect=self.helper), \
             patch.object(subprocess, 'run', side_effect=self.native), \
             patch.object(textedit, 'run_reference', return_value={'probes': self.probes}), \
             patch.object(textedit, 'process_identity', return_value=time.strftime('%a %b %d %H:%M:%S %Y ', time.localtime(time.time() + 60)) + textedit.TEXTEDIT), \
             patch.object(textedit.os, 'kill', side_effect=self.kill), patch.object(textedit.time, 'sleep'), \
             patch.object(textedit, 'real_input_since', side_effect=lambda launched: self.human_input), \
             patch.object(Path, 'home', return_value=self.root), \
             patch.object(textedit.time, 'strftime', return_value=f'run-{self.run_id}'):
            code = textedit.run()
        result = self.root / '.codex/evidence/browser-computer-use-macos' / f'run-{self.run_id}'
        return code, (result / 'RESULT.txt').read_text(), list(result.glob('line*.json'))
