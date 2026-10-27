// MP-08/MP-10/MP-11: the actual pool's parent is also the worker's packet
// directory. Browser and Desktop must both use the kernel reader's root.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {LinuxCapture} from './kernel-browser-native.mjs';
import {nativeDesktopWorker} from './kernel-desktop-source.mjs';
test('MP-08/MP-10/MP-11 browser and desktop allocate worker packets in the kernel packet directory',async()=>{
 const root=await mkdtemp(path.join(tmpdir(),'chariox-native-pool-'));
 const desktop=path.join(root,'desktop'),packets=path.join(root,'packets');
 const before={worker:process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER,root:process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT};
 try{
  await mkdir(desktop,{mode:0o700});await mkdir(packets,{mode:0o700});
  process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER='/kernel';process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT=packets;
  const source=new LinuxCapture({display:{root:desktop}}),pool=await source.createRasterPool();
  assert.equal(path.dirname(pool),packets);assert.equal(path.dirname(pool),nativeDesktopWorker().root);
 }finally{
  for(const [key,value] of [['CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER',before.worker],['CHARIOX_BROWSER_DISPLAY_PACKET_ROOT',before.root]])if(value===undefined)delete process.env[key];else process.env[key]=value;
  await rm(root,{recursive:true,force:true});
 }
});
