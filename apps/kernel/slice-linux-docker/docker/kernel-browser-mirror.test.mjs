// MP-08/MP-10/MP-11: service epochs, privacy, bounded streams and resource admission.
import test from 'node:test';
import assert from 'node:assert/strict';
import { MirrorService } from './kernel-browser-mirror.mjs';
import { materializeMirrorResources,mirrorHash } from './kernel-browser-mirror-resources.mjs';
import { encodePng } from './kernel-browser-pixels.mjs';
function fixture() {
  const tab={tab_id:'t',target_id:'target',document_id:'d'},state={document:'d',snapshot:{root:'n1',nodes:[{id:'n1',parent:null,children:['n2'],kind:'element',tag:'div',box:{x:0,y:0,width:80,height:40}},{id:'n2',parent:'n1',children:[],kind:'text',text:'fixture'}],fonts:[],resources:[],scroll:{x:0,y:0},focused:null,selection:null}};
  const connection={async send(method,params){
    if(method==='Target.createTarget')return {targetId:'css-initial'};
    if(method==='Target.closeTarget')return {success:true};
    if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'frame',loaderId:state.document}}};
    if(method==='Page.getResourceTree')return {frameTree:{frame:{id:'frame'},resources:[]}};
    if(method==='DOM.getDocument')return {root:{nodeId:1,children:[]}};
    if(method==='DOM.querySelectorAll')return {nodeIds:[]};
    if(method==='Emulation.setDeviceMetricsOverride')return {};
    if(method==='Runtime.evaluate')return {result:{value:params.expression.includes('.read(')?structuredClone(state.snapshot):params.expression.includes('Object.fromEntries([...style]')?{}:true}};
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

test('MP-08/MP-11: region fallback removes selection metadata referring to its hidden descendants',async()=>{
 const {service,state}=fixture();state.snapshot.nodes[0].children=['n3'];state.snapshot.nodes[1].parent='n3';state.snapshot.nodes.push({id:'n3',parent:'n1',children:['n2'],kind:'element',tag:'p',box:{x:0,y:0,width:80,height:40}});state.snapshot.selection={anchor_id:'n2',anchor_offset:0,focus_id:'n2',focus_offset:3};
 const subscribed=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');const first=await service.next(next(subscribed.subscription_id),'a');assert(first.selection);
 const fallback=await service.next(next(subscribed.subscription_id,first.sequence,['n3']),'a');assert.equal(fallback.selection,null);assert(fallback.removed.includes('n2'));assert.equal(fallback.nodes.find(n=>n.id==='n3').kind,'tile');
});

test('MP-08/MP-11: isolated-world deltas keep a private full base before public protection transforms',async()=>{
 const {service,state}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 const first=await service.next(next(s.subscription_id),'a');
 state.snapshot={...state.snapshot,nodes:[{...state.snapshot.nodes[1],text:'incremental'}],incremental:true,removed:[]};
 const patch=await service.next(next(s.subscription_id,first.sequence),'a');
 assert.equal(service.streams.get(s.subscription_id).previous.nodes.length,2);
 assert.deepEqual(patch.nodes.map(n=>n.id),['n2']);
 const raw=service.streams.get(s.subscription_id).observed;
 assert.equal(raw.nodes[0].kind,'element');
 await service.next(next(s.subscription_id,patch.sequence,['n1']),'a');
 assert.equal(raw.nodes[0].kind,'element','public drift cannot corrupt isolated delta base');
});

test('MP-08/MP-10: cached canonical hashes retain original bytes across styles, order and removals',async()=>{
 const {MirrorTreeHasher}=await import('./kernel-browser-mirror-resources.mjs');
 const hasher=new MirrorTreeHasher(),{state}=fixture();let source={...state.snapshot};delete source.resources;source.selection=null;
 assert.equal(hasher.hash(source),mirrorHash(source));
 source.nodes[0].style={color:'rgb(1, 2, 3)',display:'block'};assert.equal(hasher.hash(source),mirrorHash(source));
 source.nodes[0].style={display:'block',color:'rgb(1, 2, 3)'};assert.equal(hasher.hash(source),mirrorHash(source));
 source.nodes=source.nodes.slice(0,1);source.nodes[0].children=[];assert.equal(hasher.hash(source),mirrorHash(source));
});

// MP-11: default CSS is obtained from a disposable blank target, never the page.
test('MP-11: CSS initial probe always closes its trusted blank target',async()=>{
 const {mirrorInitialStyles}=await import('./kernel-browser-mirror-styles.mjs');
 for(const fails of [false,true]){
  const calls=[];const connection={async send(method,params){calls.push({method,params});return method==='Target.createTarget'?{targetId:'blank-css'}:{success:true};}};
  const browser={async resolvePageTarget(id){assert.equal(id,'blank-css');return {sessionId:'blank-session',connection:{async send(method,params,session){assert.equal(method,'Runtime.evaluate');assert.equal(session,'blank-session');assert(params.expression.includes("node.style.all='initial'"));if(fails)throw Error('probe failed');return {result:{value:{margin:'0px'}}};}}};}};
  if(fails)await assert.rejects(mirrorInitialStyles(browser,connection),/probe failed/);else assert.deepEqual(await mirrorInitialStyles(browser,connection),{margin:'0px'});
  assert.deepEqual(calls,[{method:'Target.createTarget',params:{url:'about:blank',background:true}},{method:'Target.closeTarget',params:{targetId:'blank-css'}}]);
 }
});

test('MP-10/MP-11: fully opaque foreign/closed regions do not require compositor readback',async()=>{
 for(const reason of ['cross_origin_frame','opaque_shadow']){
  const {service,state,host}=fixture();state.snapshot.nodes[1]={id:'n2',parent:'n1',children:[],kind:'tile',tag:'img',box:{x:5,y:5,width:30,height:20},reason};
  host.screenshot=async()=>{throw Error('opaque region must not capture')};
  const s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:2},'a');const packet=await service.next(next(s.subscription_id),'a');assert.equal(packet.nodes[1].reason,reason);assert.deepEqual(packet.tiles,[]);
 }
});

test('MP-11: policy changes during snapshot or capture fence the reply and private base',async()=>{
 for(const stage of ['snapshot','capture']){
  const {service,state,host}=fixture(),sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
  const change=()=>{host.protection={values:['fixture'],targets:[],unknown:false};service.invalidate();};
  if(stage==='snapshot'){const evaluate=service.evaluate.bind(service);service.evaluate=async(world,expression)=>{const result=await evaluate(world,expression);if(expression.includes('.read('))change();return result;};}
  else{state.snapshot.nodes[0].children.push('n3');state.snapshot.nodes.push({id:'n3',parent:'n1',children:[],kind:'tile',tag:'canvas',box:{x:0,y:0,width:10,height:10},reason:'opaque_media'});const capture=host.screenshot;host.screenshot=async(...args)=>{const result=await capture(...args);change();return result;};}
  await assert.rejects(service.next(next(sub.subscription_id),'a'),/policy|stale/);const stream=service.streams.get(sub.subscription_id);assert.equal(stream.sequence,0);assert.equal(stream.previous,null);assert.equal(stream.observed,null);
 }
});
test('MP-11: delayed mirror focus cannot dispatch after protection invalidation',async()=>{
 const {service,host}=fixture(),sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),packet=await service.next(next(sub.subscription_id),'a');
 const resolved=await service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:sub.subscription_id,sequence:packet.sequence,action:{kind:'focus',node_id:'n1'}},'a');host.protection={values:['fixture'],targets:[],unknown:false};service.invalidate();await assert.rejects(resolved.perform(),/policy|stale/);
});
