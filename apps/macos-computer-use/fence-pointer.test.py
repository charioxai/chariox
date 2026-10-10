import unittest
import json
import subprocess
from fence_pointer import PointerLease

class PointerTests(unittest.TestCase):
    def setUp(self):
        self.state={'frontmostPID':42,'pointer':{'x':2,'y':3},'windows':[{'pid':42,'windowID':7,'bounds':{'X':0,'Y':0,'Width':100,'Height':100}}]}
        self.warps=[]
        def warp(script):
            result=self.execute_script(script,self.state)
            self.warps.extend(result['warps'])
            self.state['pointer']=result['point']
            return json.dumps(result['result'])
        self.lease=PointerLease(lambda:self.state,warp,dict(self.state['pointer']))
        self.lease.bind(42,7)
    def execute_script(self,script,state):
        runner=r"""const vm=require('node:vm'),fs=require('node:fs');
const {script,state}=JSON.parse(fs.readFileSync(0,'utf8'));let point=state.pointer,warps=[];
const $={NSWorkspace:{sharedWorkspace:{frontmostApplication:{processIdentifier:state.frontmostPID}}},
 CGWindowListCopyWindowInfo:()=>state.windows.map(w=>({kCGWindowNumber:w.windowID,kCGWindowOwnerPID:w.pid,kCGWindowBounds:w.bounds})),
 CGEventCreate:()=>null,CGEventGetLocation:()=>point,CGWarpMouseCursorPosition:p=>{point=p;warps.push(p);return 0;},NSThread:{sleepForTimeInterval(){}}};
const result=JSON.parse(vm.runInNewContext(script,{ObjC:{import(){},deepUnwrap:x=>x,castRefToObject:x=>x},$,JSON,Number,Math}));
process.stdout.write(JSON.stringify({result,point,warps}));"""
        return json.loads(subprocess.check_output(['node','-e',runner],input=json.dumps({'script':script,'state':state}),text=True))
    def test_actual_single_warp_refuses_takeover_after_python_precheck(self):
        for change in ['focus','pointer','window']:
            self.setUp()
            def launch(script):
                if change=='focus':self.state['frontmostPID']=99
                elif change=='pointer':self.state['pointer']={'x':40,'y':50}
                else:self.state['windows'][0]['windowID']=8
                result=self.execute_script(script,self.state)
                self.warps.extend(result['warps'])
                return json.dumps(result['result'])
            self.lease.run_script=launch
            with self.assertRaises(RuntimeError):self.lease.warp({'x':4,'y':5})
            self.assertEqual(self.warps,[])
            self.assertTrue(self.lease.retired)
    def test_focus_loss_prevents_warp_jitter_launch_and_restore(self):
        self.state['frontmostPID']=99
        with self.assertRaises(RuntimeError):self.lease.warp({'x':4,'y':5})
        with self.assertRaises(RuntimeError):self.lease.jitter_script([{'x':4,'y':5}])
        self.lease.restore();self.assertEqual(self.warps,[])
    def test_human_pointer_move_prevents_restore(self):
        self.lease.warp({'x':4,'y':5})
        self.state['pointer']={'x':20,'y':30};self.lease.restore()
        self.assertEqual(len(self.warps),1)
    def test_window_replacement_prevents_jitter_launch(self):
        self.state['windows'][0]['windowID']=8
        with self.assertRaises(RuntimeError):self.lease.jitter_script([{'x':4,'y':5}])
        self.assertEqual(self.warps,[])
    def test_jitter_takeover_blocks_followup_and_restore(self):
        self.state['pointer']={'x':4,'y':5};self.state['frontmostPID']=99
        with self.assertRaises(RuntimeError):self.lease.finish_jitter('{"last":{"x":4,"y":5},"stopped":true}')
        self.lease.restore();self.assertEqual(self.warps,[])
    def test_reported_takeover_remains_retired_after_focus_returns(self):
        self.state['pointer']={'x':4,'y':5}
        with self.assertRaises(RuntimeError):self.lease.finish_jitter('{"last":{"x":4,"y":5},"stopped":true}')
        self.state['frontmostPID']=42
        self.lease.restore()
        self.assertEqual(self.warps,[])
    def test_failed_require_remains_retired_after_pointer_returns(self):
        self.state['pointer']={'x':20,'y':30}
        with self.assertRaises(RuntimeError):self.lease.require()
        self.state['pointer']={'x':2,'y':3}
        self.lease.restore()
        self.assertEqual(self.warps,[])
    def test_actual_jitter_script_stops_before_next_warp_on_takeover(self):
        script=self.lease.jitter_script([{'x':4,'y':5},{'x':6,'y':7}])
        runner=r'''const vm=require('node:vm'), fs=require('node:fs');
const script=JSON.parse(fs.readFileSync(0,'utf8'));
let point={x:2,y:3}, warps=0;
const app={processIdentifier:42};
const $={NSWorkspace:{sharedWorkspace:{frontmostApplication:app}},
 CGWindowListCopyWindowInfo:()=>[{kCGWindowNumber:7,kCGWindowOwnerPID:42,kCGWindowBounds:{X:0,Y:0,Width:100,Height:100}}],
 CGEventCreate:()=>null, CGEventGetLocation:()=>point,
 CGWarpMouseCursorPosition:p=>{point=p;warps++;return 0;},
 NSThread:{sleepForTimeInterval:()=>{app.processIdentifier=99;}}};
const result=vm.runInNewContext(script,{ObjC:{import(){},deepUnwrap:x=>x,castRefToObject:x=>x},$,JSON,Number,Math});
process.stdout.write(JSON.stringify({warps,result:JSON.parse(result)}));'''
        result=json.loads(subprocess.check_output(['node','-e',runner],input=json.dumps(script),text=True))
        self.assertEqual(result['warps'],1)
        self.assertTrue(result['result']['stopped'])
if __name__=='__main__':unittest.main()
