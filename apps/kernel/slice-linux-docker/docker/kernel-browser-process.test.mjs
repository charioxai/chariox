// MD-DISPLAY-04: repeated failed browser recovery settles its real owned Xvfb.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {LinuxOwnedDesktop} from './linux-owned-desktop.mjs';
import {HostChromium} from './kernel-browser-process.mjs';
import {OwnedDisplay,ownsDisplay} from './kernel-browser-owned-display.mjs';
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
 try{await assert.rejects(host.start(),/did not become ready/);assert(linked,'native capture must use the desktop that launched Chromium');}
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
 assert(launchArguments('/private',false,true,false,2).includes('--force-device-scale-factor=2'));
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
