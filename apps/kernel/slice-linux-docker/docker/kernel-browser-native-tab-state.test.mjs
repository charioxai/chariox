// MP-08/MP-10/MP-11: a native state snapshot is usable only between owner events.
import test from 'node:test';import assert from 'node:assert/strict';
import {ChromeTabGroups,TAB_GROUPS_EXTENSION_ID} from './kernel-browser-tab-groups.mjs';
function fixture(){let listener,fail=false,release;const calls=[];
 const connection={subscribe(fn){listener=fn;return()=>{listener=null}},async send(method,params,sessionId){calls.push({method,params,sessionId});
  if(method==='Target.getTargets')return {targetInfos:[{type:'service_worker',targetId:'worker',url:`chrome-extension://${TAB_GROUPS_EXTENSION_ID}/sw.js`}]};
  if(method==='Target.attachToTarget')return {sessionId:'worker-session'};
  if(method==='Runtime.evaluate'){if(params.expression==='charioxTabs.revision === 2')return{result:{value:true}};if(fail)throw Error('disconnected');if(release)await release;return {result:{value:params.expression==='charioxTabs.active()'?'target-a':{target_id:'target-a',width:1280,height:713}}}}
  return {};
 }};return {groups:new ChromeTabGroups(connection),calls,event:message=>listener?.(message),setFailure(value){fail=value},hold(value){release=value}};
}
test('MP-08/MP-10/MP-11 a fresh viewport is reused locally until a native owner change',async()=>{
 const h=fixture();await h.groups.viewport();const count=h.calls.length;
 assert.deepEqual(h.groups.cachedViewport?.(),{target_id:'target-a',width:1280,height:713});
 assert.equal(h.groups.cachedActive?.(),'target-a');assert.equal(h.calls.length,count,'cached reads send no CDP requests');
 h.event({method:'Runtime.bindingCalled',sessionId:'other-session',params:{name:'__charioxNativeWindowChanged',payload:''}});
 assert.equal(h.groups.cachedActive?.(),'target-a','another target cannot retire or supply native state');
 h.event({method:'Runtime.bindingCalled',sessionId:'worker-session',params:{name:'__charioxNativeWindowChanged',payload:''}});
 assert.equal(h.groups.cachedViewport?.(),null);assert.equal(h.groups.cachedActive?.(),null);
 await h.groups.active();assert.equal(h.groups.cachedActive?.(),'target-a');assert.equal(h.groups.cachedViewport?.(),null,'active alone cannot certify page geometry');h.groups.close?.();
});
test('MP-08/MP-10/MP-11 owner events during an outstanding read cannot restore a stale cache',async()=>{
 const h=fixture();await h.groups.viewport();let resume;h.hold(new Promise(resolve=>resume=resolve));const pending=h.groups.viewport();while(h.calls.filter(c=>c.method==='Runtime.evaluate'&&c.params.expression!=='charioxTabs.revision === 2').length<2)await Promise.resolve();
 h.event({method:'Runtime.bindingCalled',sessionId:'worker-session',params:{name:'__charioxNativeWindowChanged',payload:''}});resume();await pending;
 assert.equal(h.groups.cachedViewport?.(),null,'an older query cannot undo native retirement');h.groups.close?.();
});
test('MP-08/MP-10/MP-11 detached extension state is never reusable',async()=>{
 const h=fixture();await h.groups.viewport();h.event({method:'Target.detachedFromTarget',params:{sessionId:'worker-session'}});
 assert.equal(h.groups.cachedActive?.(),null);assert.equal(h.groups.cachedViewport?.(),null);h.groups.close?.();
});

