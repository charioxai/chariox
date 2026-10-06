// MP-08 / MP-11: real scoped AT-SPI actions, stale denial and protected pixels.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import os from 'node:os';import path from 'node:path';
import { LinuxOwnedDesktop } from './docker/linux-owned-desktop.mjs';
import { NativeAccessibility } from './docker/native-accessibility.mjs';
import { NativeComputer } from './docker/native-computer.mjs';
const root=await mkdtemp(path.join(os.tmpdir(),'culinux-atspi-state-'));
const desktop=new LinuxOwnedDesktop(root,{environment:{PATH:'/usr/bin:/bin',HOME:root,LANG:'C.UTF-8'}});
try {
 const binding=await desktop.start();
 await desktop.launch('/usr/bin/python3',[fileURLToPath(new URL('./native-accessibility-fixture.py',import.meta.url)),root],binding.environment);
 const accessibility=new NativeAccessibility({binding:()=>desktop.binding()});
 let snapshot;
 for(let n=0;n<50;n++){snapshot=await accessibility.snapshot('agent:a',{});if(snapshot.nodes.some(node=>node.name==='Public action'))break;await delay(100);}
 assert(snapshot.nodes.some(node=>node.role==='password text'),'real password role required');
 assert(!JSON.stringify(snapshot).includes('synthetic-password-canary'));
 const target=snapshot.nodes.find(node=>node.name==='Public action');assert(target);
 await assert.rejects(accessibility.action('agent:b',{target_id:target.target_id,tree_revision:snapshot.tree_revision,action:target.actions[0]},{}));
 await writeFile(path.join(root,'change'),'change');await delay(200);
 await assert.rejects(accessibility.action('agent:a',{target_id:target.target_id,tree_revision:snapshot.tree_revision,action:target.actions[0]},{}),/stale/);
 snapshot=await accessibility.snapshot('agent:a',{});
 const fresh=snapshot.nodes.find(node=>node.name==='Changed public action');assert(fresh);
 await accessibility.action('agent:a',{target_id:fresh.target_id,tree_revision:snapshot.tree_revision,action:fresh.actions[0]},{});
 await delay(100);assert.equal(await readFile(path.join(root,'clicked'),'utf8'),'public effect');
 const native=new NativeComputer({placement:'host',binding:()=>desktop.binding()});
 const image=await native.request({op:'screenshot',surface_id:binding.surface_id,generation:binding.generation},{});
 assert.equal(image.protected,true);
 const ocr=await native.request({op:'ocr',surface_id:binding.surface_id,generation:binding.generation},{});
 assert.equal(ocr.text,'[protected]');
 console.log(JSON.stringify({items:['MP-08','MP-11'],result:'PASS',checks:['private-bus-scoped-tree','real-password-role','password-text-withheld','foreign-observer-denied','stale-control-denied','fresh-accessibility-effect','password-pixels-and-OCR-masked'],source:process.env.CULINUX_SOURCE,limits:'AT-SPI/native protection only; no paid providers, optimized stream, Web/TUI or MP-10 Path-1 acceptance'}));
}finally{await desktop.stop();await rm(root,{recursive:true,force:true});}
