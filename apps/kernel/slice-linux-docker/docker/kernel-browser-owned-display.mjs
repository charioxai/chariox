// MD-DISPLAY-02/04: private kernel-created X server capability, never DISPLAY adoption.
import {spawn,execFile} from 'node:child_process';
import {promisify} from 'node:util';
const exec=promisify(execFile);
// Xvfb starts its hardware pointer at the screen center. A CDP screenshot may
// temporarily enlarge the page beneath it and synthesize hover mutations. Park
// it outside every owned browser window as Chromium starts, never during
// an input/capture operation and never on an adopted user display.
const parkPointer=`import ctypes as c,ctypes.util
x=c.CDLL(ctypes.util.find_library('X11'))
x.XOpenDisplay.restype=c.c_void_p;x.XOpenDisplay.argtypes=[c.c_char_p]
d=x.XOpenDisplay(None)
if not d: raise RuntimeError('Owned display pointer unavailable')
x.XDefaultRootWindow.restype=c.c_ulong;x.XDefaultRootWindow.argtypes=[c.c_void_p]
x.XWarpPointer.argtypes=[c.c_void_p,c.c_ulong,c.c_ulong,c.c_int,c.c_int,c.c_uint,c.c_uint,c.c_int,c.c_int]
x.XSync.argtypes=[c.c_void_p,c.c_int];x.XCloseDisplay.argtypes=[c.c_void_p]
x.XWarpPointer(d,0,x.XDefaultRootWindow(d),0,0,0,0,2559,1799)
x.XSync(d,0);x.XCloseDisplay(d)
`;
import {randomBytes} from 'node:crypto';
import {writeFile,unlink} from 'node:fs/promises';
import path from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
const owned=new WeakSet();
export const ownsDisplay=d=>owned.has(d)&&Number.isSafeInteger(d.child?.pid)&&d.child.pid>1&&d.child.exitCode===null&&d.child.signalCode===null;
export class OwnedDisplay {
 constructor(root){this.root=root;this.child=null;}
 async start(){
  if(ownsDisplay(this))return this.environment;
  const auth=path.join(this.root,'display.xauth');this.auth=auth;
  const field=b=>{const n=Buffer.alloc(2);n.writeUInt16BE(b.length);return Buffer.concat([n,b])};
  // FamilyWild entry; cookie file and isolated Unix socket stay private to this kernel.
  await writeFile(auth,Buffer.concat([Buffer.from([255,255]),field(Buffer.alloc(0)),field(Buffer.alloc(0)),field(Buffer.from('MIT-MAGIC-COOKIE-1')),field(randomBytes(16))]),{mode:0o600});
  const child=spawn('/usr/bin/Xvfb',['-displayfd','3','-screen','0','2560x1800x24','-nolisten','tcp','-auth',auth],{stdio:['ignore','ignore','ignore','pipe']});this.child=child;
  let failed=false;child.on('error',()=>failed=true);let text='';child.stdio[3]?.on('data',b=>{if(text.length<64)text+=b});
  try{
   for(let n=0;n<100;n++){
    if(failed||child.exitCode!==null||child.signalCode!==null)throw Error('MD-DISPLAY: owned X server unavailable');
    if(/^\d+\n$/.test(text)&&Number(text)<65536){this.environment={DISPLAY:':'+text.trim(),XAUTHORITY:auth};owned.add(this);return this.environment}
    await delay(20);
   }
   throw Error('MD-DISPLAY: owned X server readiness timeout');
  }catch(error){await this.close();throw error}
 }
 async parkPointer(){
  if(!ownsDisplay(this))throw Error('MD-DISPLAY: pointer requires an owned display');
  await exec(process.env.CHARIOX_BROWSER_DISPLAY_PYTHON||'python3',['-c',parkPointer],{env:{...process.env,...this.environment},timeout:2000,maxBuffer:1024});
 }
 async close(){
  owned.delete(this);const child=this.child;this.child=null;
  if(child?.pid!==undefined){if(!Number.isSafeInteger(child.pid)||child.pid<=1)throw Error('MD-DISPLAY: unsafe X server PID');
   if(child.exitCode===null&&child.signalCode===null)child.kill('SIGTERM');
   for(let n=0;n<50&&child.exitCode===null&&child.signalCode===null;n++)await delay(20);
   if(child.exitCode===null&&child.signalCode===null){child.kill('SIGKILL');await new Promise(r=>child.once('exit',r))}
  }
  if(this.auth)await unlink(this.auth).catch(e=>{if(e.code!=='ENOENT')throw e});
 }
}
