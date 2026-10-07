// MD-DISPLAY-02/04: fragmented raw frames must not recopy the full image per chunk.
import test from 'node:test';import assert from 'node:assert/strict';
import {NativePipe} from './kernel-browser-native-pipe.mjs';
const packet=(header,pixels)=>{const text=Buffer.from(JSON.stringify(header)),n=Buffer.alloc(4);n.writeUInt32BE(text.length);return Buffer.concat([n,text,pixels])};
test('MD-DISPLAY linear raw framing across arbitrary chunks and multiple frames',()=>{
 const seen=[],pipe=new NativePipe(h=>{assert.equal(h.length,16)},(h,p)=>seen.push(p));const wire=packet({length:16},Buffer.from(Array.from({length:16},(_,i)=>i)));
 for(const n of [1,2,3,5,64]){seen.length=0;for(let frame=0;frame<2;frame++)for(let at=0;at<wire.length;at+=n)pipe.push(wire.subarray(at,at+n));assert.equal(seen.length,2);assert.deepEqual(seen[0],Buffer.from(Array.from({length:16},(_,i)=>i)));assert.notEqual(seen[0],seen[1]);}
});
test('MD-DISPLAY bounded raw framing rejects malformed header before pixel allocation',()=>{
 for(const size of [0,8193,0xffffffff]){const b=Buffer.alloc(4);b.writeUInt32BE(size);assert.throws(()=>new NativePipe(()=>{},()=>{}).push(b),/header/)}
 assert.throws(()=>new NativePipe(()=>{},()=>{}).push(packet({length:2560*1600*4+1},Buffer.alloc(0))),/frame/);
});
test('MP-08/MP-10/MP-11 bounded dual native damage sets survive fragmented headers',()=>{
 const tiles=Array.from({length:128},(_,n)=>[n%80*32,Math.floor(n/80)*32,n%80*32+32,Math.floor(n/80)*32+32]);
 const header={length:1,damage_tiles:tiles,adjacent_damage_tiles:tiles,signature:'f'.repeat(16),serial:17,base_serial:3,native_cpu:[1,2,3],captured_ms:1900000000000,damage:[0,0,2560,1600]};
 let seen;const wire=packet(header,Buffer.from([0]));assert.ok(wire.readUInt32BE(0)>4096,'Retina dual metadata exceeds the old private header bound');
 const pipe=new NativePipe(()=>{},h=>seen=h);
 for(let at=0;at<wire.length;at+=7)pipe.push(wire.subarray(at,at+7));
 assert.deepEqual(seen,header);
});
