// MP-08/MP-10/MP-11: §2a private resolver regressions; not live acceptance.
import test from 'node:test';
import assert from 'node:assert/strict';
import { Mirror2Regions } from './kernel-browser-mirror2-regions.mjs';

function fixture() {
  const tab = { tab_id: 'tab', document_id: 'doc', target_id: 'target' };
  const host = { generation: 7, protection: {}, pageSize: () => ({width:1280,height:800}), tabs: new Map([['tab', tab]]), scales: new Map([['tab', 2]]), displays: new Map(), removed: [],
    async removeDisplay(id) { this.removed.push(id); this.displays.delete(id); } };
  const stream = { id: 'mirror', scope: 'owner', tab_id: 'tab', wire: 2, document_id: 'doc', issued: 6, resetAt: 3, policy: host.protection, chain: Promise.resolve(), frames: new Map(), frameSlots: new Map(),
    introduced: new Map([['n1', 3], ['n2', 3], ['n3', 3]]), identity: new Map(), world: { ref: 'observer', connection: { isOpen: () => true }, frame: { loaderId: 'doc' } } };
  stream.world.sessionId='s';stream.world.contextId=77;stream.world.frame.id='frame';
  let layerListener;
  Object.assign(stream.world.connection,{subscribe(fn){layerListener=fn;return()=>{layerListener=null}},async send(method,params){
    if(method==='LayerTree.enable')layerListener?.({sessionId:'s',method:'LayerTree.layerTreeDidChange',params:{layers:[{backendNodeId:42,drawsContent:true,width:200,height:100}]}});
    if(method==='Runtime.evaluate')return{result:params.awaitPromise?{value:true}:{objectId:'media'}};
    if(method==='DOM.describeNode')return{node:{nodeType:1,nodeName:'CANVAS',backendNodeId:42}};
    return{};
  }});
  host.browser = { connection: stream.world.connection };
  const service = { host, streams: new Map([['mirror', stream]]), require(id, scope, generation) { if (id !== 'mirror' || scope !== stream.scope || generation !== host.generation || !this.streams.has(id)) throw Error('foreign'); return stream; } };
  let boxes = [{ id: 'n3', box: [10, 20, 200, 100] }];
  const mirror = { host, service, frames:{loaderId:async()=>tab.document_id}, hurry() {}, async regionBoxes() { return boxes; }, async resolveInput(s, t, input) { return { input }; } };
  const regions = new Mirror2Regions(mirror);
  regions.record(stream, { reset: true, nodes: [{id:'n1', parent:null, kind:'document'}, {id:'n2', parent:'n1', kind:'element'}, {id:'n3', parent:'n2', kind:'tile'}] });
  const target = {mirror_subscription_id:'mirror', generation:7, document_id:'doc', sequence:5, region_id:'n3'};
  return { host, stream, service, mirror, regions, target, tab, boxes, setBoxes: value => { boxes = value; } };
}

test('MP-11: stale/ahead/unintroduced sequences and foreign caller refuse', async () => {
  const f = fixture();
  for (const sequence of [0, 2, 7, 3.5]) await assert.rejects(f.regions.resolve({...f.target, sequence}, 'owner'));
  f.stream.introduced.set('n3', 6); await assert.rejects(f.regions.resolve(f.target, 'owner'));
  await assert.rejects(f.regions.resolve(f.target, 'other'));
  await assert.rejects(f.regions.resolve({...f.target, rect: {x:0,y:0,width:1280,height:800}}, 'owner'));
});
test('MP-11: changed identity retires the exact display and cannot be re-used', async () => {
  const f = fixture(), lease = await f.regions.resolve(f.target, 'owner');
  assert.equal(await lease.isCurrent(), true);
  lease.attachDisplay({subscription_id:'display', observed_by:'owner', tab_id:'tab'});
  f.stream.identity.set('n3', 6);
  assert.equal(await lease.isCurrent(), false); assert.deepEqual(f.host.removed, ['display']);
  await assert.rejects(f.regions.resolve(f.target, 'owner'));
});
test('MP-11: a transformed region leases its on-screen rect; masks and ordinary elements never grant a crop', async () => {
  const f = fixture(); f.boxes[0].distorted = 'transform';
  const lease = await f.regions.resolve(f.target, 'owner');
  assert.deepEqual(lease.rect, {x:10,y:20,width:200,height:100}); assert.deepEqual(lease.element_offset, {x:0,y:0});
  for (const kind of ['mask', 'element']) { f.regions.record(f.stream, {reset:true,nodes:[{id:'n3',parent:null,kind}]}); await assert.rejects(f.regions.resolve(f.target,'owner')); }
});
test('MP-11: navigation, subscription loss, policy change and removal retire', async () => {
  for (const mutate of [f => f.tab.document_id='next', f => f.service.streams.clear(), f => f.host.protection={}, f => f.regions.record(f.stream,{ops:[{op:'children',id:'n2',nodes:[]}]})]) {
    const f=fixture(), lease=await f.regions.resolve(f.target,'owner'); mutate(f);
    assert.equal(await lease.isCurrent(),false); await assert.rejects(lease.resolveInput({sequence:5,x:1,y:1,kind:'click'},'owner'));
  }
});
test('MP-08/MP-10/MP-11: clipped live geometry and region input use mirror fences', async () => {
  const f=fixture(); f.setBoxes([{id:'n3',box:[-20,20,100,100]}]);
  const lease=await f.regions.resolve(f.target,'owner');
  assert.deepEqual(lease.rect,{x:0,y:20,width:80,height:100}); assert.equal(lease.device_scale_factor,2);
  const resolved=await lease.resolveInput({sequence:5,x:4,y:8,kind:'click'},'owner');
  assert.deepEqual(resolved.input.action,{kind:'click',node_id:'n3',x:24,y:8});
  await assert.rejects(lease.resolveInput({sequence:5,x:80,y:8,kind:'click'},'owner'));
  await assert.rejects(lease.resolveInput({sequence:5,x:4,y:8,kind:'click',_agent_input:true},'owner'));
  f.setBoxes([{id:'n3',box:[-20,21,100,100]}]); assert.equal(await lease.isCurrent(),false);
});
test('MP-08/MP-10/MP-11: private placement is relative to the original opaque element, including frame/viewport clipping', async () => {
  for (const [box,clip,expected] of [
    [[-20,20,100,100],undefined,{x:20,y:0}],
    [[50,60,200,100],[70,80,80,40],{x:20,y:20}],
    [[10.25,20.75,200,100],undefined,{x:0,y:0}],
  ]) {
    const f=fixture(); f.setBoxes([{id:'n3',box,...(clip?{clip}:{})}]);
    const lease=await f.regions.resolve(f.target,'owner');
    assert.deepEqual(lease.element_offset,expected);assert(Object.isFrozen(lease.element_offset));
    await assert.rejects(f.regions.resolve({...f.target,element_offset:expected},'owner'));
    await lease.close();assert.equal(await lease.isCurrent(),false);
  }
});
test('MP-11: resolver serializes behind the issued packet and rechecks after live measurement', async () => {
  const f=fixture(); let release; f.stream.chain=new Promise(r=>release=r); let done=false;
  const pending=f.regions.resolve(f.target,'owner').then(x=>{done=true;return x}); await new Promise(r=>setImmediate(r)); assert.equal(done,false); release(); await pending;
  f.mirror.regionBoxes=async()=>{f.host.protection={};return f.boxes;}; await assert.rejects(f.regions.resolve(f.target,'owner'));
});

