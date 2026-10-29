// MD-DISPLAY-04: repeated failed browser recovery settles its real owned Xvfb.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,readdir,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {LinuxOwnedDesktop} from './linux-owned-desktop.mjs';
import {HostChromium} from './kernel-browser-process.mjs';
import {OwnedDisplay,ownsDisplay} from './kernel-browser-owned-display.mjs';
import {policyNamespaceAvailable} from './kernel-browser-linux.mjs';
import {pageLimit,chromeBandHeight} from './kernel-browser-geometry.mjs';

test('MP-08/MP-10 the kernel browser screen fits its largest page and Chrome band',{skip:process.platform!=='linux'},()=>{
 const host=new HostChromium('/unstarted');
 assert.deepEqual(host.desktop.size,{width:pageLimit.width,height:pageLimit.height+chromeBandHeight});
 assert.equal(host.scale,1,'screen capacity never selects DSF2 rendering');
});
// MP-08/MP-11 union seam (2026-10-15): the bounded TMPDIR (/proc/<kernel pid>/fd/<n>) cannot be opened
// from Chromium's private policy namespace, so every managed launch aborted ("Failed to create socket directory").
const chromium=process.env.CHARIOX_TEST_CHROMIUM;
test('MP-08/MP-11 managed Chromium starts with its singleton in the held private directory',
 {skip:!chromium||process.platform!=='linux'||process.getuid()===0||!policyNamespaceAvailable()?'needs CHARIOX_TEST_CHROMIUM, a non-root Linux user and the policy namespace':false},async()=>{
 const root=await mkdtemp(tmpdir()+'/chariox-managed-launch-');
 const host=new HostChromium(root,{environment:{...process.env,CHARIOX_KERNEL_BROWSER_EXECUTABLE:chromium,CHARIOX_KERNEL_BROWSER_HEADLESS:'1'}});
 try{
  const connection=await host.start();
  assert.equal(host.policy,'managed');
  assert.match((await connection.send('Browser.getVersion')).product,/Chrome/);
  assert((await readdir(root)).some(name=>/^(com\.google\.Chrome|org\.chromium\.Chromium)\./.test(name)),'singleton socket directory in the private directory');
 }finally{await host.stop();await rm(root,{recursive:true,force:true});}
});
test('MP-08/MP-11 native window ownership selects the CDP browser only from verified descendants',async()=>{
 const {ownedBrowserPid}=await import('./kernel-browser-process.mjs');
 assert.equal(typeof ownedBrowserPid,'function');
 const processInfo=[{type:'browser',id:42},{type:'renderer',id:43}],owned=[{pid:41},{pid:42},{pid:43}];
 assert.equal(ownedBrowserPid(processInfo,owned),42,'the supervisor PID41 owns no Chromium window');
 for(const candidates of [[],[{type:'browser',id:99}],[{type:'browser',id:1}],[{type:'browser',id:42},{type:'browser',id:43}]])
  assert.throws(()=>ownedBrowserPid(candidates,owned),/owned browser PID/);
});
test('MP-08/MP-10 production Chromium exposes its shared desktop to native capture',async()=>{
 const root=await mkdtemp(tmpdir()+'/chariox-desktop-native-binding-'),getuid=process.getuid;
 process.getuid=()=>1000;
 const host=new HostChromium(root,{environment:{CHARIOX_KERNEL_BROWSER_EXECUTABLE:'/bin/false'}});
 let linked=false;
 const desktop={runtime:root,binding:()=>({environment:{PATH:process.env.PATH}}),start:async()=>({environment:{PATH:process.env.PATH}}),
  recordOwned:async()=>{linked=host.display===desktop},stop:async()=>{},close:async()=>{}};
 host.desktop=desktop;
 try{await assert.rejects(host.start(),/host Chromium launch failed/);assert(linked,'native capture must use the desktop that launched Chromium');}
 finally{process.getuid=getuid;await host.stop();await rm(root,{recursive:true,force:true});}
});
test('MD-DISPLAY recovering an exited browser closes the previous X server before a failed restart',async()=>{
 const root=await mkdtemp(tmpdir()+'/chariox-md-recovery-');
 const host=new HostChromium(root,{environment:{CHARIOX_KERNEL_BROWSER_EXECUTABLE:'/nonexistent/md-browser',CHARIOX_KERNEL_BROWSER_DISPLAY:'1'}});
 const displays=[];
 try{
  for(let n=0;n<3;n++){
   const display=new OwnedDisplay(root);displays.push(display);await display.start();assert.equal(ownsDisplay(display),true);
   host.display=display;host.child={pid:undefined,exitCode:1,signalCode:null};
   await assert.rejects(host.start(),/install native Chromium|root Chromium/);
   assert.equal(ownsDisplay(display),false,'the old owned X server must settle even when recovery cannot launch Chromium');
   assert.equal(host.display,null);
  }
 }finally{await host.stop();for(const display of displays)await display.close();await rm(root,{recursive:true,force:true})}
});

