// MP-08/MP-10/MP-11: warm event-bound page key path, supplementary regression.
import test from 'node:test';import assert from 'node:assert/strict';
import {inputHostTab} from './kernel-browser-input.mjs';
const tab={target_id:'t',document_id:'doc'};
function fixture({retireDuringEvaluation=false}={}){
 const calls=[];let current=true,native=0;
 const connection={send:async(method)=>{calls.push(method);if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'f',loaderId:current?'doc':'next'}}};if(method==='Runtime.evaluate'){if(retireDuringEvaluation)current=false;return {result:{value:true}}}if(method==='Page.createIsolatedWorld')return {executionContextId:7};return {}}};
 const browser={chariox_text_worlds:new Map([['t',{document:'doc',contextId:7}]]),resolvePageTarget:async()=>({connection,sessionId:'s'}),inputCapture:{run:async(_c,_s,fn)=>fn()}};
 return {calls,browser,options:{documentCurrent:()=>current,nativeKey:()=>{native++;return true}},native:()=>native};
}
test('MP-08/MP-10/MP-11 warm native page key has only the final live focus evaluation',async()=>{
 const f=fixture();await inputHostTab(f.browser,tab,{kind:'key',key:'a'},f.options);
 assert.equal(f.native(),1);assert.deepEqual(f.calls,['Runtime.evaluate']);
});
test('MP-08/MP-10/MP-11 document retirement while focus evaluation awaits refuses XTest',async()=>{
 const f=fixture({retireDuringEvaluation:true});await assert.rejects(inputHostTab(f.browser,tab,{kind:'key',key:'a'},f.options),{code:'stale_document_reference'});assert.equal(f.native(),0);
});
test('MP-08/MP-10/MP-11 a missing focus world never binds a different loader',async()=>{
 const f=fixture();f.browser.chariox_text_worlds.clear();
 const {connection}=await f.browser.resolvePageTarget();const send=connection.send;
 connection.send=async(method,...args)=>method==='Page.getFrameTree'?(f.calls.push(method),{frameTree:{frame:{id:'f',loaderId:'next'}}}):send(method,...args);
 await assert.rejects(inputHostTab(f.browser,tab,{kind:'key',key:'a'},f.options),{code:'stale_document_reference'});assert.equal(f.native(),0);
 assert(!f.calls.includes('Page.createIsolatedWorld'),'a stale loader must be refused before world creation');
});
