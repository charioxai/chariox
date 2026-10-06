// MP-08 / MP-11: physical native input/OCR/image oracle; no paid provider claim.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import { createHash } from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import { LinuxOwnedDesktop } from './docker/linux-owned-desktop.mjs';
import { NativeComputer } from './docker/native-computer.mjs';
const root=await mkdtemp(path.join(os.tmpdir(),'culinux-native-state-'));
const desktop=new LinuxOwnedDesktop(root,{environment:{PATH:'/usr/bin:/bin',HOME:root,LANG:'C.UTF-8'}});
try {
  const binding=await desktop.start();
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
  const native=new NativeComputer({placement:'host',binding:()=>desktop.binding(),wakeCapture:event=>wakes.push(event)});
  const command=input=>native.request({op:'input',surface_id:binding.surface_id,generation:binding.generation,input},{});
  const publicText='Hello Grüße 世界😀';
  await command({kind:'text',text:publicText});
  await command({kind:'key',key:'ctrl+s'});await delay(100);
  const {readFile}=await import('node:fs/promises');
  assert.equal((await readFile(file,'utf8')).trim(),publicText);
  for(const state of ['down','up'])await command({kind:'keycode',keycode:38,state});
  await command({kind:'hold',key:'Right',duration_ms:60});
  const screenshot=await native.request({op:'screenshot',surface_id:binding.surface_id,generation:binding.generation},{});
  assert.equal(screenshot.mime_type,'image/png');assert.equal(screenshot.width,1280);
  const ocr=await native.request({op:'ocr',surface_id:binding.surface_id,generation:binding.generation,query:'Hello'},{});
  assert(ocr.targets.length>0);
  const masked=await native.request({op:'ocr',surface_id:binding.surface_id,generation:binding.generation},{values:['synthetic-private-value']});
  assert.equal(masked.text,'[protected]');assert.deepEqual(masked.targets,[]);
  assert.equal(wakes.length,5);
  console.log(JSON.stringify({items:['MP-08','MP-11'],result:'PASS',text_sha256:createHash('sha256').update(publicText).digest('hex'),checks:['Unicode-editor-save','physical-down-up','bounded-hold','protected-PNG','real-OCR-target','registry-mask','capture-wake'],source:process.env.CULINUX_SOURCE,limits:'Native adapter only; no provider MCP receipt, viewer stream, IME preedit, Web/TUI or MP-10 acceptance'}));
}finally{await desktop.stop();await rm(root,{recursive:true,force:true});}
