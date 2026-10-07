// MP-08/MP-10/MP-11: service epochs, privacy, bounded streams and resource admission.
import test from 'node:test';
import {randomBytes} from 'node:crypto';
import {gunzipSync} from 'node:zlib';
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
    if(method==='Browser.getWindowForTarget')return {windowId:1};
    if(method==='Browser.setWindowBounds')return {};
    if(method==='Runtime.releaseObjectGroup')return {};
    if(method==='Runtime.evaluate'&&params.expression.includes('.customHosts()'))return {result:{value:null}};
    if(method==='Runtime.evaluate'&&params.expression.includes('.fontKeys()'))return {result:{value:[]}};
    if(method==='Runtime.evaluate')return {result:{value:params.expression.includes('.read(')?structuredClone(state.snapshot):params.expression.includes('Object.fromEntries([...style]')?{}:params.expression.includes('.epoch()')?(state.snapshot.revision??0):true}};
    throw Error(`unexpected CDP method ${method}`);
  }};
  const host={generation:1,scales:new Map(),protection:{values:[],targets:[],unknown:false},async target(){return {...tab,document_id:state.document};},async displayTarget(){return {...tab,document_id:state.document};},browser:{async resolvePageTarget(){return {connection,sessionId:'session'};},async ensureFocusWorld(){return {contextId:1};}},async screenshot(){return {data_base64:encodePng(1280,800,Buffer.alloc(1280*800*4,100)),protected_regions:[]};}};
  return {host,state,service:new MirrorService(host)};
}
const next=(subscription_id,after_sequence=0,drift_nodes=[])=>({subscription_id,generation:1,after_sequence,drift_nodes});
test('MP-11: an additional mirror observer does not reset canonical headed geometry',async()=>{
 const {service,host}=fixture();host.chromium={display:{}};
 const {connection}=await host.browser.resolvePageTarget('target'),send=connection.send;let changes=0;
 connection.send=async(method,...args)=>{if(['Browser.setWindowBounds','Emulation.setDeviceMetricsOverride'].includes(method))changes++;return send.call(connection,method,...args)};
 await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 assert.equal(changes,2);
 await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'b');
 assert.equal(changes,2,'Read-only subscription must not resize the page or invalidate another observer');
});
test('MP-11: headed tile capture keeps the native viewport unchanged',async()=>{
 const {service,state,host}=fixture();host.chromium={display:{}};
 const control=state.snapshot.nodes[0];Object.assign(control,{parent:'n0',kind:'tile',tag:'button',reason:'native_control'});
 state.snapshot.root='n0';state.snapshot.nodes.unshift({id:'n0',parent:null,children:[control.id],kind:'element',tag:'html'});
 const screenshot=host.screenshot;host.screenshot=async(tab,clip)=>{assert.equal(clip,null,'Headed CDP crops resize the viewport during observation');return screenshot(tab,clip)};
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 const packet=await service.next(next(sub.subscription_id),'a');assert.equal(packet.tiles.length,1);
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

test('opaque SVG tile does not apply its already captured opacity a second time',async()=>{
 const {service,state}=fixture();state.snapshot.nodes[0].children.push('n3');state.snapshot.nodes.push({id:'n3',parent:'n1',children:[],kind:'tile',tag:'img',reason:'opaque_media',box:{x:0,y:0,width:20,height:20},style:{opacity:'0.6',width:'20px',height:'20px'}});
 const stream=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');const packet=await service.next(next(stream.subscription_id),'a');
 assert(packet.nodes.some(n=>n.id==='n2'&&n.kind==='text'),'Ordinary text must stay mirrored beside the SVG logo');
 assert.equal(packet.nodes.find(n=>n.id==='n3').style.opacity,'0.6','Native layout retains source opacity; compositor pixels paint separately');
});

test('composited source geometry uses protected region tiles without replacing ordinary DOM',async()=>{
 for(const mode of ['ancestorOpacity','tileTransform']){
  const {service,state}=fixture();state.snapshot.nodes[0].children.push('n3');state.snapshot.nodes[0].style=mode==='ancestorOpacity'?{opacity:'0.6'}:{};state.snapshot.nodes.push({id:'n3',parent:'n1',children:[],kind:'tile',tag:'img',reason:'opaque_media',box:{x:0,y:0,width:20,height:20},style:mode==='tileTransform'?{transform:'rotate(10deg)'}:{}});
  const stream=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');const packet=await service.next(next(stream.subscription_id),'a');assert(!packet.nodes.some(n=>n.reason==='observer_bounds_or_unavailable'));assert(packet.tiles.some(t=>t.node_id==='n3'));
 }
});


test('zero-opacity ancestors do not turn invisible native controls into full-page fallback',async()=>{
 for(const dpr of [1,2]){
  const {service,state,host}=fixture();
  state.snapshot.nodes[0].children.push('hidden-parent');
  state.snapshot.nodes.push({id:'hidden-parent',parent:'n1',children:['hidden-input'],kind:'element',tag:'div',style:{opacity:'0',transform:'translateX(5px)'},box:{x:10,y:10,width:80,height:30}},
   {id:'hidden-input',parent:'hidden-parent',children:[],kind:'tile',tag:'input',reason:'native_control',style:{opacity:'1'},box:{x:10,y:10,width:80,height:30}});
  host.screenshot=async()=>{throw Error('Invisible native controls must not require source pixels')};
  const s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:dpr},'a');
  const packet=await service.next(next(s.subscription_id),'a');
  assert(packet.nodes.some(n=>n.id==='n2'&&n.kind==='text'),'Ordinary article text remains mirrored');
  assert.equal(packet.nodes.find(n=>n.id==='hidden-parent').style.opacity,'0');
  assert.deepEqual(packet.tiles,[]);
  state.snapshot.nodes.find(n=>n.id==='hidden-parent').style.opacity='0.6';
  host.screenshot=async()=>({data_base64:encodePng(1280*dpr,800*dpr,Buffer.alloc(1280*800*dpr*dpr*4,100)),protected_regions:[]});
  const shown=await service.next(next(s.subscription_id,packet.sequence),'a');
  assert(!shown.nodes.some(n=>n.reason==='observer_bounds_or_unavailable'));assert(shown.tiles.some(t=>t.node_id==='hidden-input'),'Visible composition is captured and protected as an individual region');
 }
});

