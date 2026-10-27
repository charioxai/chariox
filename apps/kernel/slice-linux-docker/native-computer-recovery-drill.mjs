// MP-08 / MP-11: real CDP pipe loss, sandboxed Chromium and native keyboard.
import assert from 'node:assert/strict';
import {mkdtemp, rm} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import {KernelBrowserHost} from './docker/kernel-browser-host.mjs';
import {isOwnedAlive} from './docker/linux-owned-process.mjs';
const root=await mkdtemp(path.join(os.tmpdir(),'culinux-recovery-'));
const host=new KernelBrowserHost(root);
const request=async params=>{const reply=await host.handle({id:'recovery',method:'host.computer',params});assert(reply.ok,reply.error?.message??'MP-08 Computer refused');return reply.result;};
try {
  const before=await request({op:'start'});
  await request({op:'input',...before,input:{kind:'keycode',keycode:38,state:'down'},observed_by:'terminal:recovery'});
  const oldKeyboard=host.nativeComputer.keyboard.child;
  const oldDesktop=await host.chromium.desktop.ownedProcesses();
  await host.chromium.connection.close();
  assert.equal(host.chromium.child.exitCode,null,'MP-08 lost CDP while Chromium remains alive');
  const after=await request({op:'start'});
  assert.notEqual(after.generation,before.generation);
  assert(oldKeyboard.exitCode!==null||oldKeyboard.signalCode!==null,'MP-11 old helper reaped');
  assert.equal(host.nativeComputer.held.size,0);
  for(const process of oldDesktop)assert.equal(await isOwnedAlive(process),false,'MP-11 old desktop settled');
  for(const state of ['down','up'])await request({op:'input',...after,input:{kind:'keycode',keycode:38,state},observed_by:'terminal:recovery'});
  assert.notEqual(host.nativeComputer.keyboard.child.pid,oldKeyboard.pid);
  console.log(JSON.stringify({items:['MP-08','MP-11'],result:'PASS',checks:['real-CDP-loss-live-Chromium','old-keyboard-and-held-keys-retired','old-desktop-settled','fresh-desktop-physical-input'],limits:'Real native/browser recovery regression; full provider and TUI acceptance is separate'}));
} finally {await host.stop();await rm(root,{recursive:true,force:true});}
