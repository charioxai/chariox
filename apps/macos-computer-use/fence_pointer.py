"""Foreground and pointer ownership for the runner's global pointer movement."""
import json

class PointerLease:
    def __init__(self, desktop, warp, saved):
        self.desktop, self.warp_native = desktop, warp
        self.saved, self.last = saved, saved
        self.pid = self.window = None
        self.bounds = None

    def bind(self, pid, window):
        self.pid, self.window = pid, window
        state = self.desktop()
        matches = [w for w in state['windows'] if w['pid'] == pid and w['windowID'] == window]
        if len(matches) != 1: raise RuntimeError('fixture-scope-lost')
        self.bounds = matches[0]['bounds']
        self.require()

    def require(self):
        state = self.desktop()
        if (state['frontmostPID'] != self.pid or not any(w['pid'] == self.pid and w['windowID'] == self.window and w['bounds'] == self.bounds for w in state['windows'])
                or any(abs(state['pointer'][k] - self.last[k]) >= 1 for k in ['x', 'y'])):
            raise RuntimeError('fixture-or-pointer-ownership-lost')
        return state

    def warp(self, point):
        self.require()
        self.warp_native(point)
        self.last = point

    def jitter_script(self, points):
        self.require()
        # Every warp has a fresh foreground, window identity and pointer check.
        return '''ObjC.import('AppKit'); ObjC.import('CoreGraphics');
var pid=__PID__, win=__WIN__, bounds=__BOUNDS__, points=__POINTS__, last=__LAST__, stopped=false;
for(var i=0;i<400;i++) {
 var foreground=Number($.NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier);
 var windows=ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo(1,0)));
 var owned=windows.some(w=>w.kCGWindowNumber===win&&w.kCGWindowOwnerPID===pid&&['X','Y','Width','Height'].every(k=>w.kCGWindowBounds[k]===bounds[k]));
 var current=$.CGEventGetLocation($.CGEventCreate(null));
 if(foreground!==pid||!owned||Math.abs(current.x-last.x)>=1||Math.abs(current.y-last.y)>=1){stopped=true;break;}
 last=points[i%2];$.CGWarpMouseCursorPosition(last);$.NSThread.sleepForTimeInterval(.002);
}
JSON.stringify({last:last,stopped:stopped});'''.replace('__PID__',str(self.pid)).replace('__WIN__',str(self.window)).replace('__BOUNDS__',json.dumps(self.bounds)).replace('__POINTS__',json.dumps(points)).replace('__LAST__',json.dumps(self.last))

    def finish_jitter(self, receipt):
        result = json.loads(receipt)
        self.last = result['last']
        if result['stopped']: raise RuntimeError('fixture-or-pointer-ownership-lost')
        self.require()

    def restore(self):
        try: self.warp(self.saved)
        except RuntimeError: pass