test('a zero-opacity tile is never normalized into visible source pixels',async()=>{
 const {service,state,host}=fixture();state.snapshot.nodes[0].children.push('hidden');
 state.snapshot.nodes.push({id:'hidden',parent:'n1',children:[],kind:'tile',tag:'img',reason:'opaque_media',style:{opacity:'0'},box:{x:10,y:10,width:80,height:30}});
 host.screenshot=async()=>{throw Error('Invisible tile must not be captured')};
 const s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:2},'a');
 const packet=await service.next(next(s.subscription_id),'a');
 assert.equal(packet.nodes.find(n=>n.id==='hidden').style.opacity,'0');assert.deepEqual(packet.tiles,[]);
});

test('MD-454: large DOM credits stream bounded chunks instead of rejecting the page',async()=>{
 const {service,state,host}=fixture();state.snapshot.nodes[0].children=[];
 for(let n=3;n<5003;n++){const id='n'+n;state.snapshot.nodes[0].children.push(id);state.snapshot.nodes.push({id,parent:'n1',children:[],kind:'element',tag:'p',style:{color:'rgb(1, 2, 3)',width:'1200px',margin:'0px',font:'16px sans-serif',padding:'1px'},attributes:{title:randomBytes(600).toString('base64')}})}
 state.snapshot.nodes=state.snapshot.nodes.filter(n=>n.id!=='n2');
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');
 let chunk=await service.next(next(sub.subscription_id),'a'),index=0;const pieces=[];
 assert.equal(chunk.kind,'mirror_chunk');assert(chunk.parts>1);
 const repeated=await service.next(next(sub.subscription_id),'a');assert.deepEqual(repeated,chunk);
 await assert.rejects(service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:sub.subscription_id,sequence:chunk.sequence,action:{kind:'key',key:'Tab'}},'a'),/stale mirror input epoch/);
 while(true){assert.equal(chunk.index,index);assert(JSON.stringify(chunk).length<400000);pieces.push(Buffer.from(chunk.payload_base64,'base64'));if(index+1===chunk.parts)break;chunk=await service.next({...next(sub.subscription_id),after_chunk:index++},'a')}
 const wire=JSON.parse(gunzipSync(Buffer.concat(pieces)).toString('utf8'));
 assert.equal(wire.nodes.length,5001);assert(wire.styles.length<10);
 const retried=await service.next({...next(sub.subscription_id),after_chunk:chunk.index-1},'a');assert.deepEqual(retried,chunk);
 const committed=await service.next(next(sub.subscription_id,chunk.sequence),'a');assert.equal(committed.base_sequence,chunk.sequence);
 host.protection={...host.protection};service.invalidate();
 await assert.rejects(service.next({...next(sub.subscription_id),after_chunk:0},'a'),/chunk/);
});

