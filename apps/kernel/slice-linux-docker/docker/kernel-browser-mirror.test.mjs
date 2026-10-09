// MP-08/MP-10/MP-11: service epochs, privacy, bounded streams and resource admission.
import test from 'node:test';
import assert from 'node:assert/strict';
import { MirrorService } from './kernel-browser-mirror.mjs';
import { materializeMirrorResources,mirrorHash } from './kernel-browser-mirror-resources.mjs';
import { encodePng } from './kernel-browser-pixels.mjs';
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
    if(method==='Runtime.evaluate')return {result:{value:params.expression.includes('.read(')?structuredClone(state.snapshot):params.expression.includes('Object.fromEntries([...style]')?{}:true}};
    throw Error(`unexpected CDP method ${method}`);
  }};
  const host={generation:1,scales:new Map(),protection:{values:[],targets:[],unknown:false},async target(){return {...tab,document_id:state.document};},async displayTarget(){return {...tab,document_id:state.document};},browser:{async resolvePageTarget(){return {connection,sessionId:'session'};},async ensureFocusWorld(){return {contextId:1};}},async screenshot(){return {data_base64:encodePng(1280,800,Buffer.alloc(1280*800*4,100)),protected_regions:[]};}};
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
test('MP-08/MP-11: only structured mirror input admits observed frame descendants',async()=>{
 for(const fallback of [false,true]) {
  const {service}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
  if(fallback){const evaluate=service.evaluate.bind(service);service.evaluate=async(world,expression)=>{if(expression.includes('.read('))throw Error('synthetic observer unavailable');return evaluate(world,expression);};}
  const packet=await service.next(next(s.subscription_id),'a');
  const resolved=await service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:packet.sequence,action:{kind:'coordinate',input:{kind:'text',text:'fixture'}}},'a');
  assert.equal(resolved.observedFrameInput===true,!fallback);
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
 const f=fixture();await f.service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');await assert.rejects(f.service.subscribe({tab_id:'t',generation:1,device_scale_factor:2},'a'),/canonical/);
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

// MP-08/MP-10/MP-11: both transports must size the actual compositor view.
for(const dpr of [1,2])test(`MP-10 mirror negotiates native view image scale at DPR${dpr}`,async()=>{
 const {service,calls}=fixture();await service.subscribe({tab_id:'t',generation:1,device_scale_factor:dpr},'a');
 assert.deepEqual(calls.find(c=>c.method==='Emulation.setDeviceMetricsOverride').params,{width:1280,height:800,deviceScaleFactor:dpr,scale:dpr,mobile:false});
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
