// L3 stage 2b: worker display leases (MP-08/MP-10/MP-11).
import test from 'node:test';import assert from 'node:assert/strict';
import {mkdtempSync,writeFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';import path from 'node:path';
import {NativeWorkerControl,retireNativeStates} from './kernel-browser-native-worker.mjs';
import {DisplayStream,PortableEncoder,directLease} from './kernel-browser-display.mjs';
import {displayMaskRegions,displayUnprotected} from './kernel-browser-pixels.mjs';

const root=mkdtempSync(path.join(process.env.TMPDIR??tmpdir(),'lease-packets-'));
process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT=root;
process.env.CHARIOX_BROWSER_DISPLAY_FRAMES=path.join(root,'frames.sock');
test.after(()=>rmSync(root,{recursive:true,force:true}));
let names=0;
const packet=()=>{const name=(++names).toString(16).padStart(32,'0')+'.json';writeFileSync(path.join(root,name),'x');return {name,length:1};};
const reply=(sequence,key=false)=>({backend:'native-openh264',whole:true,revision:sequence,stripes:[{row:0,y:0,height:32,codec:'avc1.420033',key,sequence,reference_sequence:key?null:sequence-1,length:1}],packet:packet()});
function rig({targets=false}={}){
 const writes=[],manual=[],retires=[];
 const bridge=new NativeWorkerControl({stdin:{write(bytes){for(const line of bytes.trim().split('\n'))writes.push(JSON.parse(line))}},stdio:[,,,{write(bytes){retires.push(Number(bytes))},writableLength:0}]},()=>{});
 const raw=(serial,{auto,regions}={})=>{
  const r={width:64,height:32,serial,nativeEncode:async()=>manual.shift(),nativeArm:record=>bridge.arm(record),nativeAutoCurrent:()=>bridge.auto,nativeRetireState:encoder=>bridge.retireState(encoder),nativeRetire:()=>{},nativeDelivered:()=>{},nativeGrant:lease=>bridge.grant(lease),nativeUntargeted:()=>!targets};
  if(regions)r[displayMaskRegions]=regions;else r[displayUnprotected]=true;
  if(auto!==undefined){bridge.autoEntry(serial);r.auto={generation:auto,take:()=>bridge.takeAuto(serial)};}
  return r;
 };
 const autoReply=(serial,generation,value)=>bridge.autoReceive({auto:serial,generation,length:1},Buffer.from(JSON.stringify(value)));
 return {writes,manual,retires,bridge,raw,autoReply,encoder:new PortableEncoder()};
}
// One requested key frame arms the record; the next readback is auto-encoded.
async function autoFrame(r){
 r.manual.push(reply(1,true));await r.encoder.encode(r.raw(1),8e6,true,'avc1.420033');
 r.autoReply(2,1,reply(2));return r.encoder.encode(r.raw(2,{auto:1}),8e6,false,'avc1.420033');
}

test('L3 only an unprotected auto frame with no protected target anywhere offers a lease',async()=>{
 const r=rig();const encoded=await autoFrame(r);
 assert.ok(encoded.lease&&encoded.lease.current());
 assert.deepEqual([encoded.lease.generation,encoded.lease.chain],[1,2]);
 const template={kind:'video',subscription_id:'s'};
 assert.deepEqual(encoded.lease.grant({generation:1,sequence:7,chain:2,template}),{token:r.bridge.token,generation:1});
 assert.match(r.bridge.token,/^[a-f0-9]{32}$/);
 assert.deepEqual(r.writes.at(-1),{lease:{generation:1,sequence:7,chain:2,template}});
 assert.equal(encoded.lease.grant({generation:1,sequence:8,chain:3,template}),null,'one lease at a time');
 // A requested (non-auto) frame and a masked frame never offer one.
 r.manual.push(reply(3));assert.equal((await r.encoder.encode(r.raw(3),8e6,false,'avc1.420033')).lease,undefined);
 r.manual.push(reply(4));assert.equal((await r.encoder.encode(r.raw(4,{regions:[{x:0,y:0,width:2,height:2}]}),8e6,false,'avc1.420033')).lease,undefined);
 const targeted=rig({targets:true});
 assert.equal((await autoFrame(targeted)).lease.current(),false,'a protected target elsewhere on the page refuses the grant');
});

test('MP-11 every retire event writes the retire pipe synchronously and spends the grant',async()=>{
 const a=rig(),b=rig();const offer=(await autoFrame(a)).lease;await autoFrame(b);
 assert.ok(a.bridge.grant({generation:1,sequence:1,chain:2,template:{kind:'video'}}));
 retireNativeStates();
 assert.deepEqual([a.retires,b.retires],[[1],[1]],'every live worker, before the event continues');
 assert.equal(a.bridge.lease,null);assert.equal(offer.current(),false);
 assert.equal(offer.grant({generation:1,sequence:2,chain:2,template:{kind:'video'}}),null);
 // A worker-side end spends that generation: the next record is a new one.
 const c=rig();await autoFrame(c);c.bridge.ended(1);assert.equal(c.bridge.auto,null);
 c.bridge.retiredAt=-Infinity;
 assert.equal(c.bridge.arm({encoder:'x',bitrate:8e6,reduced:false,until_ms:1}).generation,2);
 // A pipe that cannot take the write synchronously fails closed.
 const killed=[];const d=new NativeWorkerControl({stdin:{write(){}},stdio:[,,,{write(){},writableLength:3}],kill:s=>killed.push(s)},()=>{});
 d.arm({encoder:'x',bitrate:8e6,reduced:false,until_ms:1});d.retireState();
 assert.deepEqual(killed,['SIGKILL']);assert.equal(d.closed,true);
});

function stream(kind,leases=true,leaseStreak=1,now=()=>0){
 const retired=[];
 const encoder={close:async()=>{},nativeSession:'motion',nativeRetireState:e=>retired.push(e)};
 const s=new DisplayStream({subscription_id:'s',tab_id:'t',device_scale_factor:1,bitrate:8000000,codec:'avc1.420033',dependencies:true},{encoder,now,wait:async()=>{},leases,leaseStreak});
 if(kind)s.regionTarget={kind};
 return {s,retired};
}
const offer=(granted,current=()=>true)=>({generation:3,chain:9,current,renew:()=>{},grant:lease=>{granted.push(lease);return {token:'a'.repeat(32),generation:3};}});
const source=(lease,push=true,key=false)=>({motion:true,generation:1,width:1280,height:800,data_base64:'a'+names,push_delivery:push,encoded:{key,packet:packet(),...(lease?{lease}:{})}});

test('L3 a push frame carries the private grant; Node adopts the kernel sequence after the lease',async()=>{
 const {s,retired}=stream();const granted=[];
 try{
  const first=await s.frame(source(offer(granted),true,true),'d',0);
  assert.equal(JSON.parse(JSON.stringify(first))[directLease],undefined,'never serialized');
  assert.deepEqual(first[directLease](),{token:'a'.repeat(32),generation:3});
  const {key,sequence,native_packet,...template}=JSON.parse(JSON.stringify(first));
  assert.deepEqual(granted,[{generation:3,sequence:1,chain:9,template}]);
  assert.equal(template.native_packet,undefined);
  // The worker sent frames 2..6; the next credit reaching Node ends the lease.
  const next=await s.frame(source(null,true,true),'d',6);
  assert.equal(next.sequence,7);assert.deepEqual(retired,['motion']);
  assert.equal((await s.frame(source(offer([]),false),'d',7))[directLease],undefined,'pull credits never lease');
 }finally{await s.close()}
 for(const kind of ['region','browser_auxiliary']){
  const {s:region}=stream(kind);
  try{assert.equal((await region.frame(source(offer([]),true,true),'d',0))[directLease],undefined,`${kind} has per-frame-only checks`);}finally{await region.close()}
 }
});

test('MP-11 the lease renews only while the frame binding still holds',async()=>{
 const {s,retired}=stream('visible_page');let bound=true,renewed=0;
 try{
  const lease=offer([]);lease.renew=()=>renewed++;
  const first=await s.frame(source(lease,true,true),'d',0,async()=>true,()=>bound);
  assert.ok(first[directLease]());
  await new Promise(resolve=>setTimeout(resolve,130));
  assert.ok(renewed>=1&&!retired.length);
  bound=false;await new Promise(resolve=>setTimeout(resolve,130));
  assert.deepEqual(retired,['motion'],'a binding that no longer holds retires at once');
  bound=true;const stale=await s.frame(source(offer([],()=>false)),'d',1,async()=>true,()=>bound);
  assert.equal(stale[directLease](),null,'a retired record is never granted');
 }finally{await s.close()}
});

test('L3 a retire storm neither re-arms nor grants until the retires have been quiet',()=>{
 const r=rig(),record={encoder:'e',bitrate:8e6,reduced:false,until_ms:Date.now()+1e6};
 assert.ok(r.bridge.arm(record),'the first record arms at once');
 r.bridge.retireState();
 assert.equal(r.bridge.arm(record),null,'a record right after a retire would be retired again');
 r.bridge.retireState();
 assert.equal(r.bridge.arm(record),null,'a retire with no record still extends the quiet window');
 r.bridge.retiredAt-=NativeWorkerControl.retireQuietMs;
 assert.ok(r.bridge.arm(record),'quiet long enough: the next frame arms again');
 r.bridge.ended(r.bridge.auto.generation);
 assert.equal(r.bridge.arm(record),null,'a refused grant counts as a retire');
});

test('L3 Node offers no lease when leases are disabled',async()=>{
 const {s}=stream(undefined,false);const granted=[];
 try{
  const first=await s.frame(source(offer(granted),true,true),'d',0);
  assert.equal(first[directLease],undefined);assert.deepEqual(granted,[]);
 }finally{await s.close()}
});

test('L3 only sustained motion is offered a lease; a lease end starts the count again',async()=>{
 let clock=0;const defaults=new DisplayStream({subscription_id:'s',tab_id:'t',device_scale_factor:1,bitrate:8000000,codec:'avc1.420033',dependencies:true},{encoder:{close:async()=>{},nativeSession:'motion',nativeRetireState:()=>{}},now:()=>clock,wait:async()=>{}});
 const granted=[];
 try{
  let sequence=0,offered=0;
  const frame=async()=>{const f=await defaults.frame(source(offer(granted),true,true),'d',sequence++);if(f[directLease])offered++;return f;};
  for(let n=0;n<29;n++){clock+=16;await frame();}
  assert.equal(offered,0,'29 consecutive frames are not yet sustained motion (a typed key, a scroll notch)');
  clock+=16;await frame();assert.equal(offered,1,'the 30th consecutive frame is');
  clock+=500;await frame();assert.equal(offered,1,'a pause restarts the count');
  for(let n=0;n<29;n++){clock+=16;await frame();}
  assert.equal(offered,2,'30 consecutive frames after the pause');
  defaults.adoptLease(0);defaults.leased=true;
  defaults.adoptLease(sequence);clock+=16;await frame();assert.equal(offered,2,'the end of a lease restarts the count');
 }finally{await defaults.close()}
});
