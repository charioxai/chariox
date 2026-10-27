// MP-08/MP-10/MP-11: service epochs, privacy, bounded streams and resource admission.
import test from 'node:test';
import assert from 'node:assert/strict';
import { SampleLane } from './kernel-browser-sample-lane.mjs';
import { MirrorService } from './kernel-browser-mirror.mjs';
import { materializeMirrorResources,mirrorHash } from './kernel-browser-mirror-resources.mjs';
import { encodePng,decodePng } from './kernel-browser-pixels.mjs';
import { displayGeometry as geometry,hostViewport } from './kernel-browser-geometry.mjs';
import { spawnSync } from 'node:child_process';
// These fixtures exercise the host with the mirror and display kill-switched; tests that need them on set "1" themselves.
process.env.CHARIOX_KERNEL_BROWSER_MIRROR ??= '0'; process.env.CHARIOX_KERNEL_BROWSER_DISPLAY ??= '0';
function fixture() {
  const tab={tab_id:'t',target_id:'target',document_id:'d'},state={document:'d',snapshot:{root:'n1',nodes:[{id:'n1',parent:null,children:['n2'],kind:'element',tag:'div',box:{x:0,y:0,width:80,height:40}},{id:'n2',parent:'n1',children:[],kind:'text',text:'fixture'}],fonts:[],resources:[],scroll:{x:0,y:0},focused:null,selection:null}};
  const calls=[],connection={async send(method,params){
    calls.push({method,params});
    if(method==='Target.createTarget')return {targetId:'css-initial'};
    if(method==='Target.closeTarget')return {success:true};
    if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'frame',loaderId:state.document}}};
    if(method==='Page.getResourceTree')return {frameTree:{frame:{id:'frame'},resources:[]}};
    if(method==='DOM.getDocument')return {root:{nodeId:1,children:[]}};
    if(method==='DOM.querySelectorAll')return {nodeIds:[]};
    if(method==='Emulation.setDeviceMetricsOverride')return {};
    if(method==='Page.createIsolatedWorld')return {executionContextId:1};
    if(method==='Page.getLayoutMetrics')return {cssVisualViewport:{zoom:1}};
    if(method==='Runtime.evaluate')return {result:{value:params.expression.includes('dpr:devicePixelRatio')?{dpr:1,width:geometry.width,height:geometry.height,visible:true}:params.expression.includes('.read(')?structuredClone(state.snapshot):params.expression.includes('Object.fromEntries([...style]')?{}:true}};
    throw Error(`unexpected CDP method ${method}`);
  }};
  const host={generation:1,scales:new Map(),viewport:()=>hostViewport(),protection:{values:[],targets:[],unknown:false},async target(){return {...tab,document_id:state.document};},async displayTarget(){return {...tab,document_id:state.document};},browser:{async resolvePageTarget(){return {connection,sessionId:'session'};},async ensureFocusWorld(){return {contextId:1};}},async screenshot(_tab,clip){const scale=hostViewport().device_scale_factor,width=(clip?.width??geometry.width)*scale,height=(clip?.height??geometry.height)*scale;return {data_base64:encodePng(width,height,Buffer.alloc(width*height*4,100)),protected_regions:[]};}};
  return {host,state,calls,service:new MirrorService(host)};
}
const next=(subscription_id,after_sequence=0,drift_nodes=[])=>({subscription_id,generation:1,after_sequence,drift_nodes});
test('MP-08/MP-11: registered and retired matching text uses visible tiles before hashing incremental bases',async()=>{
 const {service,state,host}=fixture();host.protection.values=['fixture'];
 state.snapshot.nodes[0].parent='n3';state.snapshot.nodes.unshift({id:'n3',parent:null,children:['n1'],kind:'element',tag:'html',box:{x:0,y:0,width:1280,height:800}});state.snapshot.root='n3';
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 const first=await service.next(next(sub.subscription_id),'a');
 assert.equal(first.nodes.find(n=>n.id==='n1').kind,'tile');
 assert(!JSON.stringify(first.nodes).includes('fixture'),'MP-11 ordinary matching text never enters the structured wire tree');
 assert.equal(first.tiles.length,1,'MP-08 ordinary matching pixels stay visible');
 assert.equal(first.hash,mirrorHash({root:first.root,nodes:first.nodes,fonts:first.fonts,scroll:first.scroll,focused:first.focused,selection:first.selection}));
 state.snapshot.nodes[2].text='ordinary';
 const delta=await service.next(next(sub.subscription_id,first.sequence),'a');
 assert(!delta.reset);assert.equal(delta.base_sequence,first.sequence);assert.equal(delta.nodes.find(n=>n.id==='n1').kind,'element');
 const records=new Map(first.nodes.map(n=>[n.id,n]));for(const id of delta.removed)records.delete(id);for(const n of delta.nodes)records.set(n.id,n);
 assert.equal(delta.hash,mirrorHash({root:delta.root,nodes:[records.get('n3'),records.get('n1'),records.get('n2')],fonts:delta.fonts,scroll:delta.scroll,focused:delta.focused,selection:delta.selection}));
 state.snapshot.nodes[1].attributes={title:'fixture'};state.snapshot.nodes[1].pseudo={'::before':{text:'FIXTURE',style:{color:'black'}}};
 const retired=await service.next(next(sub.subscription_id,delta.sequence),'a');
 assert(retired.nodes.some(n=>n.reason==='observer_bounds_or_unavailable'),'MP-08 unmeasured generated text uses full compositor pixels');assert(retired.nodes.every(n=>n.attributes===undefined&&n.pseudo===undefined));
 clearInterval(service.expiry);service.clear();
});
test('MP-08/MP-11: matching styles fonts and overflowing text use protected full compositor fallback',async()=>{
 for(const seam of ['style','font','overflow','pseudo']) {
  const {service,state,host}=fixture();host.protection.values=['fixture'];
  if(seam==='style')state.snapshot.nodes[0].style={'font-family':'fixture'};
  if(seam==='font')state.snapshot.fonts=[{family:'fixture',resource:'missing'}];
  if(seam==='pseudo')state.snapshot.nodes[0].pseudo={'::after':{text:'fixture',style:{'white-space':'nowrap','overflow':'visible'}}};
  if(seam==='overflow')state.snapshot.nodes[1].box={x:70,y:0,width:80,height:40};
  const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),packet=await service.next(next(sub.subscription_id),'a');
  assert(packet.nodes.some(n=>n.reason==='observer_bounds_or_unavailable'));assert(!JSON.stringify(packet.nodes).includes('fixture'));assert.equal(packet.tiles.length,1);
  clearInterval(service.expiry);service.clear();
 }
});
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
 const f=fixture();await f.service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');assert.equal((await f.service.subscribe({tab_id:'t',generation:1,device_scale_factor:2},'a')).device_scale_factor,1);
});
test('MP-11: resource service reads only Chromium-loaded resources, strips URL, rejects SVG and policy-protected bytes',async()=>{
 let reads=0;const png=encodePng(20,20,Buffer.alloc(20*20*4,120));
 const connection={async send(method,params){if(method==='Page.getResourceTree')return {frameTree:{frame:{id:'frame'},resources:[{url:'https://fixture/image'},{url:'https://fixture/svg'}]}};if(method==='Page.getResourceContent'){reads++;return {base64Encoded:true,content:params.url.endsWith('/svg')?Buffer.from('<svg onload="alert(1)"/>').toString('base64'):png};}throw Error('unexpected');}};
 const descriptors=[{key:'r0',url:'https://fixture/image',kind:'image'},{key:'r1',url:'https://fixture/svg',kind:'image'},{key:'r2',url:'http://private-address/unloaded',kind:'image'}];
 const result=await materializeMirrorResources(connection,'s',descriptors,[]);assert.equal(reads,2);assert.equal(result.resources.size,1);assert.equal(result.mapped.get('r1'),null);assert.equal(result.mapped.get('r2'),null);assert(!JSON.stringify([...result.resources.values()]).includes('https://fixture'));
 assert.equal((await materializeMirrorResources(connection,'s',descriptors,['synthetic-private-value'])).resources.size,1,'MP-11 value registration alone cannot mask media');
});