test('MP-08/MP-10 source browser keeps normal animation cadence rather than outrunning capture',async()=>{
 const {launchArguments}=await import('./kernel-browser-process.mjs');
 assert(!launchArguments('/private',false,true).includes('--disable-frame-rate-limit'));
});

// MP-08/MP-10/MP-11: after an unclean kernel stop Chromium's crash-restore
// bubble covered the page in the owned window, so native attestation refused
// the window until a click dismissed it. The host restores tabs itself.
test('MP-08/MP-10 relaunch after an unclean stop shows no crash-restore bubble',async()=>{
 const {launchArguments}=await import('./kernel-browser-process.mjs');
 for(const display of [false,true]){
  const args=launchArguments('/private',false,display);
  assert(args.includes('--hide-crash-restore-bubble'));
  assert(!args.includes('--disable-session-crashed-bubble'),'obsolete switch: Chromium ignores it');
 }
});

// Chromium 137+ gives WebGL no software fallback on a host without a usable GPU
// unless SwiftShader is opted in; without it getContext("webgl") returns null.
test('kernel browser renders WebGL in software on hosts without a GPU',async()=>{
 const {launchArguments}=await import('./kernel-browser-process.mjs');
 for(const [headless,display] of [[false,false],[false,true],[true,false]])
  assert(launchArguments('/private',headless,display).includes('--enable-unsafe-swiftshader'));
});

// MP-08/MP-10: LCD stripes are tied to a physical panel, whereas these
// captured pixels are shown on arbitrary remote panels at either DPR.
test('MP-08/MP-10 remote display requests grayscale text antialiasing while ordinary browser launch stays native',async()=>{
 const {launchArguments}=await import('./kernel-browser-process.mjs');
 assert(launchArguments('/private',false,true).includes('--disable-lcd-text'));
 assert(!launchArguments('/private',false,false).includes('--disable-lcd-text'));
});

// MP-08/MP-10/MP-11: physical browser scale stays independent of the
// negotiated page raster. Both clients must attest the same bounded window.
test('MP-10 host display renders at the geometry density for negotiated DPR1/2 pages',async()=>{
 const {launchArguments,HostChromium}=await import('./kernel-browser-process.mjs');
 const {displayGeometry}=await import('./kernel-browser-geometry.mjs');
 const chromium=new HostChromium('/private');
 assert.equal(chromium.scale,chromium.desktop?1:displayGeometry.dpr);
 assert(launchArguments('/private',false,true,2).includes('--force-device-scale-factor=2'));
});

