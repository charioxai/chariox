// MP-08 / MP-11: real scoped AT-SPI actions, stale denial and protected pixels.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import os from 'node:os';import path from 'node:path';
import { isOwnedAlive } from './docker/linux-owned-process.mjs';
import { LinuxOwnedDesktop } from './docker/linux-owned-desktop.mjs';
import { NativeAccessibility } from './docker/native-accessibility.mjs';
import { NativeComputer } from './docker/native-computer.mjs';
const root=await mkdtemp(path.join(os.tmpdir(),'culinux-atspi-state-'));
let native;
const desktop=new LinuxOwnedDesktop(root,{environment:{PATH:'/usr/bin:/bin',HOME:root,LANG:'C.UTF-8'}});
let cleaning;
const cleanup=()=>cleaning??=(async()=>{try{await native?.close();}finally{const processes=await desktop.ownedProcesses();await desktop.stop();for(const item of processes)assert.equal(await isOwnedAlive(item),false,'owned desktop process must be settled');await rm(root,{recursive:true,force:true});}})();
for(const signal of ['SIGTERM','SIGINT'])process.once(signal,()=>{void cleanup().finally(()=>process.exit(143));});
try {
 const binding=await desktop.start();
 console.log('MP-08 public fixture DISPLAY='+binding.environment.DISPLAY);
 await desktop.launch('/usr/bin/python3',[fileURLToPath(new URL('./native-accessibility-fixture.py',import.meta.url)),root],binding.environment);
 const accessibility=new NativeAccessibility({binding:()=>desktop.binding()});
 let snapshot;
 for(let n=0;n<50;n++){snapshot=await accessibility.snapshot('agent:a',{});if(snapshot.nodes.some(node=>node.name==='Public action'))break;await delay(100);}
 if(!snapshot.nodes.some(node=>node.role==='password text'))console.log('MP-08 fixture discovery status',JSON.stringify({nodes:snapshot.nodes.map(node=>node.role),fallback:snapshot.fallback,complete:snapshot.complete}));
 assert(snapshot.nodes.some(node=>node.role==='password text'),'real password role required');
 assert(!JSON.stringify(snapshot).includes('synthetic-password-canary'));
 const target=snapshot.nodes.find(node=>node.name==='Public action');assert(target);
 await assert.rejects(accessibility.action('agent:b',{target_id:target.target_id,tree_revision:snapshot.tree_revision,action:target.actions[0]},{}));
 await writeFile(path.join(root,'change'),'change');await delay(200);
 await assert.rejects(accessibility.action('agent:a',{target_id:target.target_id,tree_revision:snapshot.tree_revision,action:target.actions[0]},{}),error=>error.code==='user_domain_stale_reference');
 let applied=false;
 for(let attempt=0;attempt<5;attempt++){
  await delay(100);snapshot=await accessibility.snapshot('agent:a',{});
  const fresh=snapshot.nodes.find(node=>node.name==='Changed public action');assert(fresh);
  try{await accessibility.action('agent:a',{target_id:fresh.target_id,tree_revision:snapshot.tree_revision,action:fresh.actions[0]},{});applied=true;break;}
  catch(error){if(error.code!=='user_domain_stale_reference')throw error;}
 }
 assert(applied,'rediscovery must find a settled fresh target');
 await delay(100);assert.equal(await readFile(path.join(root,'clicked'),'utf8'),'public effect');
 // Sharing the private display/bus is insufficient to acquire app scope.
 const foreignRoot=path.join(root,'foreign');await mkdir(foreignRoot);
 const foreign=spawn('/usr/bin/python3',[fileURLToPath(new URL('./native-accessibility-fixture.py',import.meta.url)),foreignRoot],{env:binding.environment,stdio:'ignore'});
 foreign.on('error',()=>{});
 try{
  await delay(300);
  const scoped=await accessibility.read({});
  assert(!scoped.tree.nodes.some(node=>node.pid===foreign.pid),'unregistered app must be excluded');
  assert.equal(scoped.tree.complete,false,'foreign visible app makes pixel coverage incomplete');
 }finally{await desktop.recordOwned(foreign);}
 native=new NativeComputer({placement:'host',binding:()=>desktop.binding()});
 const image=await native.request({op:'screenshot',surface_id:binding.surface_id,generation:binding.generation},{});
 assert.equal(image.protected,true);
 const pixels=spawnSync('/usr/bin/python3',['-c',"import sys,base64,io;from PIL import Image;im=Image.open(io.BytesIO(base64.b64decode(sys.stdin.read()))).convert('RGB');assert im.getbbox() is None"],{env:binding.environment,input:image.data_base64});
 assert.equal(pixels.status,0,'protected PNG must contain only black pixels');
 const ocr=await native.request({op:'ocr',surface_id:binding.surface_id,generation:binding.generation},{});
 assert.equal(ocr.text,'[protected]');
 console.log(JSON.stringify({items:['MP-08','MP-11'],result:'PASS',checks:['private-bus-scoped-tree','real-password-role','password-text-withheld','foreign-observer-denied','unregistered-app-denied-and-coverage-incomplete','stale-control-denied','fresh-accessibility-effect','password-pixels-and-OCR-masked'],source:process.env.CULINUX_SOURCE,limits:'AT-SPI/native protection only; no paid providers, optimized stream, Web/TUI or MP-10 Path-1 acceptance'}));
}finally{await cleanup();}