test('MD-454: repeated source descriptors share one admitted resource without a page-count cap',async()=>{
 const data=encodePng(1,1,Buffer.from([1,2,3,255]));let reads=0;
 const connection={async send(method){if(method==='Page.getResourceTree')return {frameTree:{frame:{id:'f'},resources:[{url:'https://example.test/image.png'}]}};if(method==='Page.getResourceContent'){reads++;return {base64Encoded:true,content:data}};throw Error(method)}};
 const result=await materializeMirrorResources(connection,'s',Array.from({length:500},(_,i)=>({key:'r'+i,url:'https://example.test/image.png',kind:'image'})),[]);
 assert.equal(result.mapped.size,500);assert.equal(result.resources.size,1);assert.equal(reads,1);
});

test('MD-454: transformed native controls remain region tiles instead of replacing the DOM',async()=>{
 const {service,state}=fixture();state.snapshot.nodes.push({id:'n3',parent:'n1',children:[],kind:'tile',tag:'button',reason:'native_control',style:{transform:'matrix(0, -1, 1, 0, 0, 0)',width:'30px',height:'20px'},box:{x:10,y:10,width:20,height:30}});state.snapshot.nodes[0].children.push('n3');
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');const packet=await service.next(next(sub.subscription_id),'a');
 assert(!packet.nodes.some(n=>n.reason==='observer_bounds_or_unavailable'));assert(packet.tiles.some(t=>t.node_id==='n3'));
});


test('MD-454: exact-settled native regions reuse verified pixels until source or protection changes',async()=>{
 const {service,state,host}=fixture();state.snapshot.revision=0;
 state.snapshot.nodes.push({id:'n3',parent:'n1',children:[],kind:'tile',tag:'button',reason:'native_control',style:{width:'30px',height:'20px'},box:{x:10,y:10,width:30,height:20}});state.snapshot.nodes[0].children.push('n3');
 let captures=0;const capture=host.screenshot;host.screenshot=async(...args)=>{captures++;return capture(...args)};
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');let seq=0;
 for(let n=0;n<3;n++)seq=(await service.next(next(sub.subscription_id,seq),'a')).sequence;
 assert.equal(captures,2,'One motion crop, one exact full-frame refinement, then cache');
 state.snapshot.revision++;seq=(await service.next(next(sub.subscription_id,seq),'a')).sequence;assert.equal(captures,3,'A source mutation forces fresh protected pixels');
 host.protection={...host.protection};service.invalidate();await service.next(next(sub.subscription_id,seq),'a');assert.equal(captures,4,'Policy invalidation cannot reuse the old protected cache');
});

test('MD-454: source protection/document changes during chunk delivery discard the unissued epoch',async()=>{
 const {service,state}=fixture();state.snapshot.revision=0;state.snapshot.nodes[0].children=[];
 for(let n=3;n<1203;n++){const id='n'+n;state.snapshot.nodes[0].children.push(id);state.snapshot.nodes.push({id,parent:'n1',children:[],kind:'element',tag:'p',attributes:{title:randomBytes(600).toString('base64')}})}state.snapshot.nodes=state.snapshot.nodes.filter(n=>n.id!=='n2');
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');const first=await service.next(next(sub.subscription_id),'a');assert(first.parts>1);state.snapshot.revision++;
 await assert.rejects(service.next({...next(sub.subscription_id),after_chunk:0},'a'),/mirror frame changed before commit/);
 await assert.rejects(service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:sub.subscription_id,sequence:first.sequence,action:{kind:'key',key:'Tab'}},'a'),/stale mirror (input epoch|protection policy)/);
});


