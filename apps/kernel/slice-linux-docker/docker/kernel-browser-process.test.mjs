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
