// L3 (AUDIT2 §4): worker auto-encode under a Node state record (MP-08/MP-10/MP-11).
import test from 'node:test';import assert from 'node:assert/strict';
import {mkdtempSync,writeFileSync,existsSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';import path from 'node:path';
import {NativeWorkerControl} from './kernel-browser-native-worker.mjs';
import {PortableEncoder} from './kernel-browser-display.mjs';
import {LinuxCapture} from './kernel-browser-native.mjs';
import {displayMaskRegions,displayUnprotected} from './kernel-browser-pixels.mjs';

const root=mkdtempSync(path.join(process.env.TMPDIR??tmpdir(),'auto-packets-'));
process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT=root;
test.after(()=>rmSync(root,{recursive:true,force:true}));
let names=0;
const packet=()=>{const name=(++names).toString(16).padStart(32,'0')+'.json';writeFileSync(path.join(root,name),'x');return {name,length:1};};
const gone=p=>!existsSync(path.join(root,p.name));
const reply=(sequence,key=false)=>({backend:'native-openh264',whole:true,revision:sequence,stripes:[{row:0,y:0,height:32,codec:'avc1.420033',key,sequence,reference_sequence:key?null:sequence-1,length:1}],packet:packet()});
function rig(){
 const writes=[],requests=[],manual=[];
 const bridge=new NativeWorkerControl({stdin:{write(bytes){for(const line of bytes.trim().split('\n'))writes.push(JSON.parse(line))}},stdio:[,,,{write(bytes){writes.push({state_retire:Number(bytes)})},writableLength:0}]},()=>{});
 const raw=(serial,{auto,regions}={})=>{
  const r={width:64,height:32,serial,nativeEncode:async q=>{requests.push(q);return manual.shift()},nativeArm:record=>bridge.arm(record),nativeAutoCurrent:()=>bridge.auto,nativeRetireState:encoder=>bridge.retireState(encoder),nativeRetire:()=>{},nativeDelivered:()=>{}};
  if(regions)r[displayMaskRegions]=regions;else r[displayUnprotected]=true;
  if(auto!==undefined){bridge.autoEntry(serial);r.auto={generation:auto,take:()=>bridge.takeAuto(serial)};}
  return r;
 };
 const autoReply=(serial,generation,value)=>bridge.autoReceive({auto:serial,generation,length:1},Buffer.from(JSON.stringify(value)));
 return {writes,requests,manual,bridge,raw,autoReply,encoder:new PortableEncoder()};
}

test('L3 a current unprotected auto frame is used without an encode request',async()=>{
 const {writes,requests,manual,raw,autoReply,encoder}=rig();
 manual.push(reply(1,true));
 await encoder.encode(raw(1),8e6,true,'avc1.420033');
 assert.equal(requests.length,1);
 const armed=writes.filter(w=>w.state);assert.equal(armed.length,1);
 assert.deepEqual({...armed[0].state,until_ms:0},{generation:1,encoder:encoder.nativeSession,bitrate:8e6,reduced:false,until_ms:0});
 // The auto reply may arrive before or after the frame header.
 const early=reply(2);autoReply(2,1,early);
 assert.equal((await encoder.encode(raw(2,{auto:1}),8e6,false,'avc1.420033')).packet.name,early.packet.name);
 const second=raw(3,{auto:1}),late=reply(3);const pending=encoder.encode(second,8e6,false,'avc1.420033');autoReply(3,1,late);
 assert.equal((await pending).packet.name,late.packet.name);
 assert.equal(requests.length,1,'no request for auto-encoded readbacks');
 assert.equal(writes.filter(w=>w.state).length,1,'renewal only near expiry');
});

test('L3 frames of a retired generation are refused and the chain gap re-encodes a key at once',async()=>{
 const {writes,requests,manual,bridge,raw,autoReply,encoder}=rig();
 manual.push(reply(1,true));await encoder.encode(raw(1),8e6,true,'avc1.420033');
 bridge.retireState();
 assert.deepEqual(writes.at(-1),{state_retire:1});
 // Read back before the retire reached the worker: encoded, never published.
 const stale=raw(2,{auto:1}),staleReply=reply(2);autoReply(2,1,staleReply);
 manual.push(reply(3));manual.push(reply(4,true));
 // The gap re-encodes the same readback as a key at once: skipping it would hold its pixels until the next damage.
 const rekeyed=await encoder.encode(stale,8e6,false,'avc1.420033');
 assert.equal(rekeyed.key,true);assert.equal(requests.length,3);assert.equal(requests.at(-1).reset,true);
 bridge.dropAuto(2);assert.ok(gone(staleReply.packet),'the unpublished auto packet is deleted');
 // The next key (after the retire quiet window) re-arms under a new generation; generation 1 stays refused.
 bridge.retiredAt=-Infinity;manual.push(reply(1,true));await encoder.encode(raw(4),8e6,true,'avc1.420033');
 assert.equal(writes.filter(w=>w.state).at(-1).state.generation,2);
 const old=raw(5,{auto:1});autoReply(5,1,reply(2));manual.push(reply(2));
 await encoder.encode(old,8e6,false,'avc1.420033');
 assert.equal(requests.length,5,'a generation-1 frame is never used under generation 2');
});

test('L3 protected frames never use an auto packet',async()=>{
 const {writes,requests,manual,raw,autoReply,encoder}=rig();
 manual.push(reply(1,true));await encoder.encode(raw(1),8e6,true,'avc1.420033');
 const regions=[{x:1,y:2,width:3,height:4}],unmasked=reply(2);
 const masked=raw(2,{auto:1,regions});autoReply(2,1,unmasked);manual.push(reply(1,true));
 const out=await encoder.encode(masked,8e6,false,'avc1.420033',regions);
 assert.notEqual(out.packet.name,unmasked.packet.name);
 assert.deepEqual(requests.at(-1).regions,regions);assert.equal(requests.at(-1).reset,true);
 assert.equal(requests.at(-1).unprotected,undefined);
 assert.deepEqual(writes.find(w=>w.state_retire),{state_retire:1});
 await new Promise(r=>setImmediate(r));assert.ok(gone(unmasked.packet));
 assert.equal(writes.filter(w=>w.state).length,1,'a protected frame never re-arms');
});

test('L3 a protection change retires the record before any other work',()=>{
 const writes=[],bridge=new NativeWorkerControl({stdin:{write(bytes){for(const line of bytes.trim().split('\n'))writes.push(JSON.parse(line))}},stdio:[,,,{write(bytes){writes.push({state_retire:Number(bytes)})},writableLength:0}]},()=>{});
 bridge.arm({encoder:'motion',bitrate:8e6,reduced:false,until_ms:1});
 const capture=new LinuxCapture({allowed:()=>true});capture.nativeWorker=bridge;capture.regions={retire(){}};
 capture.latest={raw:{release(){}}};
 capture.retireFrames();
 assert.deepEqual(writes.slice(1),[{state_retire:1},{reject:'region_revision'}]);
 assert.equal(bridge.auto,null);
 bridge.retiredAt=-Infinity;bridge.arm({encoder:'motion',bitrate:8e6,reduced:false,until_ms:1});
 capture.fence();
 assert.deepEqual(writes.at(-1),{state_retire:2});
});

test('L3 close discards a completed auto packet nobody took',()=>{
 const {bridge,autoReply}=rig();
 bridge.autoEntry(2);const done=reply(2);autoReply(2,1,done);
 bridge.close();
 assert.ok(gone(done.packet),'the completed unconsumed packet is deleted at close');
 bridge.dropAuto(2);
});

test('L3 an auto reply arriving after close is discarded, not thrown',()=>{
 const {bridge,autoReply}=rig();
 bridge.close();
 const late=reply(3);
 assert.doesNotThrow(()=>autoReply(3,1,late));
 assert.ok(gone(late.packet),'the late packet is deleted');
});
