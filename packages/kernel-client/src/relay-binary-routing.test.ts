import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"
import { decodeCompactRelayRouting } from "./relay-binary-routing.js"
const fixture = JSON.parse(readFileSync(new URL("../../../apps/kernel/src/local/api/tests/protocol_shapes/relay-compact-961.json", import.meta.url), "utf8")) as {frames: {magic:string;kind:number;id:string;wire_hex:string;sender_public_key:string;nonce:string}[]}
test("MP-08/MP-09/MP-10/MP-11 CXR routing matches kernel vectors without normalizing IDs",()=>{
 for(const frame of fixture.frames) {
  const bytes=Buffer.from(frame.wire_hex,"hex"),event=frame.magic==="CXR1"
  if(frame.kind===0) { assert.throws(()=>decodeCompactRelayRouting(bytes,event));continue }
  const result=decodeCompactRelayRouting(bytes,event)
  assert.equal(result.id,frame.id);assert.equal(result.sender_public_key,frame.sender_public_key);assert.equal(result.nonce,frame.nonce)
  assert.equal(result.eventId,event?17:0);assert.deepEqual(result.ciphertext,Buffer.alloc(16))
  for(let length=0;length<bytes.length;length++)assert.throws(()=>decodeCompactRelayRouting(bytes.subarray(0,length),event))
  for(const offset of [3,4,event?15:7]) {const bad=Buffer.from(bytes);bad[offset]=255;assert.throws(()=>decodeCompactRelayRouting(bad,event))}
 }
})
