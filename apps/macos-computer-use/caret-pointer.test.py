"""Exercise both public caret runners with native warp scripts in a Node VM."""
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


fixture = load('fixture', 'text-click-drill.py')
textedit = load('textedit', 'textedit-click-drill.py')
VM = """const vm=require('node:vm'),fs=require('node:fs');
const {script,state}=JSON.parse(fs.readFileSync(0,'utf8'));let point=state.pointer,warps=[];
const $={NSWorkspace:{sharedWorkspace:{frontmostApplication:{processIdentifier:state.frontmostPID}}},
CGWindowListCopyWindowInfo:()=>state.windows.map(w=>({kCGWindowNumber:w.windowID,kCGWindowOwnerPID:w.pid,kCGWindowBounds:w.bounds})),
CGEventCreate:()=>null,CGEventGetLocation:()=>point,CGWarpMouseCursorPosition:p=>{point=p;warps.push(p);return 0;}};
const result=vm.runInNewContext(script,{ObjC:{import(){},deepUnwrap:x=>x,castRefToObject:x=>x},$,JSON,Number,Math});
process.stdout.write(JSON.stringify({stdout:String(result),point,warps}));"""


class CaretPointerTests(unittest.TestCase):
    def setUp(self):
        evidence = Path.home() / '.codex/evidence/browser-computer-use-macos'
        evidence.mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix='caret-takeover-', dir=evidence))
        self.window = {'pid': 42, 'windowID': 7, 'layer': 0, 'bounds': {'X': 0, 'Y': 0, 'Width': 1000, 'Height': 1000}}
        self.state = {'frontmostPID': 42, 'pointer': {'x': 2, 'y': 3}, 'windows': [self.window]}
        self.warps = []
        self.launch_change = None
        self.return_focus = False
        self.native_change = None
        self.run_id = 0
        self.helper_refusal = False
        self.clicks = []

    def tearDown(self):
        # Exact public fixture files only; no recursive removal or runtime home.
        inventory = list(self.root.rglob('*'))
        assert all(not p.is_symlink() and (p.is_dir() or p.suffix in ['.json', '.txt']) for p in inventory)
        for path in sorted(inventory, key=lambda p: len(p.parts), reverse=True):
            path.rmdir() if path.is_dir() else path.unlink()
        self.root.rmdir()

    def desktop(self):
        result = copy.deepcopy(self.state)
        result['cgPointer'] = result['pointer']
        if self.return_focus and self.state['frontmostPID'] == 99:
            self.state['frontmostPID'] = 42
        return result

    def native(self, args, **kwargs):
        if args[0] == 'open':
            self.state['windows'] = [self.window]
            return SimpleNamespace(stdout='', stderr='', returncode=0)
        self.assertEqual(args[0], 'osascript')
        if self.native_change:
            if self.native_change == 'focus': self.state['frontmostPID'] = 99
            elif self.native_change == 'pointer': self.state['pointer'] = {'x': 70, 'y': 80}
            else: self.state['windows'] = []
            self.native_change = None
        child = subprocess.Popen(['node', '-e', VM], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        output, _ = child.communicate(json.dumps({'script': args[-1], 'state': self.state}))
        self.assertEqual(child.returncode, 0)
        result = json.loads(output)
        self.warps += result['warps']; self.state['pointer'] = result['point']
        return SimpleNamespace(stdout=result['stdout'], stderr='', returncode=0)

    def helper(self, evidence, label, arguments):
        if '--drill-permissions' in arguments: return 'drillPermission accessibility=granted'
        if '--click-at-pointer' in arguments and self.launch_change:
            self.state['frontmostPID'] = 99
        if '--click' in arguments:
            self.clicks.append(label)
        if '--click-at-pointer' in arguments and self.helper_refusal:
            # The helper saw focus loss; all facts have returned before Python samples.
            return 'refused: target (not-frontmost)'
        point = self.state['pointer'] if '--click-at-pointer' in arguments else {'x': 20, 'y': 30}
        return f'dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location=125 length=0; point={point["x"]},{point["y"]}'

    def invoke(self, runner):
        self.run_id += 1
        drill = fixture if runner == 'fixture' else textedit.drill
        with patch.object(drill, 'desktop', side_effect=self.desktop), patch.object(drill, 'require_gui'), \
             patch.object(drill, 'wait_for_gui'), patch.object(drill, 'launch_helper', side_effect=self.helper), \
             patch.object(subprocess, 'run', side_effect=self.native):
            if runner == 'fixture':
                with patch.object(drill, 'selection', return_value=('125', '0')):
                    try: drill.verify_caret(self.root, 'probe', ['--window-id', '7'], 42, '20', '30', '125', True)
                    except RuntimeError: pass
            else:
                self.state['windows'] = []
                def reference(evidence, label, scope, lease):
                    return {'probes': [{'name': 'probe', 'point': {'x': 20, 'y': 30}, 'expectedLocation': 125},
                                       {'name': 'second', 'point': {'x': 40, 'y': 50}, 'expectedLocation': 125}]}
                def kill(pid, sig): self.state['windows'] = []
                with patch.object(Path, 'home', return_value=self.root), patch.object(textedit, 'reference', side_effect=reference), \
                     patch.object(textedit, 'process_identity', return_value=time.strftime('%a %b %d %H:%M:%S %Y ', time.localtime(time.time() + 60)) + textedit.TEXTEDIT), \
                     patch.object(textedit.os, 'kill', side_effect=kill), patch.object(textedit.time, 'sleep'), \
                     patch.object(textedit, 'real_input_since', return_value=False), \
                     patch.object(textedit.time, 'strftime', return_value=f'takeover-{self.run_id}'):
                    textedit.run()

    def test_helper_takeover_refusal_retires_lease_before_restored_facts_are_sampled(self):
        for runner in ['fixture', 'textedit']:
            with self.subTest(runner=runner):
                self.state = {'frontmostPID': 42, 'pointer': {'x': 2, 'y': 3}, 'windows': [self.window]}
                self.warps = []; self.clicks = []; self.helper_refusal = True
                self.invoke(runner)
                self.assertEqual(self.warps, [{'x': 20.0, 'y': 30.0}])
                pointer_clicks = [name for name in self.clicks if name.endswith('pointer') or name == 'probe']
                self.assertEqual(len(pointer_clicks), 1)

    def test_takeover_after_precheck_never_warps_in_either_runner(self):
        for runner in ['fixture', 'textedit']:
            for change in ['focus', 'pointer', 'window']:
                with self.subTest(runner=runner, change=change):
                    self.state = {'frontmostPID': 42, 'pointer': {'x': 2, 'y': 3}, 'windows': [self.window]}
                    self.warps = []; self.native_change = change
                    self.invoke(runner)
                    self.assertEqual(self.warps, [])

    def test_keyboard_takeover_never_restores_even_when_focus_returns(self):
        for runner in ['fixture', 'textedit']:
            for returns in [False, True]:
                with self.subTest(runner=runner, returns=returns):
                    self.state = {'frontmostPID': 42, 'pointer': {'x': 2, 'y': 3}, 'windows': [self.window]}
                    self.warps = []; self.launch_change = 'focus'; self.return_focus = returns
                    self.invoke(runner)
                    self.assertEqual(self.warps, [{'x': 20.0, 'y': 30.0}])


if __name__ == '__main__': unittest.main()
