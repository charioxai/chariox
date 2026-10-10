"""Foreground and pointer ownership for the runner's global pointer movement."""
import json
import subprocess


def run_pointer_script(script):
    return subprocess.run(['osascript', '-l', 'JavaScript', '-e', script],
                          capture_output=True, text=True, check=True, timeout=3).stdout


class PointerLease:
    def __init__(self, desktop, run_script, saved):
        self.desktop, self.run_script = desktop, run_script
        self.saved, self.last = saved, saved
        self.pid = self.window = None
        self.bounds = None
        self.retired = False

    def bind(self, pid, window):
        self.pid, self.window = pid, window
        state = self.desktop()
        matches = [w for w in state['windows'] if w['pid'] == pid and w['windowID'] == window]
        if len(matches) != 1:
            self.retired = True
            raise RuntimeError('fixture-scope-lost')
        self.bounds = matches[0]['bounds']
        self.require()

    def require(self):
        if self.retired:
            raise RuntimeError('fixture-or-pointer-ownership-lost')
        try:
            state = self.desktop()
            if (state['frontmostPID'] != self.pid or not any(w['pid'] == self.pid and w['windowID'] == self.window and w['bounds'] == self.bounds for w in state['windows'])
                    or any(abs(state['pointer'][k] - self.last[k]) >= 1 for k in ['x', 'y'])):
                raise RuntimeError('fixture-or-pointer-ownership-lost')
            return state
        except Exception:
            self.retired = True
            raise

    def _script(self, points, iterations, pause):
        self.require()
        # Both single warps and jitter check inside the native process, directly
        # before every mutation. The Python snapshot is only a preliminary fence.
        return '''ObjC.import('AppKit'); ObjC.import('CoreGraphics');
var pid=__PID__, win=__WIN__, bounds=__BOUNDS__, points=__POINTS__, last=__LAST__, stopped=false;
for(var i=0;i<__COUNT__;i++) {
 var foreground=Number($.NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier);
 var windows=ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo(1,0)));
 var owned=windows.filter(w=>w.kCGWindowNumber===win&&w.kCGWindowOwnerPID===pid&&['X','Y','Width','Height'].every(k=>w.kCGWindowBounds[k]===bounds[k])).length===1;
 var current=$.CGEventGetLocation($.CGEventCreate(null));
 if(foreground!==pid||!owned||Math.abs(current.x-last.x)>=1||Math.abs(current.y-last.y)>=1){stopped=true;break;}
 var next=points[i%points.length];
 if(Number($.CGWarpMouseCursorPosition(next))!==0){stopped=true;break;}
 last=next;__PAUSE__
}
JSON.stringify({last:last,stopped:stopped});'''.replace('__PID__', str(self.pid)).replace('__WIN__', str(self.window)).replace('__BOUNDS__', json.dumps(self.bounds)).replace('__POINTS__', json.dumps(points)).replace('__LAST__', json.dumps(self.last)).replace('__COUNT__', str(iterations)).replace('__PAUSE__', '$.NSThread.sleepForTimeInterval(.002);' if pause else '')

    def warp(self, point):
        script = self._script([point], 1, False)
        try:
            self.finish_jitter(self.run_script(script))
        except Exception:
            self.retired = True
            raise

    def jitter_script(self, points):
        return self._script(points, 400, True)

    def finish_jitter(self, receipt):
        try:
            result = json.loads(receipt)
            self.last = result['last']
            if result['stopped']:
                raise RuntimeError('fixture-or-pointer-ownership-lost')
            self.require()
        except Exception:
            self.retired = True
            raise

    def restore(self):
        if self.retired:
            return
        try:
            self.warp(self.saved)
        except Exception:
            self.retired = True