test('MP-11: a children delta retains issued opaque siblings and removes departed subtrees', async () => {
  const f=fixture(); f.regions.record(f.stream,{ops:[{op:'children',id:'n2',children:['n3','n4'],nodes:[{id:'n4',parent:'n2',kind:'element'}]}]});
  assert.equal((await f.regions.resolve(f.target,'owner')).region_id,'n3');
  f.regions.record(f.stream,{ops:[{op:'children',id:'n1',children:[],nodes:[]}]});
  await assert.rejects(f.regions.resolve(f.target,'owner'));
});

test('MP-11: only current kernel-reported media gets a comparison exclusion',async()=>{
 const f=fixture(),lease=await f.regions.resolve(f.target,'owner');
 assert.deepEqual((await lease.attestationState()).media,[]);
 f.boxes[0].media=true;
 assert.deepEqual((await lease.attestationState()).media.map(({id,box})=>({id,box})),[{id:'n3',box:[10,20,200,100]}]);
 await assert.rejects(f.regions.resolve({...f.target,media:[f.boxes[0]]},'owner'));
 f.boxes.push({id:'n4',box:[0,0,10,10],media:true});
 await assert.rejects(lease.attestationState(),/region_target/,'unissued media cannot supply an exclusion');
});
test('MP-11: stale documents and wrong target identities refuse attestation evidence',async()=>{
 for(const mutate of [f=>f.tab.document_id='next',f=>f.host.tabs.set('tab',{...f.tab,target_id:'other'}),f=>f.stream.identity.set('n3',6)]){
  const f=fixture(),lease=await f.regions.resolve(f.target,'owner');f.boxes[0].media=true;mutate(f);
  await assert.rejects(lease.attestationState(),/retired/);
 }
});
test('MP-08/MP-10/MP-11 completed subscribe cancellation does not poison the owned region lease',async()=>{
 const f=fixture(),request=new AbortController();f.boxes[0].media=true;
 const cancelled=new AbortController();cancelled.abort();
 await assert.rejects(f.regions.resolve(f.target,'owner',{signal:cancelled.signal}));
 const lease=await f.regions.resolve(f.target,'owner',{signal:request.signal});
 request.abort(); // RPC lifetime ended; the display has its own close/expiry.
 assert.equal(await lease.isCurrent(),true);
 assert.equal((await lease.attestationState()).media.length,1);
 await lease.close();assert.equal(await lease.isCurrent(),false);
});

test('MP-08/MP-10/MP-11: region leases cover the resized viewport and retire when it changes', async () => {
  const f=fixture(); let size={width:1610,height:900}; f.host.pageSize=()=>({...size});
  f.setBoxes([{id:'n3',box:[1250,780,500,200]}]);
  const lease=await f.regions.resolve(f.target,'owner');
  assert.deepEqual(lease.rect,{x:1250,y:780,width:360,height:120});
  assert.equal(lease.css_width,1610); assert.equal(lease.css_height,900);
  lease.attachDisplay({subscription_id:'display',observed_by:'owner',tab_id:'tab'});
  size={width:1700,height:900};
  assert.equal(lease.isAlive(),false);
  assert.equal(await lease.isCurrent(),false); assert.deepEqual(f.host.removed,['display']);
});