test('MP-11: resource cache is epoch bounded and oversized decoded images never reach the client',async()=>{
 const cache=new Map([['old',{resource_id:'a'.repeat(64),mime_type:'image/png',data_base64:'AA=='}]]);
 const image=Buffer.from(encodePng(1,1,Buffer.alloc(4)),'base64');image.writeUInt32BE(100000,16);
 const connection={async send(method){return method==='Page.getResourceTree'?{frameTree:{frame:{id:'frame'},resources:[{url:'https://fixture/huge'}]}}:{base64Encoded:true,content:image.toString('base64')};}};
 const result=await materializeMirrorResources(connection,'s',[{key:'r0',url:'https://fixture/huge',kind:'image'}],[],cache);
 assert.equal(result.resources.size,0);assert.equal(result.mapped.get('r0'),null);assert.equal(cache.size,0);
});

test('MP-08/MP-11: App tabs mirror like any page under the same observation protection',async()=>{
 const {service,host}=fixture();host.browser.appTabs={apps:new Map([['app',{targetId:'target'}]]),async flushDrafts(){}};host.protection.values=['fixture'];
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 const packet=await service.next(next(sub.subscription_id),'a');
 assert.equal(packet.tab_id,'t');assert(!JSON.stringify(packet.nodes).includes('fixture'));
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

test('MP-10/MP-11: MP-11 foreign/closed regions display compositor pixels',async()=>{
 for(const reason of ['cross_origin_frame','opaque_shadow']){
  const {service,state,host}=fixture();state.snapshot.nodes[1]={id:'n2',parent:'n1',children:[],kind:'tile',tag:'img',box:{x:5,y:5,width:30,height:20},reason};
  let captures=0;const capture=host.screenshot;host.screenshot=async(...args)=>{captures++;return capture(...args)};
  const s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:2},'a');const packet=await service.next(next(s.subscription_id),'a');assert.equal(packet.nodes[1].reason,reason);assert(packet.tiles.length>0);assert.equal(captures,1);
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

// MP-08/MP-11: exact Cloud admission window; sequence refusal dispatches nothing.
test('MP-11: previous issued input epoch remains admitted while next packet is in flight',async()=>{
 const {service}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 const first=await service.next(next(s.subscription_id),'a');await service.next(next(s.subscription_id,first.sequence),'a');
 const result=await service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:first.sequence,action:{kind:'click',node_id:'n1'}},'a');assert(result.input);
});
test('MP-11: input epoch window is exactly latest eight issued sequences and two seconds',async()=>{
 const {service}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 for(let sequence=0;sequence<9;sequence++)await service.next(next(s.subscription_id,sequence),'a');
 const input=sequence=>service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence,action:{kind:'key',key:'Tab'}},'a');
 for(const sequence of [0,1,10])await assert.rejects(input(sequence),/MP-11: stale mirror input epoch/);
 for(let sequence=2;sequence<=9;sequence++)assert((await input(sequence)).input);
 const stream=service.streams.get(s.subscription_id);for(const epoch of stream.epochs)epoch.issuedAt=service.now()-2001;
 await assert.rejects(input(9),/MP-11: stale mirror input epoch/);assert.equal(stream.epochs.length,0);
});
test('MP-11: old epoch cannot retarget changed, removed, protected nodes or selection offsets',async()=>{
 for(const change of ['changed','removed','protected']){
  const {service,state}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');const first=await service.next(next(s.subscription_id),'a');
  if(change==='changed')state.snapshot.nodes[0].box.x=2;
  if(change==='removed'){state.snapshot.nodes[0].children=[];state.snapshot.nodes.pop();}
  if(change==='protected'){state.snapshot.nodes[1]={...state.snapshot.nodes[1],kind:'mask',text:''};}
  await service.next(next(s.subscription_id,first.sequence),'a');
  const action=change==='changed'?{kind:'click',node_id:'n1'}:{kind:'selection',anchor_id:'n2',anchor_offset:0,focus_id:'n2',focus_offset:7};
  await assert.rejects(service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:first.sequence,action},'a'),error=>!error.message.includes('stale mirror input epoch'));
 }
});
test('MP-11: document/policy failures never use the sequence-only retry marker',async()=>{
 const {service,host,state}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');const first=await service.next(next(s.subscription_id),'a');
 const input=()=>service.resolveInput({tab_id:'t',document_id:state.document},{subscription_id:s.subscription_id,sequence:first.sequence,action:{kind:'key',key:'Tab'}},'a');
 state.document='new';await assert.rejects(input(),error=>!error.message.includes('stale mirror input epoch'));state.document='d';
 host.protection={values:[],targets:[],unknown:false};service.invalidate();await assert.rejects(input(),error=>!error.message.includes('stale mirror input epoch'));
});

