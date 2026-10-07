// MP-08 / MP-11: physical native input/OCR/image oracle; no paid provider claim.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import { createHash } from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import { isOwnedAlive } from './docker/linux-owned-process.mjs';
import { LinuxOwnedDesktop } from './docker/linux-owned-desktop.mjs';
import { NativeComputer } from './docker/native-computer.mjs';
const root=await mkdtemp(path.join(os.tmpdir(),'culinux-native-state-'));
let native;let keyboardProcess;
const desktop=new LinuxOwnedDesktop(root,{environment:{PATH:'/usr/bin:/bin',HOME:root,LANG:'C.UTF-8'}});
let cleaning;
const cleanup=()=>cleaning??=(async()=>{console.error('MP-11 cleanup native begin');try{await native?.close();console.error('MP-11 cleanup native settled');if(keyboardProcess){assert(keyboardProcess.exitCode!==null || keyboardProcess.signalCode!==null,'warm keyboard must be reaped');}}finally{const processes=await desktop.ownedProcesses();await desktop.stop();for(const item of processes)assert.equal(await isOwnedAlive(item),false,'owned desktop process must be settled');console.error('MP-11 cleanup desktop settled');await rm(root,{recursive:true,force:true});console.error('MP-11 cleanup scratch removed');}})();
for(const signal of ['SIGTERM','SIGINT'])process.once(signal,()=>{void cleanup().finally(()=>process.exit(143));});
try {
  const binding=await desktop.start();
  console.log('MP-08 public fixture DISPLAY='+binding.environment.DISPLAY);
  const file=path.join(root,'public.txt');await writeFile(file,'');
  await desktop.launch('mousepad',[file],binding.environment);
  let window;
  for(let n=0;n<100;n++){
    const search=spawnSync('xdotool',['search','--onlyvisible','--class','mousepad'],{env:binding.environment,encoding:'utf8'});
    window=search.stdout?.trim().split('\n')[0];if(window)break;await delay(50);
  }
  assert(window,'native editor window required');
  assert.equal(spawnSync('xdotool',['windowactivate','--sync',window],{env:binding.environment}).status,0);
  const wakes=[];
  native=new NativeComputer({placement:'host',binding:()=>desktop.binding(),wakeCapture:event=>wakes.push(event)});
  const command=input=>native.request({op:'input',surface_id:binding.surface_id,generation:binding.generation,input},{});
  const publicText='Hello Grüße 世界😀';
  await command({kind:'text',text:publicText});
  await command({kind:'key',key:'ctrl+s'});await delay(100);
  const {readFile}=await import('node:fs/promises');
  assert.equal((await readFile(file,'utf8')).trim(),publicText);
  for(const state of ['down','up'])await command({kind:'keycode',keycode:38,state});
  keyboardProcess=native.keyboard?.child;assert(keyboardProcess,'warm physical channel remains owned after key-up');
  await command({kind:'hold',key:'Right',duration_ms:60});
  const screenshot=await native.request({op:'screenshot',surface_id:binding.surface_id,generation:binding.generation},{});
  assert.equal(screenshot.mime_type,'image/png');assert.equal(screenshot.width,1280);
  const ocr=await native.request({op:'ocr',surface_id:binding.surface_id,generation:binding.generation,query:'Hello'},{});
  assert(ocr.targets.length>0);
  const masked=await native.request({op:'ocr',surface_id:binding.surface_id,generation:binding.generation},{values:['synthetic-private-value']});
  assert.equal(masked.text,'[protected]');assert.deepEqual(masked.targets,[]);
  const clipboard='public clipboard';await command({kind:'clipboard_write',text:clipboard});
  assert.equal((await native.request({op:'clipboard_read',surface_id:binding.surface_id,generation:binding.generation},{})).text,clipboard);
  const cancellation=new AbortController();
  const hold=native.request({op:'input',surface_id:binding.surface_id,generation:binding.generation,input:{kind:'hold',key:'Right',duration_ms:10000}}, {}, {signal:cancellation.signal});
  await delay(300);cancellation.abort();await assert.rejects(hold,/cancelled/);
  const released=spawnSync('/usr/bin/python3',['-c',"from Xlib import display;d=display.Display();keys=d.query_keymap();assert not keys[114//8] & (1 << (114%8));d.close()"],{env:binding.environment});
  assert.equal(released.status,0,'cancelled bounded key released before next actor');
  assert.equal(wakes.length,6);
  console.log(JSON.stringify({items:['MP-08','MP-11'],result:'PASS',text_sha256:createHash('sha256').update(publicText).digest('hex'),checks:['Unicode-editor-save','physical-down-up','bounded-hold','protected-PNG','real-OCR-target','registry-mask','capture-wake','owned-clipboard','cancelled-hold-key-released'],source:process.env.CULINUX_SOURCE,limits:'Native adapter only; no provider MCP receipt, viewer stream, IME preedit, Web/TUI or MP-10 acceptance'}));
}finally{await cleanup();}
