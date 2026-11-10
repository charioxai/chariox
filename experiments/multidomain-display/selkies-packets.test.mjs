// MD-DISPLAY-03: malformed callback input must not escape owned teardown.
import test from 'node:test';
import assert from 'node:assert/strict';
import {PassThrough} from 'node:stream';
import {EventEmitter} from 'node:events';
import {readPackets} from './selkies-packets.mjs';
const tick=()=>new Promise(r=>setImmediate(r));
function stream(){const s=new EventEmitter();s.stdout=new PassThrough();s.stdin=new PassThrough();return s}
function packet(y=0){const b=Buffer.alloc(12);b[0]=4;b[1]=1;b.writeUInt16BE(y,4);b.writeUInt16BE(1920,6);b.writeUInt16BE(1200,8);return JSON.stringify({kind:'binary',data_base64:b.toString('base64')})+'\n'}
for(const [name,line,send,pattern] of [['striped',packet(10),()=>{},/striped/],['malformed JSON','{broken\n',()=>{},/JSON/],['socket buffer limit',packet(),()=>{throw Error('bounded socket queue exceeded')},/bounded socket/]])test(`MD-DISPLAY-03 ${name} reaches awaited failure check`,async()=>{
 const s=stream(),errors=[],reader=readPackets(s,send,()=>{},e=>errors.push(e));let cleaned=false;
 try{s.stdout.write('{"kind":"ready"}\n');assert.equal(reader.ready,true);s.stdout.write(line);await tick();assert.equal(errors.length,1);assert.throws(()=>reader.check(),pattern);s.stdout.write(packet());await tick();assert.equal(errors.length,1)}finally{reader.close();s.stdout.destroy();s.stdin.destroy();cleaned=true}
 assert.equal(cleaned,true);
});
test('MD-DISPLAY-03 supported keyframe preserves geometry/config and counted payload',async()=>{
 const s=stream(),sent=[],sizes=[],reader=readPackets(s,(...v)=>sent.push(v),n=>sizes.push(n),e=>{throw e});
 try{s.stdout.write(packet());await tick();assert.deepEqual(sizes,[12]);assert.equal(sent[0][0].config.codedWidth,1920);assert.equal(sent[0][0].config.codedHeight,1200);assert.equal(sent[0][0].type,'key');reader.check()}finally{reader.close();s.stdout.destroy();s.stdin.destroy()}
});