test('MP-11: sequence-only refusal never enters native focus capture or dispatch',async()=>{
 const {inputHostTab}=await import('./kernel-browser-input.mjs');let captures=0,dispatches=0;
 const connection={async send(method){assert.equal(method,'Page.getFrameTree');return {frameTree:{frame:{id:'frame',loaderId:'d'}}};}};
 const browser={async resolvePageTarget(){return {connection,sessionId:'s'}},inputCapture:{async run(){captures++;assert.fail('focus emulation must not run')}}};
 await assert.rejects(inputHostTab(browser,{target_id:'t',document_id:'d'},{kind:'mirror'},{onDispatch(){dispatches++},resolveMirror:async()=>{throw Error('MP-11: stale mirror input epoch')}}),/MP-11: stale mirror input epoch/);assert.equal(captures,0);assert.equal(dispatches,0);
});
test('MP-08/MP-11: private CSS palettes preserve exact public shapes and reject missing references',async()=>{
 const {service,state}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');state.snapshot.nodes[0].style_index=0;state.snapshot.styles=[{all:'initial',color:'rgb(1, 2, 3)'}];
 const packet=await service.next(next(s.subscription_id),'a');assert.deepEqual(packet.nodes[0].style,state.snapshot.styles[0]);assert(!JSON.stringify(packet).includes('style_index'));assert(!('styles'in packet));
 state.snapshot.nodes[0].style_index=2;await assert.rejects(service.next(next(s.subscription_id,packet.sequence),'a'),/private CSS reference/);
});

