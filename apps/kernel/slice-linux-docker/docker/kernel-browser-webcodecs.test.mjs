// MD-DISPLAY-02/04: document-bound encoder failure and lifetime ownership.
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {BrowserEncoder} from './kernel-browser-webcodecs.mjs';
function fixture(){
 const calls=[];let document='doc1',fail=false,held;
 const connection={async send(method,params){calls.push({method,params});if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'frame',loaderId:document}}};if(method==='Page.createIsolatedWorld')return {executionContextId:calls.length};if(method==='Runtime.callFunctionOn'&&params.awaitPromise){if(held)await held;if(fail)return {exceptionDetails:{}};return {result:{value:{data_base64:'AA==',key:true}}}}return {}}};
 const fallback={calls:[],async encode(...args){this.calls.push(args);return {data_base64:'AQ==',key:true}},async close(){this.closed=true}};
 return {encoder:new BrowserEncoder({async resolvePageTarget(){return {connection,sessionId:'session'}}},'target',fallback),calls,fallback,setDoc:d=>document=d,setFail:()=>fail=true,setHeld:h=>held=h};
}
test('MD-DISPLAY isolated encoder is document bound and falls back with independent key',async()=>{const f=fixture();await f.encoder.encode('AA==',4000000);f.setDoc('doc2');await f.encoder.encode('AA==',4000000);assert.equal(f.calls.filter(c=>c.method==='Page.createIsolatedWorld').length,2);assert.equal(f.calls.find(c=>c.method==='Page.createIsolatedWorld').params.grantUniveralAccess,false);f.setFail();await f.encoder.encode('AA==',4000000,false);assert.equal(f.fallback.calls[0][2],true);await f.encoder.close();assert.equal(f.fallback.closed,true)});
test('MD-DISPLAY close retires an awaited encoder result and queue stays bounded',async()=>{const f=fixture();let release;f.setHeld(new Promise(r=>release=r));const job=f.encoder.encode('AA==',4000000);await new Promise(r=>setImmediate(r));await assert.rejects(f.encoder.encode('AA==',4000000),/busy/);await f.encoder.close();release();await assert.rejects(job,/closed/);assert.equal(f.fallback.calls.length,0)});
test('MD-DISPLAY browser callback errors reject awaited encode, not an unhandled callback',async()=>{const f=fixture();await f.encoder.encode('AA==',4000000);const install=f.calls.find(c=>c.method==='Runtime.evaluate').params.expression;let opts;const bitmap={width:8,height:8,close(){}};const scope={setTimeout,clearTimeout,atob,btoa,Blob,Uint8Array,performance,createImageBitmap:async()=>bitmap,VideoFrame:class{close(){}},VideoEncoder:class{static async isConfigSupported(){return {supported:true}}constructor(o){opts=o}configure(){}encode(){queueMicrotask(()=>opts.output({byteLength:4,copyTo(){throw Error('callback')}}))}close(){}}};vm.runInNewContext(install,scope);await assert.rejects(scope.charioxDisplayCodec.encode({image:'AA==',bitrate:4000000,codec:'vp09.00.40.08',reset:true}),/callback/);scope.charioxDisplayCodec.close();await f.encoder.close()});
test('MD-DISPLAY fixed isolated worker rejects callback/close and never accepts overlapping work',async()=>{const f=fixture();await f.encoder.encode('AA==',4000000);const install=f.calls.find(c=>c.method==='Runtime.evaluate').params.expression;let worker;const scope={setTimeout,clearTimeout,Blob,URL:{createObjectURL(){return 'blob:owned-fixture'},revokeObjectURL(){}},Worker:class{constructor(){worker=this}postMessage(value){this.sent=value}terminate(){this.closed=true}}};vm.runInNewContext(install,scope);const codec=scope.charioxDisplayCodec;const first=codec.encode({image:'AA=='});await assert.rejects(codec.encode({image:'AA=='}),/busy/);worker.onmessage({data:{id:worker.sent.id,result:{data_base64:'AA==',key:true}}});assert.equal((await first).key,true);const pending=codec.encode({image:'AA=='});codec.close();await assert.rejects(pending,/failed/);assert.equal(worker.closed,true);worker.onmessage({data:{id:worker.sent.id,result:{data_base64:'AQ==',key:true}}});await f.encoder.close()});

test('MD-DISPLAY two viewers own distinct codec worlds, dependency chains and teardown',async()=>{
 const worlds=new Map(),scopes=new Map();let id=0;
 const connection={async send(method,p){
  if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'frame',loaderId:'doc'}}};
  if(method==='Page.createIsolatedWorld'){if(!worlds.has(p.worldName))worlds.set(p.worldName,++id);return {executionContextId:worlds.get(p.worldName)}}
  if(method==='Runtime.evaluate'){scopes.set(p.contextId,{frames:0,closed:false});return {}}
  if(method==='Runtime.callFunctionOn'){
   const s=scopes.get(p.executionContextId);
   if(!p.awaitPromise){s.closed=true;return {}}
   if(s.closed)return {exceptionDetails:{}};
   return {result:{value:{data_base64:Buffer.from(String(++s.frames)).toString('base64'),key:s.frames===1}}};
  }return {};
 }};
 const browser={async resolvePageTarget(){return {connection,sessionId:'shared-session'}}};
 const fallback=()=>({encode(){throw Error('two viewers must not fallback')},close:async()=>{}});
 const a=new BrowserEncoder(browser,'target',fallback()),b=new BrowserEncoder(browser,'target',fallback());
 try{
  assert.equal((await a.encode('AA==',2000000,true)).key,true);
  assert.equal((await b.encode('AA==',2000000,true)).key,true);
  assert.equal(Buffer.from((await a.encode('AA==',2000000)).data_base64,'base64').toString(),'2');
  assert.equal(Buffer.from((await b.encode('AA==',2000000)).data_base64,'base64').toString(),'2');
  assert.equal(worlds.size,2);await a.close();
  assert.equal(Buffer.from((await b.encode('AA==',2000000)).data_base64,'base64').toString(),'3');
 }finally{await a.close();await b.close();assert([...scopes.values()].every(s=>s.closed))}
});