test('MD-454: content cache reuses hashes while revalidating stable URLs and evicting unused bodies',async()=>{
 const cache=new Map();let body=encodePng(1,1,Buffer.from([1,2,3,255])),reads=0;
 const connection={async send(method){if(method==='Page.getResourceTree')return {frameTree:{frame:{id:'f'},resources:[{url:'https://fixture/resource'}]}};reads++;return {base64Encoded:true,content:body}}};const descriptors=[{key:'r0',url:'https://fixture/resource',kind:'image'}];
 const first=await materializeMirrorResources(connection,'s',descriptors,[],cache),second=await materializeMirrorResources(connection,'s',descriptors,[],cache);assert.equal([...first.resources.values()][0],[...second.resources.values()][0]);assert.equal(reads,2,'URL bodies revalidate on every read');
 body=encodePng(1,1,Buffer.from([4,5,6,255]));const third=await materializeMirrorResources(connection,'s',descriptors,[],cache);assert.notEqual(third.mapped.get('r0'),first.mapped.get('r0'));assert.equal(cache.size,1);assert(!cache.has(first.mapped.get('r0')));
});

test('MD-454: decoded resource memory budget degrades only excess regions, never the entire DOM',async()=>{
 const bodies=Array.from({length:5},(_,i)=>encodePng(2048,2048,Buffer.alloc(2048*2048*4,i+100)));
 const connection={async send(method,params){if(method==='Page.getResourceTree')return {frameTree:{frame:{id:'f'},resources:bodies.map((_,i)=>({url:'https://fixture/'+i}))}};return {base64Encoded:true,content:bodies[Number(params.url.slice(-1))]}}};
 const result=await materializeMirrorResources(connection,'s',bodies.map((_,i)=>({key:'r'+i,url:'https://fixture/'+i,kind:'image'})),[]);assert.equal(result.resources.size,4);assert.equal(result.mapped.get('r4'),null);assert.equal(result.mapped.size,5);
});


test('MD-454: unused/unloaded font faces do not hide a family with an admitted loaded face',async()=>{
 const {service,state,host}=fixture();state.snapshot.nodes[0].style={'font-family':'ExampleFont', 'font-weight':'400'};state.snapshot.fonts=[{family:'ExampleFont',weight:'400',style:'normal',resource:'r0'},{family:'ExampleFont',weight:'700',style:'normal',resource:'r1'}];state.snapshot.resources=[{key:'r0',url:'https://fixture/loaded.woff2',kind:'font'},{key:'r1',url:'https://fixture/unused.woff2',kind:'font'}];
 const font=Buffer.alloc(48);font.write('wOF2');font.writeUInt32BE(48,8);font.writeUInt16BE(1,12);font.writeUInt32BE(1000,16);
 const resolve=host.browser.resolvePageTarget;host.browser.resolvePageTarget=async(...args)=>{const world=await resolve(...args),send=world.connection.send;world.connection.send=async(method,params)=>method==='Page.getResourceTree'?{frameTree:{frame:{id:'f'},resources:[{url:'https://fixture/loaded.woff2'}]}}:method==='Page.getResourceContent'?{base64Encoded:true,content:font.toString('base64')}:send(method,params);return world};
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a');const packet=await service.next(next(sub.subscription_id),'a');assert(!packet.nodes.some(n=>n.reason==='observer_bounds_or_unavailable'));assert(packet.nodes.some(n=>n.id==='n2'&&n.kind==='text'));assert.equal(packet.fonts.length,1);
});


test('MD-454: browser-loaded font response is used when Page cache drops its body',async()=>{
 const font=Buffer.alloc(48);font.write('wOF2');font.writeUInt32BE(48,8);font.writeUInt16BE(1,12);font.writeUInt32BE(1000,16);let reads=0;
 const connection={async send(method){if(method==='Page.getResourceTree')return {frameTree:{frame:{id:'f'},resources:[{url:'https://fixture/font.woff2'}]}};throw Error('Page resource body evicted')}};
 const result=await materializeMirrorResources(connection,'s',[{key:'r0',url:'https://fixture/font.woff2',kind:'font'}],[],new Map(),{loadedFontBody:async()=>{reads++;return {base64Encoded:true,content:font.toString('base64')}}});assert.equal(result.resources.size,1);assert.equal(reads,1);
});