test('MP-08/MP-10/MP-11: idle tile credit refines with full native readback after a motion crop',async()=>{
 const {service,state,host}=fixture(),clips=[];state.snapshot.nodes[1]={id:'n2',parent:'n1',children:[],kind:'tile',tag:'img',box:{x:10,y:10,width:20,height:10},reason:'opaque_media'};host.inputEpochs=new Map();const screenshot=host.screenshot;host.screenshot=async(tab,clip)=>{clips.push(clip);return screenshot(tab,clip)};
 const s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:2},'a');const first=await service.next(next(s.subscription_id),'a');await service.next(next(s.subscription_id,first.sequence),'a');assert(clips[0]);assert.equal(clips[1],null);
 host.inputEpochs.set('t',1);await service.next(next(s.subscription_id,2),'a');assert(clips[2]);
});

test('MP-08/MP-10/MP-11: typing defers full-frame refinement until input is quiet',async()=>{
 const {service,state,host}=fixture(),clips=[];state.snapshot.nodes[1]={id:'n2',parent:'n1',children:[],kind:'tile',tag:'img',box:{x:10,y:10,width:20,height:10},reason:'native_control'};host.inputEpochs=new Map([['t',1]]);host.inputChangedAt=new Map([['t',performance.now()]]);const screenshot=host.screenshot;host.screenshot=async(tab,clip)=>{clips.push(clip);return screenshot(tab,clip)};
 const s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:2},'a');const first=await service.next(next(s.subscription_id),'a');const second=await service.next(next(s.subscription_id,first.sequence),'a');
 assert(clips[0]);assert(clips[1],'a caret blink while typing keeps the crop');
 host.inputChangedAt.set('t',performance.now()-2000);await service.next(next(s.subscription_id,second.sequence),'a');assert.equal(clips[2],null,'settled input refines');
});

test('MP-08/MP-11: kernel refuses overlapping credits instead of racing private observer bases',async()=>{
 const {service}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');let unblock;const held=new Promise(resolve=>{unblock=resolve}),world=service.world.bind(service);service.world=async(...args)=>{await held;return world(...args)};
 const first=service.next(next(s.subscription_id),'a');await assert.rejects(service.next(next(s.subscription_id),'a'),/credit already outstanding/);unblock();assert.equal((await first).sequence,1);assert.equal(service.streams.get(s.subscription_id).busy,false);
});

