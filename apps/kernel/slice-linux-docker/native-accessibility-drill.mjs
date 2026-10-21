// MP-08 / MP-11: real scoped AT-SPI actions, stale denial, visible desktop and Vault-fill-only masking.
import assert from 'node:assert/strict';
import { execFile, spawn, spawnSync } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdir, mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import os from 'node:os';import path from 'node:path';
import { isOwnedAlive } from './docker/linux-owned-process.mjs';
import { LinuxOwnedDesktop } from './docker/linux-owned-desktop.mjs';
import { NativeAccessibility } from './docker/native-accessibility.mjs';
import { NativeComputer } from './docker/native-computer.mjs';
const execute=promisify(execFile);
const keyboard=fileURLToPath(new URL('./docker/slice-keyboard.py',import.meta.url));
const value='disposable-native-value-7';
const root=await mkdtemp(path.join(os.tmpdir(),'culinux-atspi-state-'));
let native;
const desktop=new LinuxOwnedDesktop(root,{environment:{PATH:'/usr/bin:/bin',HOME:root,LANG:'C.UTF-8'}});
let cleaning;
const cleanup=()=>cleaning??=(async()=>{try{await native?.close();}finally{const processes=await desktop.ownedProcesses();await desktop.stop();for(const item of processes)assert.equal(await isOwnedAlive(item),false,'owned desktop process must be settled');await rm(root,{recursive:true,force:true});}})();
for(const signal of ['SIGTERM','SIGINT'])process.once(signal,()=>{void cleanup().finally(()=>process.exit(143));});
try {
 const binding=await desktop.start();
 console.log('MP-08 public fixture DISPLAY='+binding.environment.DISPLAY);
 await desktop.launch('/usr/bin/python3',[fileURLToPath(new URL('./native-accessibility-fixture.py',import.meta.url)),root,'plain-entry'],binding.environment);
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
 // Pixel census of the PNG: non-black pixels and magenta glyph pixels inside each named rectangle.
 const census=(image,rects)=>{
  const result=spawnSync('/usr/bin/python3',['-c',"import sys,base64,io,json;from PIL import Image;im=Image.open(io.BytesIO(base64.b64decode(sys.stdin.read()))).convert('RGB');rects=json.loads(sys.argv[1]);px=im.load()\ndef count(r,test):\n x,y,w,h=r;return sum(1 for j in range(max(y,0),min(y+h,im.height)) for i in range(max(x,0),min(x+w,im.width)) if test(px[i,j]))\nprint(json.dumps({'total':count((0,0,im.width,im.height),lambda p:p!=(0,0,0)),**{k:{'lit':count(r,lambda p:p!=(0,0,0)),'magenta':count(r,lambda p:p[0]>200 and p[1]<50 and p[2]>200)} for k,r in rects.items()}}))",JSON.stringify(rects)],{env:binding.environment,input:image.data_base64,encoding:'utf8'});
  assert.equal(result.status,0,'pixel census must run: '+result.stderr);
  return JSON.parse(result.stdout);
 };
 const surface={surface_id:binding.surface_id,generation:binding.generation};
 const fresh=(await accessibility.read({})).tree.nodes;
 const rect=node=>{assert(node?.bounds,'native control bounds required');return node.bounds;};
 const rects={button:rect(fresh.find(node=>node.name==='Changed public action')),password:rect(fresh.find(node=>node.role==='password text')),plain:rect(fresh.find(node=>node.name==='Plain text'))};
 const type=async(vault)=>{
  const [x,y,w,h]=rects.plain;
  await native.request({op:'input',...surface,input:{kind:'click',x:x+Math.floor(w/2),y:y+Math.floor(h/2)}},{});
  if(vault){
   const fillTarget=JSON.parse((await execute('/usr/bin/python3',[keyboard,'secret-target'],{env:binding.environment})).stdout);
   const child=execFile('/usr/bin/python3',[keyboard,'secret',JSON.stringify(fillTarget)],{env:binding.environment});
   child.stdin.end(value);await new Promise((resolve,reject)=>child.on('close',code=>code===0?resolve():reject(Error('Vault keyboard exit '+code))));
  }else await native.request({op:'input',...surface,input:{kind:'text',text:'public typed text'}},{});
  await native.request({op:'input',...surface,input:{kind:'move',x:1270,y:790}},{});
  await delay(1500);
 };
 // No Vault registration: the whole desktop is visible, the public control draws, the password entry shows dots, ordinary text shows.
 await type(false);
 const open=await native.request({op:'screenshot',...surface},{values:[],targets:[],unknown:false});
 assert.equal(open.protected,false,'a screenshot is never a whole-image blackout');
 const visible=census(open,rects);
 assert(visible.total>10000,'desktop pixels stay visible');
 assert(visible.button.lit>100,'public control stays visible');
 assert(visible.password.lit>100,'password entry draws its dots');
 assert(visible.plain.magenta>20,'ordinary entry text is visible before any Vault fill');
 const openOcr=await native.request({op:'ocr',...surface},{values:[],targets:[],unknown:false});
 assert.notEqual(openOcr.text,'[protected]','OCR reads the visible desktop');
 // Real Vault fill into the ordinary entry: only that field is masked in the screenshot and OCR.
 const [px,py,pw,ph]=rects.plain;
 await native.request({op:'input',...surface,input:{kind:'click',x:px+Math.floor(pw/2),y:py+Math.floor(ph/2)}},{});
 await native.request({op:'input',...surface,input:{kind:'key',key:'ctrl+a'}},{});
 await native.request({op:'input',...surface,input:{kind:'key',key:'BackSpace'}},{});
 await type(true);
 const policy={values:[value],targets:[],unknown:false};
 const filled=await native.request({op:'screenshot',...surface},policy);
 assert.equal(filled.protected,false,'a Vault fill masks its field, not the whole image');
 const masked=census(filled,rects);
 assert.equal(masked.plain.magenta,0,'the Vault-filled field shows no glyph pixels');
 assert(masked.button.lit>100,'public control stays visible after a Vault fill');
 assert(masked.password.lit>100,'password dots stay visible after a Vault fill');
 const outside=({total,plain})=>total-plain.lit;
 assert(outside(masked)>outside(visible)*0.95,'only the filled field is covered, not the rest of the desktop');
 const filledOcr=await native.request({op:'ocr',...surface},policy);
 assert.notEqual(filledOcr.text,'[protected]','OCR still reads the unfilled desktop');
 assert(!filledOcr.text.includes(value),'OCR never returns the Vault value');
 console.log(JSON.stringify({items:['MP-08','MP-11'],result:'PASS',checks:['private-bus-scoped-tree','real-password-role','password-text-withheld','foreign-observer-denied','unregistered-app-denied-and-coverage-incomplete','stale-control-denied','fresh-accessibility-effect','public-controls-and-password-dots-visible','screenshot-and-OCR-readable-without-vault-registration','vault-filled-field-only-masked-in-screenshot-and-OCR'],source:process.env.CULINUX_SOURCE,limits:'AT-SPI/native protection only; no paid providers, optimized stream, Web/TUI or MP-10 Path-1 acceptance'}));
}finally{await cleanup();}