test('MD-454: native custom-host changes during chunk credits revoke the unissued frame',async()=>{
 const {service,state}=fixture();state.snapshot.nodes[0].children=[];
 for(let n=3;n<1203;n++){const id='n'+n;state.snapshot.nodes[0].children.push(id);state.snapshot.nodes.push({id,parent:'n1',children:[],kind:'element',tag:'p',attributes:{title:randomBytes(600).toString('base64')}})}state.snapshot.nodes=state.snapshot.nodes.filter(n=>n.id!=='n2');
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),first=await service.next(next(sub.subscription_id),'a');assert(first.parts>1);
 const stream=service.streams.get(sub.subscription_id);let released=false;stream.frameCustom={verify:async()=>{throw Error('MP-11: custom host changed during mirror frame')},release:async()=>{released=true}};
 await assert.rejects(service.next({...next(sub.subscription_id),after_chunk:0},'a'),/custom host changed/);
 assert(released);assert.equal(stream.pending,null);assert.equal(stream.previous,null);assert.deepEqual(stream.epochs,[]);
});

test('MD-454: protection updates during a chunk native verification fence the reply',async()=>{
 const {service,state,host}=fixture();state.snapshot.nodes[0].children=[];
 for(let n=3;n<1203;n++){const id='n'+n;state.snapshot.nodes[0].children.push(id);state.snapshot.nodes.push({id,parent:'n1',children:[],kind:'element',tag:'p',attributes:{title:randomBytes(600).toString('base64')}})}state.snapshot.nodes=state.snapshot.nodes.filter(n=>n.id!=='n2');
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),first=await service.next(next(sub.subscription_id),'a');assert(first.parts>1);
 const stream=service.streams.get(sub.subscription_id);stream.frameCustom={verify:async()=>{host.protection={...host.protection};service.invalidate()},release:async()=>{}};
 await assert.rejects(service.next({...next(sub.subscription_id),after_chunk:0},'a'),/chunk policy or document changed/);
 assert.equal(stream.pending,null);assert.equal(stream.previous,null);assert.deepEqual(stream.epochs,[]);
});

// A credit may move viewport geometry while a wheel waits in the client queue.
// This is a pre-dispatch refusal only; pointer clicks must never be replayed.
test('MP-11: changed wheel geometry refreshes once before effects; clicks still refuse',async()=>{
 const {service,state}=fixture(),s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),first=await service.next(next(s.subscription_id),'a');
 state.snapshot.nodes[0].box.width=81;await service.next(next(s.subscription_id,first.sequence),'a');service.world=async()=>assert.fail('no native work before admission');
 const input=kind=>service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:first.sequence,action:{kind:'coordinate',input:{kind,x:10,y:10,delta_x:0,delta_y:20}}},'a');
 await assert.rejects(input('scroll'),/MP-11: stale mirror input epoch/);
 await assert.rejects(input('click'),error=>!error.message.includes('stale mirror input epoch'));
});

test('MP-11: native paint overlays preserve public inline flow but never protected descendants',async()=>{
 const {service,state}=fixture();state.snapshot.nodes[0].kind='tile';state.snapshot.nodes[0].tag='label';state.snapshot.nodes[0].reason='unsupported_paint';state.snapshot.nodes[0].children.push('n3');
 state.snapshot.nodes.push({id:'n3',parent:'n1',children:[],kind:'mask',tag:'div',box:{x:40,y:0,width:20,height:20}});
 // Keep the overlay host below the document root; whole-page opaque roots refuse.
 state.snapshot.nodes[0].parent='n0';state.snapshot.nodes.unshift({id:'n0',parent:null,children:['n1'],kind:'element',tag:'html'});state.snapshot.root='n0';
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),packet=await service.next(next(sub.subscription_id),'a');
 assert.equal(packet.nodes.find(n=>n.id==='n1').tag,'label');assert.deepEqual(packet.nodes.find(n=>n.id==='n1').children,['n2','n3']);assert.equal(packet.nodes.find(n=>n.id==='n2').text,'fixture');assert.equal(packet.nodes.find(n=>n.id==='n3').kind,'mask');
 assert.equal(packet.tiles.length,1);
});
test('MP-11: trusted live wheel-geometry refusal never dispatches and cannot replay a click',async()=>{
 const {service,state,host}=fixture();state.snapshot.nodes[1]={id:'n2',parent:'n1',children:[],kind:'element',tag:'p',box:{x:5,y:5,width:20,height:20}};
 const s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),p=await service.next(next(s.subscription_id),'a');
 const evaluate=service.evaluate.bind(service);service.evaluate=(w,e)=>e.includes('.coordinateTarget(')?Promise.resolve({scroll_epoch_refused:true}):evaluate(w,e);
 const input=kind=>service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:p.sequence,action:{kind:'coordinate',input:{kind,x:10,y:10,delta_x:0,delta_y:20}}},'a');
 const wheel=await input('scroll');await assert.rejects(wheel.guard(),/MP-11: stale mirror input epoch/);
 const click=await input('click');await assert.rejects(click.guard(),error=>!error.message.includes('stale mirror input epoch'));
});

