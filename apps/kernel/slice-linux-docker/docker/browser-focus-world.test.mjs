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

// MP-08 / MP-10 / MP-11: the owned preload already creates this isolated world.
function preloadedFixture() {
 let calls=0;
 const connection={isOpen:()=>true,send:async()=>{calls++;return{executionContextId:9}}};
 const b=new BrowserCdpClient({focusWorldSource:'void 0'});
 b.connection=connection;b.sessionsByTarget.set('t','s');b.targetsBySession.set('s','t');
 b.recordConnectionEvent({sessionId:'s',method:'Page.frameNavigated',params:{frame:{id:'top',loaderId:'doc'}}});
 const context={id:7,uniqueId:'unique-live-world',name:'chariox-controller-focus',auxData:{frameId:'top',type:'isolated',isDefault:false}};
 const emit=context=>b.recordConnectionEvent({sessionId:'s',method:'Runtime.executionContextCreated',params:{context}});
 return{b,connection,context,emit,calls:()=>calls,frame:{id:'top',loaderId:'doc'}};
}
test('MP-08/MP-10/MP-11 reuse the current owned preload without another renderer world creation',async()=>{
 const f=preloadedFixture();f.emit(f.context);
 const world=await f.b.ensureFocusWorld(f.connection,'s','t',f.frame);
 assert.equal(world.contextId,7);assert.equal(world.uniqueContextId,f.context.uniqueId);assert.equal(f.calls(),0);
});
for(const invalid of ['name','default','type','frame','unique','source','frame-owner']){
 test(`MP-11 reject unowned or incomplete preload context: ${invalid}`,async()=>{
  const f=preloadedFixture(),context=structuredClone(f.context);
  if(invalid==='name')context.name='page-world';
  if(invalid==='default')context.auxData.isDefault=true;
  if(invalid==='type')context.auxData.type='default';
  if(invalid==='frame')context.auxData.frameId='child';
  if(invalid==='unique')delete context.uniqueId;
  if(invalid==='source')f.b.focusWorldSource=undefined;
  if(invalid==='frame-owner')f.b.pageFramesByTarget.get('t').connection={};
  f.emit(context);
  const world=await f.b.ensureFocusWorld(f.connection,'s','t',f.frame);
  assert.equal(world.contextId,9);assert.equal(f.calls(),1);
 });
}
for(const method of ['Runtime.executionContextDestroyed','Runtime.executionContextsCleared','Page.frameNavigated']){
 test(`MP-11 retire preload ownership on ${method}`,async()=>{
  const f=preloadedFixture();f.emit(f.context);
  const previous=await f.b.ensureFocusWorld(f.connection,'s','t',f.frame);assert.equal(previous.contextId,7);
  const params=method==='Runtime.executionContextDestroyed'?{executionContextId:7,executionContextUniqueId:f.context.uniqueId}:method==='Page.frameNavigated'?{frame:{id:'top',loaderId:'new-doc'}}:{};
  f.b.recordConnectionEvent({sessionId:'s',method,params});
  const frame=method==='Page.frameNavigated'?{id:'top',loaderId:'new-doc'}:f.frame;
  const current=await f.b.ensureFocusWorld(f.connection,'s','t',frame);assert.equal(current.contextId,9);assert.equal(f.calls(),1);
 });
}

test('MP-08/MP-10/MP-11 observed preload releases an in-flight creator without waiting for its renderer reply',async()=>{
 const f=preloadedFixture(),created=deferred();f.connection.send=()=>created.promise;
 const first=f.b.ensureFocusWorld(f.connection,'s','t',f.frame);f.emit(f.context);
 let timer;
 try {
  const world=await Promise.race([first,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('blocked on redundant world creation')),100)})]);
  assert.equal(world,f.b.focusWorldsByTarget.get('t'));assert.equal(world.contextId,7);
  created.reject(Error('late renderer timeout'));
  await new Promise(setImmediate);assert.equal(f.b.focusWorldsByTarget.get('t'),world);
 } finally {clearTimeout(timer);created.resolve({executionContextId:7});await first;}
});
test('MP-11 a delayed destruction for a reused numeric context ID does not retire its new unique owner',async()=>{
 const f=preloadedFixture();f.emit(f.context);
 const world=await f.b.ensureFocusWorld(f.connection,'s','t',f.frame);
 f.b.recordConnectionEvent({sessionId:'s',method:'Runtime.executionContextDestroyed',params:{executionContextId:7,executionContextUniqueId:'retired-unique-world'}});
 assert.equal(await f.b.ensureFocusWorld(f.connection,'s','t',f.frame),world);assert.equal(f.calls(),0);
});

test('MP-11 a queued old-connection context event cannot claim the new connection world',async()=>{
 const callbacks=[];
 const connection=()=>({isOpen:()=>true,send:async()=>({}),close:async()=>{},subscribe:listener=>{callbacks.push(listener);return()=>{}}});
 const first=connection(),second=connection();let count=0;
 const b=new BrowserCdpClient({focusWorldSource:'void 0',connectionFactory:async()=>++count===1?first:second});
 try {
  await b.openConnection();await b.openConnection();b.targetsBySession.set('s','t');b.sessionsByTarget.set('t','s');
  callbacks[1]({sessionId:'s',method:'Page.frameNavigated',params:{frame:{id:'top',loaderId:'doc'}}});
  const context={id:7,uniqueId:'new-connection-world',name:'chariox-controller-focus',auxData:{frameId:'top',type:'isolated',isDefault:false}};
  callbacks[1]({sessionId:'s',method:'Runtime.executionContextCreated',params:{context}});
  const current=b.focusWorldsByTarget.get('t');assert.equal(current.uniqueContextId,context.uniqueId);
  callbacks[0]({sessionId:'s',method:'Runtime.executionContextCreated',params:{context:{...context,uniqueId:'old-connection-world'}}});
  assert.equal(b.focusWorldsByTarget.get('t'),current);assert.equal(current.uniqueContextId,'new-connection-world');
 } finally {await b.close();}
});

test('MP-11 a new unique context gets a fresh owner even when its numeric ID and loader are reused',async()=>{
 const f=preloadedFixture();f.emit(f.context);
 const previous=await f.b.ensureFocusWorld(f.connection,'s','t',f.frame);previous.mirror2Installed=true;
 f.emit({...f.context,uniqueId:'replacement-unique-world'});
 const current=await f.b.ensureFocusWorld(f.connection,'s','t',f.frame);
 assert.notEqual(current,previous);assert.equal(current.mirror2Installed,undefined);assert.equal(current.uniqueContextId,'replacement-unique-world');
 f.b.recordConnectionEvent({sessionId:'s',method:'Runtime.executionContextDestroyed',params:{executionContextId:7,executionContextUniqueId:f.context.uniqueId}});
 assert.equal(f.b.focusWorldsByTarget.get('t'),current);
});
