// MP-08/MP-10/MP-11: one trusted world per connection/session/document; late creation never replaces a newer owner.
import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserCdpClient} from './browser-controller-cdp.mjs';
const deferred=()=>{let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b});return {promise,resolve,reject};};
const browser=()=>Object.assign(Object.create(BrowserCdpClient.prototype),{focusWorldsByTarget:new Map()});
test('MP-08/MP-10/MP-11 native restoration and mirror startup share the same in-flight isolated world',async()=>{
 const gate=deferred(),b=browser();let calls=0;const c={send:async()=>{calls++;return gate.promise;}},frame={id:'top',loaderId:'doc'};
 const first=b.ensureFocusWorld(c,'s','t',frame),second=b.ensureFocusWorld(c,'s','t',frame);gate.resolve({executionContextId:7});
 const [a,d]=await Promise.all([first,second]);assert.equal(calls,1);assert.equal(a,d);assert.equal(a.contextId,7);
});
test('MP-11 a late old-document world cannot overwrite the new document world',async()=>{
 const old=deferred(),fresh=deferred(),b=browser(),c={send:(_method,p)=>p.frameId==='old-frame'?old.promise:fresh.promise};
 const first=b.ensureFocusWorld(c,'s','t',{id:'old-frame',loaderId:'old'}),second=b.ensureFocusWorld(c,'s','t',{id:'new-frame',loaderId:'new'});
 fresh.resolve({executionContextId:8});const current=await second;old.resolve({executionContextId:7});await first;
 assert.equal(b.focusWorldsByTarget.get('t'),current);assert.equal(current.contextId,8);
});
test('MP-11 a failed old world cannot retire the new world and a failed current creation retries',async()=>{
 const old=deferred(),b=browser();let retry=0;const c={send:(_method,p)=>p.frameId==='old-frame'?old.promise:Promise.resolve({executionContextId:++retry+7})};
 const first=b.ensureFocusWorld(c,'s','t',{id:'old-frame',loaderId:'old'});const rejected=assert.rejects(first,/retired/);
 const current=await b.ensureFocusWorld(c,'s','t',{id:'new-frame',loaderId:'new'});old.reject(Error('retired'));await rejected;assert.equal(b.focusWorldsByTarget.get('t'),current);
 const failed={send:async()=>{throw Error('failed');}};await assert.rejects(b.ensureFocusWorld(failed,'f','failed',{id:'frame',loaderId:'doc'}),/failed/);
 const retryWorld=await b.ensureFocusWorld(c,'f','failed',{id:'frame',loaderId:'doc'});assert(retryWorld.contextId>0);
});
test('MP-11 an unchanged loader never reuses a world across connections or sessions',async()=>{
 const b=browser(),frame={id:'frame',loaderId:'doc'},c={send:async()=>({executionContextId:7})},other={send:async()=>({executionContextId:8})};
 const first=await b.ensureFocusWorld(c,'s','t',frame),second=await b.ensureFocusWorld(other,'s','t',frame),third=await b.ensureFocusWorld(other,'new-session','t',frame);
 assert.notEqual(first,second);assert.notEqual(second,third);assert.equal(second.contextId,8);
});
