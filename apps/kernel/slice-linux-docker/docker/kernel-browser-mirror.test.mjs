// MP-08/MP-10/MP-11: service epochs, privacy, bounded streams and resource admission.
import test from 'node:test';
import assert from 'node:assert/strict';
import { MirrorService } from './kernel-browser-mirror.mjs';
import { materializeMirrorResources,mirrorHash } from './kernel-browser-mirror-resources.mjs';
import { encodePng } from './kernel-browser-pixels.mjs';
function fixture() {
  const tab={tab_id:'t',target_id:'target',document_id:'d'},state={document:'d',snapshot:{root:'n1',nodes:[{id:'n1',parent:null,children:['n2'],kind:'element',tag:'div',box:{x:0,y:0,width:80,height:40}},{id:'n2',parent:'n1',children:[],kind:'text',text:'fixture'}],fonts:[],resources:[],scroll:{x:0,y:0},focused:null,selection:null}};
  const connection={async send(method,params){
    if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'frame',loaderId:state.document}}};
    if(method==='Page.getResourceTree')return {frameTree:{frame:{id:'frame'},resources:[]}};
    if(method==='DOM.getDocument')return {root:{nodeId:1,children:[]}};
    if(method==='DOM.querySelectorAll')return {nodeIds:[]};
    if(method==='Emulation.setDeviceMetricsOverride')return {};
    if(method==='Runtime.evaluate')return {result:{value:params.expression.includes('.read(')?structuredClone(state.snapshot):true}};
    throw Error(`unexpected CDP method ${method}`);
  }};
  const host={generation:1,scales:new Map(),protection:{values:[],targets:[],unknown:false},async target(){return {...tab,document_id:state.document};},async displayTarget(){return {...tab,document_id:state.document};},browser:{async resolvePageTarget(){return {connection,sessionId:'session'};},async ensureFocusWorld(){return {contextId:1};}},async screenshot(){return {data_base64:encodePng(1280,800,Buffer.alloc(1280*800*4,100)),protected_regions:[]};}};
  return {host,state,service:new MirrorService(host)};
}
const next=(subscription_id,after_sequence=0,drift_nodes=[])=>({subscription_id,generation:1,after_sequence,drift_nodes});
test('MP-11: guessed stream IDs never authorize another terminal, expired or recovered browser',async()=>{
 const {service,host}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'terminal:a');
 await assert.rejects(service.next(next(s.subscription_id),'terminal:b'),/foreign/);
 await assert.rejects(service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:0,action:{kind:'key',key:'Tab'}},'terminal:b'),/foreign/);
 host.generation=2;await assert.rejects(service.next(next(s.subscription_id),'terminal:a'),/foreign/);
 service.streams.get(s.subscription_id).expires=0;assert.throws(()=>service.require(s.subscription_id,'terminal:a',2),/foreign/);assert.equal(service.streams.size,0);
});
test('MP-08: sanitized patches preserve stable IDs, ordering, remove state and repair lost bases',async()=>{
 const {service,state}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 const first=await service.next(next(s.subscription_id),'a');assert(first.reset);assert.equal(first.hash,mirrorHash({root:state.snapshot.root,nodes:state.snapshot.nodes,fonts:[],scroll:{x:0,y:0},focused:null,selection:null}));
 state.snapshot.nodes[1].text='changed';const changed=await service.next(next(s.subscription_id,first.sequence),'a');assert(!changed.reset);assert.deepEqual(changed.nodes.map(n=>n.id),['n2']);
 const repaired=await service.next(next(s.subscription_id,0),'a');assert(repaired.reset);assert.equal(repaired.nodes.length,2);
 state.snapshot.nodes.shift();state.snapshot.nodes[0].parent=null;state.snapshot.root='n2';const removed=await service.next(next(s.subscription_id,repaired.sequence),'a');assert.deepEqual(removed.removed,['n1']);
});
test('MP-11: protection change forces reset and refuses stale/old rendered input',async()=>{
 const {service,host}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),packet=await service.next(next(s.subscription_id),'a');
 host.protection={values:[],targets:[],unknown:false};service.invalidate();
 await assert.rejects(service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:packet.sequence,action:{kind:'key',key:'Tab'}},'a'),/stale/);
 const fresh=await service.next(next(s.subscription_id,packet.sequence),'a');assert(fresh.reset);
 await assert.rejects(service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:packet.sequence,action:{kind:'key',key:'Tab'}},'a'),/stale/);
});
test('MP-08/MP-11: layout drift falls back to protected tiles and drops the entire DOM subtree',async()=>{
 const {service}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),first=await service.next(next(s.subscription_id),'a');
 const packet=await service.next(next(s.subscription_id,first.sequence,['n1']),'a');const tile=packet.nodes.find(n=>n.kind==='tile');assert.deepEqual(tile.children,[]);assert.deepEqual(packet.removed,['n1','n2']);assert.equal(packet.tiles.length,1);
 await assert.rejects(service.next(next(s.subscription_id,packet.sequence,['foreign']),'a'),/drift/);
});
test('MP-11: stream count and canonical geometry stay bounded',async()=>{
 const {service}=fixture();for(let i=0;i<8;i++)await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 await assert.rejects(service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),/bounds/);
 const f=fixture();await f.service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');await assert.rejects(f.service.subscribe({tab_id:'t',generation:1,device_scale_factor:2},'a'),/canonical/);
});
test('MP-11: resource service reads only Chromium-loaded resources, strips URL, rejects SVG and policy-protected bytes',async()=>{
 let reads=0;const png=encodePng(20,20,Buffer.alloc(20*20*4,120));
 const connection={async send(method,params){if(method==='Page.getResourceTree')return {frameTree:{frame:{id:'frame'},resources:[{url:'https://fixture/image'},{url:'https://fixture/svg'}]}};if(method==='Page.getResourceContent'){reads++;return {base64Encoded:true,content:params.url.endsWith('/svg')?Buffer.from('<svg onload="alert(1)"/>').toString('base64'):png};}throw Error('unexpected');}};
 const descriptors=[{key:'r0',url:'https://fixture/image',kind:'image'},{key:'r1',url:'https://fixture/svg',kind:'image'},{key:'r2',url:'http://private-address/unloaded',kind:'image'}];
 const result=await materializeMirrorResources(connection,'s',descriptors,[]);assert.equal(reads,2);assert.equal(result.resources.size,1);assert.equal(result.mapped.get('r1'),null);assert.equal(result.mapped.get('r2'),null);assert(!JSON.stringify([...result.resources.values()]).includes('https://fixture'));
 await assert.rejects(materializeMirrorResources(connection,'s',descriptors,['synthetic-private-value']),/protected/);
});

test('MP-11: resource cache is epoch bounded and oversized decoded images never reach the client',async()=>{
 const cache=new Map([['old',{resource_id:'a'.repeat(64),mime_type:'image/png',data_base64:'AA=='}]]);
 const image=Buffer.from(encodePng(1,1,Buffer.alloc(4)),'base64');image.writeUInt32BE(100000,16);
 const connection={async send(method){return method==='Page.getResourceTree'?{frameTree:{frame:{id:'frame'},resources:[{url:'https://fixture/huge'}]}}:{base64Encoded:true,content:image.toString('base64')};}};
 const result=await materializeMirrorResources(connection,'s',[{key:'r0',url:'https://fixture/huge',kind:'image'}],[],cache);
 assert.equal(result.resources.size,0);assert.equal(result.mapped.get('r0'),null);assert.equal(cache.size,0);
});

test('MP-11: native App capability views never become an alternate DOM frontend',async()=>{
 const {service,host}=fixture();host.browser.appTabs={apps:new Map([['app',{targetId:'target'}]])};
 await assert.rejects(service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),/App capability/);
});