// MP-08/MP-10/MP-11 review #1001 P2: an ambient desktop cannot replace a failed private renderer.
test('MP-11 headed mirror refuses an unavailable private display before spawning Chromium', async()=>{
 const root=await mkdtemp(tmpdir()+'/chariox-md-display-refusal-');
 const start=LinuxOwnedDesktop.prototype.start,uid=process.getuid;
 process.getuid=()=>2100; // Supplementary policy seam; no real browser is launched.
 LinuxOwnedDesktop.prototype.start=async()=>{throw Error('MD-DISPLAY: owned X server unavailable')};
 const host=new HostChromium(root,{environment:{CHARIOX_KERNEL_BROWSER_EXECUTABLE:'/bin/true',CHARIOX_KERNEL_BROWSER_MIRROR:'1',CHARIOX_KERNEL_BROWSER_DISPLAY:'0',CHARIOX_KERNEL_BROWSER_HEADLESS:'0',DISPLAY:':77',XAUTHORITY:'/ambient',WAYLAND_DISPLAY:'wayland-0'}});
 try{await assert.rejects(host.start(),/owned X server unavailable/);assert.equal(host.child,null,'ambient display must not launch a browser');assert.equal(host.display,null);}
 finally{LinuxOwnedDesktop.prototype.start=start;process.getuid=uid;await host.stop();await rm(root,{recursive:true,force:true})}
});
// MP-08/MP-10/MP-11 (BROWSER-UX-14): the native band shows no dead-end sign-in chip
// (sign-in is policy-blocked); evidence r4/strip a-ExplicitBrowserSigninUIOnDesktop.png.
test('the owned window has no "Sign in to Chrome?" chip in its band', async () => {
 const {launchArguments}=await import('./kernel-browser-process.mjs');
 assert(launchArguments('/private',false,true).includes('--disable-features=ExplicitBrowserSigninUIOnDesktop,SigninPromoOnAvatarPill'));
});

test('MP-08/MP-10 browser-only stop preserves a healthy shared desktop',async()=>{
 const root=await mkdtemp(tmpdir()+'/chariox-browser-only-stop-');
 const host=new HostChromium(root);let browserStops=0,desktopStops=0;
 const desktop={isLive:()=>true,binding:()=>({}),stopBrowser:async()=>{browserStops++},stop:async()=>{desktopStops++},close:async()=>{desktopStops++}};
 host.desktop=desktop;host.display=desktop;
 try{
  await host.stop(undefined,{preserveDesktop:true});
  assert.equal(browserStops,1);assert.equal(desktopStops,0);assert.equal(host.display,desktop);
 }finally{await host.stop();await rm(root,{recursive:true,force:true});}
});

test('MP-08/MP-10 browser death closes CDP while its supervisor remains alive',async()=>{
 const {spawn}=await import('node:child_process');
 const {once}=await import('node:events');
 const {processIdentity}=await import('./linux-owned-process.mjs');
 const child=spawn(process.execPath,['-e','setTimeout(()=>{},30000)']);
 assert(Number.isSafeInteger(child.pid)&&child.pid>1);
 const host=new HostChromium('/unused');let closed=0;
 const connection={close:async()=>{closed++}};host.connection=connection;
 try{
  const identity=await processIdentity(child.pid);assert(identity);
  host.watchBrowserProcess(connection,identity,{interval:10});
  const exited=once(child,'exit');child.kill('SIGTERM');await exited;
  for(let i=0;i<100&&closed===0;i++)await new Promise(resolve=>setTimeout(resolve,10));
  assert.equal(closed,1,'the actual browser death retires the held CDP pipe');
 }finally{host.stopWatchingBrowser?.();if(child.exitCode===null&&child.signalCode===null)child.kill('SIGTERM');}
});

test('MP-08/MP-10 an old browser death check cannot close a replacement',async()=>{
 const host=new HostChromium('/unused');let resolveAlive,closed=0;
 const old={close:async()=>{closed++}},replacement={close:async()=>{closed++}};
 host.connection=old;
 try{
  host.watchBrowserProcess(old,{pid:42,started:'old'},{interval:1,alive:()=>new Promise(resolve=>{resolveAlive=resolve})});
  while(!resolveAlive)await new Promise(resolve=>setTimeout(resolve,1));
  host.connection=replacement;host.stopWatchingBrowser();resolveAlive(false);
  await new Promise(resolve=>setTimeout(resolve,10));assert.equal(closed,0);
 }finally{host.stopWatchingBrowser?.();}
});
