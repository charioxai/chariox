// MP-08/MP-10/MP-11: completed-response identity and memory bounds.
import test from 'node:test';import assert from 'node:assert/strict';
import {BrowserResponseBodies}from'./browser-controller-response-bodies.mjs';
const address='https://public.example/style.css';
const event=(cache,method,params,sessionId='S')=>cache.observe({method,sessionId,params:{requestId:'r',...params}});
const world=(connection,sessionId='S',frame='F',loader='D')=>({connection,sessionId,frame:{id:frame,loaderId:loader}});
test('MP-11: only a completed response in the same session/frame/document can be read',async()=>{
 const c=new BrowserResponseBodies(),calls=[],connection={async send(...args){calls.push(args);return{body:'a{color:red}',base64Encoded:false}}};
 event(c,'Network.requestWillBeSent',{frameId:'F',loaderId:'D',request:{url:address}});
 event(c,'Network.responseReceived',{frameId:'F',loaderId:'D',response:{url:address}});
 assert.equal(await c.read(world(connection),address),null);assert.equal(calls.length,0);
 event(c,'Network.loadingFinished',{});
 for(const w of [world(connection,'other'),world(connection,'S','other'),world(connection,'S','F','new')])assert.equal(await c.read(w,address),null);
 assert.equal((await c.read(world(connection),address)).toString(),'a{color:red}');assert.deepEqual(calls,[['Network.getResponseBody',{requestId:'r'},'S']]);
 c.drop('S');assert.equal(await c.read(world(connection),address),null);
});
test('MP-11: redirects keep only observed aliases; failures and reconnects retire metadata',async()=>{
 const c=new BrowserResponseBodies(),connection={async send(){return{body:Buffer.from('font').toString('base64'),base64Encoded:true}}};
 event(c,'Network.requestWillBeSent',{frameId:'F',loaderId:'D',request:{url:address}});
 event(c,'Network.requestWillBeSent',{frameId:'F',loaderId:'D',request:{url:address+'?redirect=1'}});
 event(c,'Network.loadingFinished',{});
 assert.equal((await c.read(world(connection),address)).toString(),'font');assert.equal((await c.read(world(connection),address+'?redirect=1')).toString(),'font');
 assert.equal(await c.read(world(connection),address,1),null);c.clear();assert.equal(await c.read(world(connection),address),null);
 event(c,'Network.requestWillBeSent',{frameId:'F',loaderId:'D',request:{url:address}});event(c,'Network.loadingFailed',{});event(c,'Network.loadingFinished',{});assert.equal(await c.read(world(connection),address),null);
});
test('MP-11: both metadata maps stay bounded',()=>{const c=new BrowserResponseBodies();for(let i=0;i<5000;i++){c.observe({sessionId:'S',method:'Network.responseReceived',params:{requestId:String(i),frameId:'F',loaderId:'D',response:{url:address+'?n='+i}}});c.observe({sessionId:'S',method:'Network.loadingFinished',params:{requestId:String(i)}})}assert(c.loaded.size<=4096);for(let i=0;i<5000;i++)c.observe({sessionId:'S',method:'Network.responseReceived',params:{requestId:String(i),frameId:'F',loaderId:'D',response:{url:address}}});assert(c.requests.size<=4096);});