test('MP-11: coordinate admission binds the observed leaf and carries live ancestor validation',async()=>{
 const {service,state}=fixture();state.snapshot.nodes[1]={id:'n2',parent:'n1',children:[],kind:'element',tag:'input',box:{x:5,y:5,width:20,height:20},attributes:{type:'text'}};
 const s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),packet=await service.next(next(s.subscription_id),'a');
 const evaluate=service.evaluate.bind(service);let expected;
 service.evaluate=async(world,expression)=>{if(expression.includes('.coordinateTarget(')){const args=JSON.parse('['+expression.slice(expression.indexOf('(')+1,-1)+']');expected=args[2];return 'n2';}return evaluate(world,expression)};
 const result=await service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:packet.sequence,action:{kind:'coordinate',input:{kind:'click',x:10,y:10}}},'a');
 assert.equal(result.input.kind,'click');assert.equal(expected,undefined,'MP-11 live guard runs at dispatch, after input focus emulation');await result.guard();assert.deepEqual(expected.map(n=>n.id),['n2','n1']);assert.deepEqual(expected[0].box,state.snapshot.nodes[1].box);
 service.evaluate=async()=> 'n1';
 const changed=await service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:packet.sequence,action:{kind:'coordinate',input:{kind:'click',x:10,y:10}}},'a');await assert.rejects(changed.guard(),error=>!error.message.includes('stale mirror input epoch'));
});

test('MP-11: ambiguous overlapping coordinate leaves refuse without entering live dispatch',async()=>{
 const {service,state}=fixture();state.snapshot.nodes[0].children=['n2','n3'];
 for(const id of ['n2','n3']){const node={id,parent:'n1',children:[],kind:'element',tag:'div',box:{x:5,y:5,width:20,height:20}};if(id==='n2')state.snapshot.nodes[1]=node;else state.snapshot.nodes.push(node);}
 const s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),packet=await service.next(next(s.subscription_id),'a');service.world=async()=>assert.fail('must refuse before live work');
 await assert.rejects(service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:packet.sequence,action:{kind:'coordinate',input:{kind:'click',x:10,y:10}}},'a'),/ambiguous/);
});

// MP-11: native-focus keys use an admitted recent epoch, never a painted-focus
// equality shortcut. Live target validation must precede the first key event.
test('MP-08/MP-11: native keyboard validates observed focus and pairs key release after Tab',async()=>{
 const {service,state,host}=fixture();state.snapshot.nodes[0].children=['n2','n3'];
 state.snapshot.nodes[1]={id:'n2',parent:'n1',children:[],kind:'element',tag:'input',box:{x:5,y:5,width:20,height:20}};
 state.snapshot.nodes.push({id:'n3',parent:'n1',children:[],kind:'element',tag:'button',box:{x:30,y:5,width:20,height:20}});state.snapshot.focused='n2';
 const s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),first=await service.next(next(s.subscription_id),'a');
 state.snapshot.focused='n3';await service.next(next(s.subscription_id,first.sequence),'a');
 await assert.rejects(service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:first.sequence,action:{kind:'key',key:'Tab'}},'a'),/changed mirror focus/);
 const calls=[];service.evaluate=async(_world,expression)=>{if(expression.includes('.activeTarget(')){calls.push(JSON.parse('['+expression.slice(expression.indexOf('(')+1,-1)+']'));return 'n3';}return true;};
 const resolve=()=>service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:first.sequence,action:{kind:'coordinate',input:{kind:'key',key:'Tab'}}},'a');
 const result=await resolve();assert.equal(calls.length,0,'native focus read waits until physical dispatch');await result.guard();
 assert.equal(calls.length,2,'native key validates current observed focus');assert.equal(calls[0][1],false,'keys admit noneditable native focus');assert.deepEqual(calls[1][0].map(n=>n.id),['n3','n1']);
 service.evaluate=async()=>assert.fail('paired key release must not recheck focus changed by keyDown');await result.guard();
 host.protection={values:[],targets:[],unknown:false};service.invalidate();await assert.rejects(result.guard(),/stale|policy/);
});

test('MP-11: native keyboard unknown/protected focus refuses with no retry marker',async()=>{
 const {service}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),first=await service.next(next(s.subscription_id),'a');
 service.evaluate=async()=> 'unknown';
 const result=await service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:first.sequence,action:{kind:'coordinate',input:{kind:'key',key:'Backspace'}}},'a');
 await assert.rejects(result.guard(),error=>!error.message.includes('stale mirror input epoch'));
});

