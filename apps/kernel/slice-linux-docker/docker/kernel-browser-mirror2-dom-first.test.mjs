// MP-08/MP-10/MP-11: supplementary packet-order regressions, not live acceptance.
import test from 'node:test';
import assert from 'node:assert/strict';
import {setTimeout as delay} from 'node:timers/promises';
import {Mirror2} from './kernel-browser-mirror2.mjs';
const pageSize=()=>({width:1280,height:800});

function page(fontRequirements) {
 const policy={targets:[]},tab={tab_id:'t',target_id:'T',document_id:'D'};
 const connection={isOpen:()=>true},world={connection,sessionId:'S',frame:{loaderId:'D'},ref:'m'};
 const stream={fontRequirements,tab_id:'t',world,issued:0,document_id:'D',policy,fallback:null,header:{},resources:new Map(),attrSequence:new Map(),ownScrolls:new Map(),frames:new Map(),frameSlots:new Map(),frameSlot:0,tileKeys:'',hurry:0};
 const host={protection:policy,generation:1,pageSize,scales:new Map(),tabs:new Map([['t',tab]]),displayTarget:async()=>tab,browser:{connection,documentIdsByTarget:new Map([['T','D']]),sessionsByTarget:new Map([['T','S']])}};
 const mirror=new Mirror2({host,assertWebTab(){},require:()=>stream,streams:new Map([['s',stream]])});
 mirror.world=async()=>world;
 const state={ops:[],changed:[],resources:[],sheets:[],scroll:[0,0],focused:null,selection:null,fragment:null,opaque:false,pressed:0,snapshot_complete:true,loaded:1};
 const nodes=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'},{id:'n4',parent:'n3',kind:'text',text:'New Tab'}];
 mirror.evaluate=async(_world,expression)=>expression.endsWith('customHosts()')?false:expression.endsWith('snapshot()')?{...state,nodes,root:'n1',resources:[{key:'r1',url:'https://public.example/font.woff2',kind:'font'}],ops:[{op:'fonts',id:'n1',required:['a'.repeat(64)]}]}:state;
 mirror.frames={attach:async()=>({records:[],ops:[],opaque:[]}),drain:async()=>({ops:[],resources:[],changed:[]}),project:(_stream,source)=>source};
 mirror.sheetOps=async()=>[];mirror.tiles=async()=>[];mirror.encode=async(_stream,packet)=>packet;
 return {mirror,stream,host,state,command:{subscription_id:'s',generation:1,after_sequence:0}};
}

test('MP-08/MP-10/MP-11 opted-in first DOM does not wait for font bytes; the next quiet credit carries them',async()=>{
 const {mirror,stream,command}=page(true),calls=[],font={key:'r1',resource_id:'b'.repeat(64),mime_type:'font/woff2',data_base64:'AAAA'};
 const gate=Promise.withResolvers();mirror.materialize=async(_world,_stream,kind)=>{calls.push(kind);await gate.promise;return [font]};
 const pending=mirror.packet(stream,command,'scope');
 try {
  const first=await Promise.race([pending,delay(150).then(()=>null)]);
  assert(first,'loaded font bytes must not hold a usable first DOM packet');
  assert.equal(first.reset,true);assert(first.nodes.length>0);assert.equal(first.resources,undefined);
  assert.deepEqual(calls,[]);assert.deepEqual(first.ops,[{op:'fonts',id:'n1',required:['a'.repeat(64)]}],'the positive native-font requirement still fences handover');
  gate.resolve();
  const next=await mirror.packet(stream,{...command,after_sequence:1},'scope',undefined,Date.now());
  assert.equal(next.sequence,2);assert.deepEqual(next.resources,[font]);assert.deepEqual(calls,[null]);
 }finally{gate.resolve();await pending.catch(()=>{});}
});

for(const typing of [false,true])test(`MP-08/MP-10/MP-11 a continuing DOM stream does not starve the opted-in viewer of font bytes${typing?' during input':''}`,async()=>{
 const {mirror,stream,state,command}=page(true),calls=[],font={key:'r1',resource_id:'b'.repeat(64),mime_type:'font/woff2',data_base64:'AAAA'};
 mirror.materialize=async(_world,_stream,kind)=>{calls.push(kind);return [font]};
 const first=await mirror.packet(stream,command,'scope');
 assert.equal(first.reset,true);assert.equal(first.resources,undefined);
 // The rest of a huge document keeps every later credit non-empty: its fonts still travel with the DOM chunks.
 state.ops=[{op:'text',id:'n4',text:'more'}];state.changed=['n4'];
 if(typing)stream.inputAt=Date.now();
 const next=await mirror.packet(stream,{...command,after_sequence:1},'scope',undefined,Date.now());
 assert.equal(next.sequence,2);assert.deepEqual(next.ops.map(op=>op.op),['text']);assert.deepEqual(next.resources,[font]);
 // P9: without recent input every due resource rides the DOM packet; an input's echo carries only the fonts.
 assert.deepEqual(calls,[typing?'font':null]);
});