test('MD-454: a source-only capture race resets unissued bytes and refuses input without native effects',async()=>{
 const {service,state,host}=fixture();state.snapshot.revision=1;
 const s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),p=await service.next(next(s.subscription_id),'a');
 const evaluate=service.evaluate.bind(service);let race=true;
 service.evaluate=async(w,e)=>{if(race&&e.endsWith('.epoch()')){race=false;state.snapshot.revision++}return evaluate(w,e)};
 await assert.rejects(service.next(next(s.subscription_id,p.sequence),'a'),/mirror frame changed before commit/);
 const stream=service.streams.get(s.subscription_id);assert.equal(stream.previous,null);assert.equal(stream.geometryResetPolicy,host.protection);
 service.world=async()=>assert.fail('refuse before native dispatch');
 await assert.rejects(service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:p.sequence,action:{kind:'key',key:'Tab'}},'a'),/stale mirror input epoch/);
 host.protection={...host.protection};
 await assert.rejects(service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:p.sequence,action:{kind:'key',key:'Tab'}},'a'),error=>!error.message.includes('stale mirror input epoch'));
});

test('MP-11: a pending text operation cannot acquire the no-effect wheel marker after focus',async()=>{
 const {service,state}=fixture();state.snapshot.nodes[0].tag='input';state.snapshot.nodes[0].form={value:'',selection_start:0,selection_end:0,checked:false,disabled:false};
 const s=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a'),p=await service.next(next(s.subscription_id),'a');
 const stream=service.streams.get(s.subscription_id),evaluate=service.evaluate.bind(service);
 service.evaluate=async(w,e)=>{if(e.includes('.locate('))return {x:10,y:10};if(e.includes('.focus(')){stream.geometryResetPolicy=stream.policy;stream.policy=null;stream.epochs=[];return true}return evaluate(w,e)};
 const text=await service.resolveInput({tab_id:'t',document_id:'d'},{subscription_id:s.subscription_id,sequence:p.sequence,action:{kind:'text',node_id:'n1',text:'x'}},'a');let marked=false;
 await assert.rejects(text.perform(async()=>assert.fail('no text dispatch'),()=>marked=true),error=>!error.message.includes('stale mirror input epoch'));assert(marked);
});

for(const change of ['ordinary source','policy','document','incomplete'])test('MD-454: fully issued frame acknowledgement retires its old observation: '+change,async()=>{
 const {service,state,host}=fixture();state.snapshot.revision=0
 const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:1},'a')
 const packet=await service.next(next(sub.subscription_id),'a'),stream=service.streams.get(sub.subscription_id)
 stream.pending={chunks:[{},{}],sequence:packet.sequence,base_sequence:0,document_id:'d',policy:host.protection,world:stream.frameWorld,sourceRevision:0,lastIssued:change!=='incomplete'}
 state.snapshot.revision=change==='incomplete'?0:1;state.snapshot.nodes[1].text='Updated ordinary text'
 if(change==='policy')host.protection={...host.protection}
 if(change==='document')state.document='new-document'
 if(change==='ordinary source'){
  const fresh=await service.next(next(sub.subscription_id,packet.sequence),'a')
  assert.equal(fresh.base_sequence,packet.sequence);assert(fresh.nodes.some(n=>n.text==='Updated ordinary text'))
 }else await assert.rejects(service.next(next(sub.subscription_id,packet.sequence),'a'),change==='incomplete'?/incomplete mirror frame acknowledgement/:/policy or document changed/)
})