// MP-08/MP-10/MP-11: attaching an observer never selects shared geometry.
for(const dpr of [1,2])test(`MP-10 mirror preserves canonical view at viewer DPR${dpr}`,async()=>{
 const {service,calls}=fixture();await service.subscribe({tab_id:'t',generation:1,device_scale_factor:dpr},'a');
 assert.equal(calls.filter(c=>c.method==='Emulation.setDeviceMetricsOverride').length,0);
});

// MP-11: no new packet is produced after the live page/native focus moves.
test('MP-11: plain keys fence live focus before keyDown and pair release after focus moves',async()=>{
 for(const key of ['Enter','Backspace','Delete','Tab']) {
  const {service,state}=fixture();state.snapshot.focused='n1';
  const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
  const packet=await service.next(next(sub.subscription_id),'a');
  let live='unobserved',calls=0;
  service.evaluate=async(_world,expression)=>{assert(expression.includes('.activeTarget('));calls++;return live;};
  const resolve=()=>service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:sub.subscription_id,sequence:packet.sequence,action:{kind:'key',key}},'a');
  const moved=await resolve();assert.equal(calls,0,'MP-11: validate at dispatch');
  await assert.rejects(async()=>moved.guard(),/focus|target/);
  live='n1';const admitted=await resolve();await admitted.guard();assert.equal(calls,3);
  live='unobserved';service.evaluate=async()=>assert.fail('MP-11: keyUp stays paired after keyDown moved focus');await admitted.guard();
 }
});

test('MP-11: plain key refuses a display fallback without painted DOM focus',async()=>{
 const {service}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 const evaluate=service.evaluate.bind(service);
 service.evaluate=async(world,expression)=>{if(expression.includes('.read('))throw Error('MP-11: observer unavailable');return evaluate(world,expression);};
 const packet=await service.next(next(s.subscription_id),'a');
 await assert.rejects(service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:packet.sequence,action:{kind:'key',key:'Backspace'}},'a'),/observed focus/);
});
test('MP-11 mirror attaches to another canonical geometry without overriding it',async()=>{
 const {execFileSync}=await import('node:child_process');
 const script=`import {MirrorService} from ${JSON.stringify(new URL('./kernel-browser-mirror.mjs',import.meta.url).href)};
 const calls=[];const host={generation:1,scales:new Map(),viewport:()=>({device_scale_factor:1}),async target(){return {tab_id:'t',target_id:'x',document_id:'d'}},browser:{async resolvePageTarget(){return {connection:{send:async method=>{calls.push(method);return {}}},sessionId:'s'}}}};
 try{await new MirrorService(host).subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');console.log('subscribed:'+calls.join())}catch(error){console.log('refused:'+calls.join())}`;
 const output=execFileSync(process.execPath,['--input-type=module','-e',script],{env:{...process.env,CHARIOX_BROWSER_DISPLAY_GEOMETRY:'1920x1080'}}).toString().trim();
 assert.equal(output,'subscribed:','an observer sends no device metrics override');
});

// MP-08/MP-10/MP-11: a viewer cannot resize a kernel-owned native browser.
test('MP-10 DPR1 and DPR2 mirrors share native density with video and provider observations',async()=>{
 const before=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try {
  const {service,host,calls}=fixture();host.scales.set('t',2);
  for(const viewerDpr of [1,2]){
   const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:viewerDpr},'a');
   assert.equal(sub.device_scale_factor,2);assert.equal(host.scales.get('t'),2);
  }
  assert.equal(calls.filter(c=>c.method==='Emulation.setDeviceMetricsOverride').length,0);
  service.clear();clearInterval(service.expiry);
 }finally{if(before===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=before;}
});