test('MP-08/MP-10/MP-11 a legacy viewer still receives first-packet fonts',async()=>{
 const {mirror,stream,command}=page(false),font={key:'r1',mime_type:'font/woff2',data_base64:'AAAA'};
 mirror.materialize=async(_world,_stream,kind)=>{assert.equal(kind,'font');return [font]};
 const first=await mirror.packet(stream,command,'scope');assert.deepEqual(first.resources,[font]);
});

for(const slot of [0,1])test(`MP-08/MP-10/MP-11 pending fonts precede a near image in resource slot ${slot}`,async()=>{
 const mirror=new Mirror2({host:{}}),font={key:'r2',kind:'font',state:'ok',resource:{key:'r2',mime_type:'font/woff2',data_base64:'BBBBBBBB'}},image={key:'r1',kind:'image',state:'ok',resource:{key:'r1',mime_type:'image/png',data_base64:'AAAAAAAA'}};
 const world={ref:'m'},stream={issued:1,movedAt:0,loadedCount:2,resources:new Map([['r1',image],['r2',font]])};
 mirror.evaluate=async(_world,expression)=>expression.includes('nearImages')?{near:[['r1',10,10]],all:['r1']}:[];
 const out=await mirror.materializeWorld(world,[image,font],slot,8,stream);
 assert.deepEqual(out.map(resource=>resource.key),['r2'],'font readiness cannot queue behind image bytes');
});

test('MP-08/MP-10/MP-11 a child document font precedes the main document image budget',async()=>{
 const mirror=new Mirror2({host:{}}),image={key:'r1',kind:'image',state:'ok',resource:{key:'r1',mime_type:'image/png',data_base64:'AAAAAAAA'}},font={key:'r1000002',slot:1,kind:'font',state:'ok',resource:{key:'r1000002',mime_type:'font/woff2',data_base64:'BBBBBBBB'}};
 const main={ref:'main'},child={ref:'child'},stream={issued:1,movedAt:0,resources:new Map([['r1',image],['r1000002',font]]),frameSlots:new Map([[1,{child}]])};
 mirror.packetBytes=()=>8;mirror.evaluate=async(_world,expression)=>expression.includes('nearImages')?{near:[['r1',10,10]],all:['r1']}:expression.includes('loadedCount')?1:[];
 assert.deepEqual((await mirror.materialize(main,stream,null,1)).map(resource=>resource.key),['r1000002']);
});

test('MP-08/MP-10/MP-11 a completed font response needs no busy-renderer image or load query',async()=>{
 const body=Buffer.alloc(48);body.write('wOF2');
 const mirror=new Mirror2({host:{browser:{responseBodies:{read:async()=>body}}}}),entry={key:'r1',kind:'font',url:'https://public.example/font.woff2',state:'new'};
 mirror.evaluate=async()=>{throw Error('the busy renderer must not hold completed font bytes')};
 const out=await mirror.materializeWorld({ref:'m'},[entry],0,256*1024,{issued:1,movedAt:0,resources:new Map([['r1',entry]])});
 assert.equal(out[0].mime_type,'font/woff2');
});

for(const retirement of ['browser','connection','session'])test(`MP-08/MP-10/MP-11 deferred font bytes refuse ${retirement} retirement with the same loader`,async()=>{
 const {mirror,stream,host,command}=page(true);
 await mirror.packet(stream,command,'scope');
 const started=Promise.withResolvers(),release=Promise.withResolvers();
 mirror.materialize=async()=>{started.resolve();await release.promise;return [{key:'r1',mime_type:'font/woff2',data_base64:'AAAA'}]};
 const pending=mirror.packet(stream,{...command,after_sequence:1},'scope',undefined,Date.now());
 await started.promise;
 if(retirement==='browser')host.browser={...host.browser};
 if(retirement==='connection')host.browser.connection={};
 if(retirement==='session')host.browser.sessionsByTarget.set('T','replacement');
 release.resolve();
 await assert.rejects(pending,error=>error.code==='stale_document_reference');
 assert.equal(stream.issued,1,'a retired owner cannot publish the deferred resource packet');
});

test('MP-08/MP-10 a font-only credit does not consume the image retry that a new load count earned',async()=>{
 const mirror=new Mirror2({host:{}}),image={key:'r1',kind:'image',state:'waiting',tries:0},font={key:'r2',kind:'font',state:'new',tries:0};
 const stream={issued:1,movedAt:0,frameSlots:new Map(),resources:new Map([['r1',image]])},due=[];
 mirror.packetBytes=()=>64;mirror.evaluate=async()=>0;
 // The real materializeWorld stamps each entry it attempts with the load count (the budget-filled case has its own test).
 mirror.materializeWorld=async(_world,entries,_slot,_budget,owner)=>{due.push(entries.map(entry=>entry.key));for(const entry of entries)entry.triedAt=owner.loadedCount;return []};
 await mirror.materialize({ref:'m'},stream,null,4);
 stream.resources.set('r2',font);
 await mirror.materialize({ref:'m'},stream,'font',5);
 due.length=0;
 await mirror.materialize({ref:'m'},stream,null,5);
 assert.deepEqual(due.flat().filter(key=>key==='r1'),['r1'],'the waiting image is retried once its load count changed');
 due.length=0;
 await mirror.materialize({ref:'m'},stream,null,5);
 assert.deepEqual(due.flat().filter(key=>key==='r1'),[],'and only once per count');
});
