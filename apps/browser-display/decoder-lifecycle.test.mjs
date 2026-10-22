// MP-08/MP-10/MP-11: keyframes reset references, not the configured codec process.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
const source=await readFile(process.env.MP08_DECODER_SOURCE??new URL('./decoder-worker.mjs',import.meta.url),'utf8');
function worker(){
 const decoders=[],messages=[];class Scope{static[Symbol.hasInstance](){return true}}
 const scope=new Scope();scope.WorkerGlobalScope=Scope;scope.self=scope;scope.postMessage=value=>messages.push(value);
 scope.VideoDecoder=class{constructor(callbacks){this.callbacks=callbacks;this.state='unconfigured';decoders.push(this)}configure(config){this.config=config;this.state='configured'}decode(chunk){this.chunk=chunk;this.callbacks.output({close(){},timestamp:chunk.timestamp})}close(){this.state='closed'}};
 scope.EncodedVideoChunk=class{constructor(value){Object.assign(this,value)}};
 vm.runInNewContext(source.slice(0,source.indexOf('export class')),scope);
 return {send:(overrides={})=>scope.onmessage({data:{id:messages.length+1,codec:'avc1.420033',key:true,sequence:messages.length+1,width:2560,height:1600,bytes:new Uint8Array([1]),...overrides}}),decoders,messages};
}
test('MP-08/MP-10 repeated independent keys reuse the configured decoder',()=>{
 const w=worker();w.send();w.send();w.send({key:false});assert.equal(w.decoders.length,1);assert.equal(w.messages.length,3);assert(w.messages.every(m=>m.frame));
});
test('MP-08/MP-10/MP-11 a geometry change requires a key and starts a new decoder',()=>{
 const w=worker();w.send();w.send({key:false,width:1280});assert(w.messages.at(-1).error);w.send({width:1280});assert(w.messages.at(-1).frame);assert.equal(w.decoders.at(-1).config.codedWidth,1280);
});
test('MP-08/MP-10 recovery handles the codec being closed before its error callback',()=>{
 const w=worker();w.send();const d=w.decoders[0];d.state='closed';d.close=()=>{throw Error('InvalidStateError')};assert.doesNotThrow(()=>d.callbacks.error(Error('decode failure')));assert(w.messages.at(-1).error);w.send();assert(w.messages.at(-1).frame);assert.equal(w.decoders.length,2);
});