// MP-08/MP-10/MP-11: observer fallback, crops and wire geometry share the host viewport.
test('MP-10 canonical mirror geometry survives attachment and full fallback',async()=>{
 const {service,host,calls}=fixture(),viewport=hostViewport();
 host.scales.set('t',viewport.device_scale_factor);
 const evaluate=service.evaluate.bind(service);
 service.evaluate=async(world,expression)=>{if(expression.includes('.read('))throw Error('MP-10: observer unavailable');return evaluate(world,expression)};
 try {
  for(const viewerDpr of [1,2]) {
   const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:viewerDpr},'a');
   const packet=await service.next(next(sub.subscription_id),'a');
   assert.equal(calls.filter(c=>c.method==='Emulation.setDeviceMetricsOverride').length,0,'attaching and reading never resize the shared page');
   assert.equal(packet.css_width,viewport.css_width);assert.equal(packet.css_height,viewport.css_height);
   assert.equal(packet.device_scale_factor,viewport.device_scale_factor);
   const tile=packet.nodes.find(n=>n.kind==='tile');
   assert.deepEqual(tile.box,{x:0,y:0,width:viewport.css_width,height:viewport.css_height});
   assert.equal(packet.tiles.length,1);
   const pixels=decodePng(packet.tiles[0].data_base64,viewport.device_scale_factor);
   assert.equal(pixels.width,viewport.desktop_pixel_width);assert.equal(pixels.height,viewport.desktop_pixel_height);
  }
 }finally{service.clear();clearInterval(service.expiry)}
});
test('MP-10 canonical mirror geometry at 1920x1080',()=>{
 const run=spawnSync(process.execPath,['--test','--test-name-pattern=^MP-10 canonical mirror geometry survives',import.meta.filename],{env:{...process.env,NODE_TEST_CONTEXT:undefined,CHARIOX_BROWSER_DISPLAY_GEOMETRY:'1920x1080',CHARIOX_KERNEL_BROWSER_DISPLAY:'1'},encoding:'utf8'});
 assert.match(run.stdout,/ok 1 - MP-10 canonical mirror geometry survives/);
 assert.equal(run.status,0,run.stdout+run.stderr);
});

test('MP-08/MP-10/MP-11: a v2 view retains the existing render lease until close or clear',async()=>{
 const {service,host}=fixture();let active=0,released=0;
 host.browser.inputCapture={async hold(){active++;let closed=false;return async()=>{if(!closed){closed=true;active--;released++}}}};
 const first=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1,wire:2},'terminal:one');assert.equal(active,1);
 service.drop(first.subscription_id);await Promise.resolve();assert.equal(active,0);assert.equal(released,1);
 await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1,wire:2},'terminal:two');assert.equal(active,1);
 service.clear();await Promise.resolve();assert.equal(active,0);assert.equal(released,2);
});

// MP-08/MP-10 (owner A/B 2026-10-14, finding 8): a new Tab's v2 view subscribes without an all-tab reconcile.
test('MP-08/MP-10 a v2 subscription reads its own Tab without an all-tab reconcile', async () => {
 const {service,host}=fixture();host.browser.inputCapture={async hold(){return async()=>{}}};let reconciles=0;const target=host.target;host.target=async(...args)=>{reconciles++;return target(...args)};
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1,wire:2},'a');
 assert.ok(sub.subscription_id);assert.equal(reconciles,0);
 await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');assert.equal(reconciles,1,'wire 1 keeps its reconcile');
});
// MP-08/MP-10 (BROWSER-UX-05): a Tab that just opened has a busy renderer; its first subscription must not ask it
// for the document twice. The page session's navigation events (the cached document) name it.
test('MP-08/MP-10 a v2 subscription of a document the page session already named asks the renderer nothing', async () => {
 const {service,host,calls,state}=fixture();host.browser.inputCapture={async hold(){return async()=>{}}};service.v2.begin=()=>{};
 host.browser.documentIdsByTarget=new Map([['target','d']]);
 await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1,wire:2},'a');
 assert.equal(calls.filter(call=>call.method==='Page.getFrameTree').length,0,'no renderer round trip');
 host.browser.documentIdsByTarget=new Map([['target','other']]);
 await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1,wire:2},'a');
 assert.equal(calls.filter(call=>call.method==='Page.getFrameTree').length,2,'MP-11: any other cached document asks the renderer (before and after the render lease)');
 host.browser.documentIdsByTarget=new Map();state.document='moved';host.displayTarget=async()=>({tab_id:'t',target_id:'target',document_id:'d'});
 await assert.rejects(service.subscribe({tab_id:'t',generation:1,device_scale_factor:1,wire:2},'a'),/moved away/,'MP-11: a moved document is still refused by the renderer');
});
// MP-08/MP-10/MP-11: merge #958's owned desktop with the v2 mirror observer.
test('MP-08 mirror observes the owned desktop density without changing it for the viewer',async()=>{
 const before=process.env.CHARIOX_KERNEL_BROWSER_MIRROR;process.env.CHARIOX_KERNEL_BROWSER_MIRROR='1';
 try{for(const wire of [1,2])for(const device_scale_factor of [1,2]){
  const {host,service,calls}=fixture();host.browser.inputCapture={async hold(){return async()=>{}}};host.viewport=()=>({...hostViewport(),device_scale_factor:1});
  try{
   const sub=await service.subscribe({tab_id:'t',generation:1,wire,device_scale_factor},'terminal:room-viewer');
   assert.equal(sub.device_scale_factor,1,'the shared desktop renders at its admitted density');
   assert.equal(host.scales.get('t'),1);
   assert.equal(calls.filter(call=>call.method==='Emulation.setDeviceMetricsOverride').length,0,'an observer cannot resize the source');
  }finally{service.clear()}
 }}finally{if(before===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_MIRROR;else process.env.CHARIOX_KERNEL_BROWSER_MIRROR=before}
});
test('protocol 560/168: a subscription names the person display size and the newest one owns the layout',async()=>{
 const {service,host}=fixture(),laid=[];host.setLayout=async(tab,id,layout)=>{laid.push([tab.tab_id,layout]);};host.releaseLayout=(tabId,id)=>{laid.push(['release',tabId,id]);};
 const phone=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1,layout_width:400,layout_height:700},'a');
 assert.deepEqual(laid.at(-1),['t',{width:400,height:700}]);
 await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1,layout_width:2000,layout_height:900},'a');
 assert.deepEqual(laid.at(-1),['t',null],'a display at or above the canonical size restores the page');
 await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 assert.deepEqual(laid.at(-1),['t',null]);
 for(const bad of [{layout_width:400},{layout_width:100,layout_height:700},{layout_width:'400',layout_height:700},{layout_width:400,layout_height:99999}])
  await assert.rejects(service.subscribe({tab_id:'t',generation:1,device_scale_factor:1,...bad},'a'),/layout bounds/);
 service.drop(phone.subscription_id);
 assert.deepEqual(laid.at(-1),['release','t',phone.subscription_id]);
});


