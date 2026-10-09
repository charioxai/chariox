// MD-DISPLAY-04: repeated failed browser recovery settles its real owned Xvfb.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {HostChromium} from './kernel-browser-process.mjs';
import {OwnedDisplay,ownsDisplay} from './kernel-browser-owned-display.mjs';
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
 assert.equal(new HostChromium('/private').scale,displayGeometry.dpr);
 assert(launchArguments('/private',false,true,2).includes('--force-device-scale-factor=2'));
});
