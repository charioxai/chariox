// MP-10: the upstream 2.0.0 codec nibble and prediction field are not payload.
import test from 'node:test';import assert from 'node:assert/strict';import {videoPacket} from './baseline-packet.mjs';
test('MP-10 Selkies codec-tagged IDR skips the complete 12-byte stripe header',()=>{
 const bytes=new Uint8Array(23),v=new DataView(bytes.buffer);bytes[0]=4;bytes[1]=0x11;v.setUint16(2,42);v.setUint16(6,1920);v.setUint16(8,1080);v.setUint16(10,41);bytes.set([0,0,0,1,0x67,0x42,0,0x2a,1,2,3],12);
 const p=videoPacket(bytes.buffer);assert.equal(p.key,true);assert.equal(p.id,42);assert.equal(p.codec,'avc1.42002A');assert.deepEqual([...p.payload],[0,0,0,1,0x67,0x42,0,0x2a,1,2,3]);
});
test('MP-10 truncated headers fail before decode',()=>assert.throws(()=>videoPacket(new Uint8Array([4,1]).buffer),/truncated/));