test('MP-08/MP-10/MP-11: App draft fence finishes before publication and rechecks protection',async()=>{
 const {service,host}=fixture(),entered=Promise.withResolvers(),commit=Promise.withResolvers();
 host.browser.appTabs={async flushDrafts(target){assert.equal(target,'target');entered.resolve();await commit.promise}};
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 let published=false;const packet=service.next(next(sub.subscription_id),'a').then(value=>{published=true;return value});
 await entered.promise;assert.equal(published,false);host.protection={revision:2,values:[],targets:[]};commit.resolve();
 await assert.rejects(packet,/stale mirror protection/);assert.equal(published,false);
});

test('MP-08/MP-10/MP-11: publishing an App draft also waits for saved browser selection',async()=>{
 const {service,host}=fixture(),committed=Promise.withResolvers();let saving=false,published=false;const lane=new SampleLane();host.sampleLane=()=>lane;
 host.browser.appTabs={async flushDrafts(){return true}};
 host.save=async()=>{saving=true;await committed.promise};
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 const packet=service.next(next(sub.subscription_id),'a').then(value=>{published=true;return value});
 await new Promise(resolve=>setTimeout(resolve,30));assert(saving,'the selection write must start before publishing the draft');assert.equal(published,false);
 committed.resolve();await packet;assert.equal(published,true);
});

test('MP-08/MP-10/MP-11: an App capture waits for a successful DOM input before saving its selection',async()=>{
 const {service,host}=fixture(),lane=new SampleLane(),input=Promise.withResolvers(),commit=Promise.withResolvers();let selected=false,saving=false,published=false;
 host.sampleLane=()=>lane;host.browser.appTabs={async flushDrafts(){return true}};
 const editing=lane.run('input',async()=>{await input.promise;selected=true});
 host.save=async()=>{saving=true;assert(selected,'save must observe the admitted DOM input selection');await commit.promise};
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 const packet=service.next(next(sub.subscription_id),'a').then(value=>{published=true;return value});packet.catch(()=>{});
 await new Promise(resolve=>setTimeout(resolve,30));assert.equal(saving,false);assert.equal(published,false);
 input.resolve();await editing;await new Promise(resolve=>setImmediate(resolve));assert(saving);assert.equal(published,false);
 commit.resolve();await packet;assert(published);
});