test('MP-08/MP-10/MP-11 resize, disconnect and close cannot revive cached geometry',async()=>{
 {
  const h=fixture();await h.groups.viewport();h.event({method:'Chariox.browserDisconnected'});assert.equal(h.groups.cachedViewport(),null);assert.equal(h.groups.cachedActive(),null);h.groups.close();
 }
 // r54: a resize moves geometry, never the active Tab (the extension's window events retire that): band keys need no fresh read.
 {
  const h=fixture();await h.groups.viewport();const guard=h.groups.nativeStateGuard();h.event({method:'Page.frameResized',sessionId:'page'});
  assert.equal(h.groups.cachedViewport(),null);assert.equal(h.groups.cachedActive(),'target-a');assert.equal(guard(),false,'an observation taken before the resize is still retired');h.groups.close();
 }
 const h=fixture();await h.groups.viewport();h.groups.close();assert.equal(h.groups.cachedViewport(),null);await assert.rejects(h.groups.viewport(),/closed/);
});

test('MP-08/MP-10/MP-11 generated extension invalidates native metadata on owner API events',async()=>{
 const {mkdtemp,readFile,rm}=await import('node:fs/promises'),os=await import('node:os'),path=await import('node:path'),vm=await import('node:vm');
 const {writeTabGroupsExtension}=await import('./kernel-browser-tab-groups.mjs');
 const root=await mkdtemp(path.join(os.tmpdir(),'chariox-native-state-'));
 try{
  const events={},event=name=>({addListener(fn){events[name]=fn}}),payloads=[];
  const chrome={runtime:{getPlatformInfo(){}},tabs:{onCreated:event('created'),onAttached:event('attached'),onActivated:event('activated'),onRemoved:event('removed'),onZoomChange:event('zoom')},windows:{onBoundsChanged:event('bounds'),onRemoved:event('windowRemoved')}};
  const context={chrome,setInterval(){},__charioxNativeWindowChanged:payload=>payloads.push(payload)};
  vm.runInNewContext(await readFile(path.join(await writeTabGroupsExtension(root),'sw.js'),'utf8'),context);
  for(const name of ['activated','removed','bounds','windowRemoved'])events[name]();
  events.zoom({oldZoomFactor:1,newZoomFactor:1});events.zoom({oldZoomFactor:1,newZoomFactor:1.25});events.zoom();
  assert.deepEqual(payloads,['activated','removed','bounds','windowRemoved','zoomsame','zoom','zoom'],'owner events invalidate only (a bounds event says so); no identity or page content is pushed');
 }finally{await rm(root,{recursive:true,force:true})}
});

// r63: the window event for the resize our own native start made must not retire the lease it fenced a moment later.
test('MP-08/MP-11 a window bounds event drops the cached snapshot but not the guards; other owner events retire both',async()=>{
 const {nativeWindowChangedBinding,nativeBoundsPayload}=await import('./kernel-browser-native-tab-state.mjs');
 for(const [payload,keeps] of [[nativeBoundsPayload,true],['zoomsame',true],['',false],['zoom',false],['activated',false],[undefined,false]]){
  const h=fixture();await h.groups.viewport();const guard=h.groups.nativeStateGuard();assert.equal(guard(),true);
  h.event({method:'Runtime.bindingCalled',sessionId:'worker-session',params:{name:nativeWindowChangedBinding,payload}});
  assert.equal(guard(),keeps,'payload '+payload);assert.equal(h.groups.cachedViewport(),null,'the cached snapshot never survives a window event');
 }
});

// Protocol 556: every retirement of the native snapshot also wakes a parked state read.
test('MP-08/MP-10 owner events, resize and detach wake the state signal',async()=>{
 const woke=[];const h=fixture();
 const groups=new ChromeTabGroups({subscribe:fn=>h.groups.connection.subscribe(fn),send:(...a)=>h.groups.connection.send(...a)},()=>woke.push(1));
 await groups.viewport();const base=woke.length;
 h.event({method:'Runtime.bindingCalled',sessionId:'worker-session',params:{name:'__charioxNativeWindowChanged',payload:''}});
 assert.equal(woke.length,base+1,'owner API event');
 h.event({method:'Page.frameResized',sessionId:'page'});assert.equal(woke.length,base+2,'page resize');
 h.event({method:'Target.detachedFromTarget',params:{sessionId:'worker-session'}});assert.equal(woke.length,base+3,'extension detach');
 groups.close();h.groups.close();
});
