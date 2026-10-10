// MP-08/MP-10/MP-11: fail closed before DOM construction on hostile/split streams.
import test from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { validateMirrorNode,validateMirrorPacket,mirrorCanonicalJson,mirrorTreeCanonicalJson,mirrorSandboxCsp } from './browser-mirror-security.js'
import { browserMirrorMinimumProtocolVersion,attachBrowserMirror,BrowserMirrorRenderer } from './browser-mirror.js'
import type { MirrorPacket } from './browser-mirror-types.js'
const packet=():MirrorPacket=>({subscription_id:'s',tab_id:'t',generation:1,document_id:'d',sequence:1,base_sequence:null,reset:true,hash:'a'.repeat(64),root:'n1',nodes:[{id:'n1',parent:null,kind:'element',tag:'div',children:[]}],removed:[],resources:[],tiles:[],fonts:[],scroll:{x:0,y:0},focused:null,selection:null,css_width:1280,css_height:800,device_scale_factor:1})
test('MP-11: scripts, active markup, event handlers, URLs and CSS exfiltration are refused',()=>{
 for(const tag of ['script','svg','object','embed','base','meta','link'])assert.throws(()=>validateMirrorNode({id:'n1',parent:null,children:[],kind:'element',tag}))
 for(const attr of ['onerror','onclick','srcdoc','href','src','action','formaction','name','data-private'])assert.throws(()=>validateMirrorNode({id:'n1',parent:null,children:[],kind:'element',tag:'div',attributes:{[attr]:'https://origin.test/private'}}))
 for(const css of ['url(https://leak.test)','image-set("https://leak.test")','u\\72l(https://leak.test)','expression(alert(1))','</style><script>1</script>'])assert.throws(()=>validateMirrorNode({id:'n1',parent:null,children:[],kind:'element',tag:'div',style:{background:css}}))
 validateMirrorNode({id:'n1',parent:null,children:[],kind:'element',tag:'div',style:{'-webkit-font-smoothing':'antialiased'}})
 assert.match(mirrorSandboxCsp,/script-src 'none'/);assert.match(mirrorSandboxCsp,/connect-src 'none'/);assert.match(mirrorSandboxCsp,/form-action 'none'/)
})
test('MP-11: opaque mask cannot carry text, values, resources or descendants',()=>{
 for(const extra of [{text:'sensitive'},{form:{value:'sensitive',checked:false,selected_index:0,selection_start:0,selection_end:1}},{resource:'b'.repeat(64)},{children:['n2']}])assert.throws(()=>validateMirrorNode({id:'n1',parent:null,children:[],kind:'mask',...extra}))
})
test('MP-11: cyclic, detached and multiply parented patches cannot change render state',()=>{
 const p=packet();const previous=validateMirrorPacket(p,new Map());
 for(const nodes of [[{...p.nodes[0]!,children:['n1']}],[{...p.nodes[0]!,children:['n2']},{id:'n2',parent:'other',kind:'text' as const,children:[],text:'x'}],[...p.nodes,{id:'n2',parent:null,kind:'text' as const,children:[],text:'x'}]])assert.throws(()=>validateMirrorPacket({...p,nodes},previous))
 assert.equal(previous.size,1)
})
test('MP-11: resources admit only bounded nonexecutable media',()=>{
 for(const mime_type of ['image/svg+xml','text/html','text/css','application/javascript'])assert.throws(()=>validateMirrorPacket({...packet(),resources:[{resource_id:'b'.repeat(64),mime_type,data_base64:'AA=='}]},new Map()))
})
test('MP-08: semantic hash survives Rust map ordering and removed-node patches',()=>{
 const a={root:'n1',nodes:[{id:'n1',children:[],kind:'element'}]},b={nodes:[{kind:'element',children:[],id:'n1'}],root:'n1'}
 assert.equal(createHash('sha256').update(mirrorCanonicalJson(a)).digest('hex'),createHash('sha256').update(mirrorCanonicalJson(b)).digest('hex'))
 const p=packet(),base=validateMirrorPacket(p,new Map());const next=validateMirrorPacket({...p,reset:false,nodes:[{...p.nodes[0]!,text:'changed'}]},base)
 assert.equal(next.get('n1')?.text,'changed');assert.equal(base.get('n1')?.text,undefined)
})
test('MP-08: protocol 443/unknown rejects before allocating a subscription or frame',async()=>{
 assert.equal(browserMirrorMinimumProtocolVersion,443)
 for(const protocolVersion of [435,442,0,NaN])await assert.rejects(attachBrowserMirror({protocolVersion,request:async()=>assert.fail('must not request')},{} as HTMLElement,{tab_id:'t',generation:1,device_scale_factor:1},()=>{}),/protocol 443/)
})

test('MP-11: admitted nodes do not retain mutable transport-owned references',()=>{
 const p=packet();p.nodes[0]!.style={color:'rgb(1, 2, 3)'};const accepted=validateMirrorPacket(p,new Map());
 p.nodes[0]!.style!.color='url(https://leak.test)';p.nodes[0]!.children.push('n2');
 assert.equal(accepted.get('n1')!.style!.color,'rgb(1, 2, 3)');assert.deepEqual(accepted.get('n1')!.children,[]);
})

test('MP-08/MP-11: cached tree hash preserves canonical bytes across retained and replaced nodes',()=>{
 const cache=new WeakMap();const p=packet(),base=validateMirrorPacket(p,new Map());
 const tree={root:p.root,nodes:[...base.values()],fonts:[],scroll:{x:0,y:0},focused:null,selection:null};
 assert.equal(mirrorTreeCanonicalJson(tree,cache),mirrorCanonicalJson(tree));assert.equal(mirrorTreeCanonicalJson(tree,cache),mirrorCanonicalJson(tree));
 const changed=validateMirrorPacket({...p,reset:false,nodes:[{...p.nodes[0]!,style:{color:'rgb(1, 2, 3)'}}]},base);tree.nodes=[...changed.values()];
 assert.equal(mirrorTreeCanonicalJson(tree,cache),mirrorCanonicalJson(tree));
});

test('MP-08/MP-11: sorted Rust style maps cannot reset vendor paint properties after assignment',()=>{
 const properties=new Map<string,string>();const element={removeAttribute(){properties.clear()},style:{setProperty(key:string,value:string){if(key==='all')properties.clear();properties.set(key,value)}}} as unknown as HTMLElement;
 const renderer=Object.create(BrowserMirrorRenderer.prototype) as {resources:Map<string,string>;style(element:HTMLElement,style:Record<string,string>):void};renderer.resources=new Map();
 renderer.style(element,{'-webkit-text-stroke-width':'2px',all:'initial',color:'black'});assert.equal(properties.get('-webkit-text-stroke-width'),'2px');assert.equal(properties.get('color'),'black');
});

test('MP-08/MP-11: nested event listeners bind once per Document independent of CSP reconciliation',async()=>{
 const listeners=new Map<string,Set<EventListener>>(),doc={addEventListener(kind:string,fn:EventListener){if(!listeners.has(kind))listeners.set(kind,new Set());listeners.get(kind)!.add(fn)},removeEventListener(kind:string,fn:EventListener){listeners.get(kind)?.delete(fn)},dispatchEvent(event:Event){for(const fn of listeners.get(event.type)??[])fn(event)}},node={} as Node,actions:unknown[]=[];
 const renderer=Object.create(BrowserMirrorRenderer.prototype) as any;
 Object.assign(renderer,{documentBindings:new Map(),disposed:false,applying:false,sequence:8,documentId:'d',inputChain:Promise.resolve(),pendingInputs:0,localFocus:null,doc:null,ids:new WeakMap([[node,'n1']]),records:new Map([['n1',{id:'n1',kind:'element'}]]),input:async(action:unknown)=>{actions.push(action)},failure:(error:unknown)=>{throw error}});
 for(let packet=0;packet<8;packet++)renderer.bindEvents(doc);
 for(const [kind,extra]of [['click',{}],['beforeinput',{inputType:'insertText',data:'Q',isComposing:false}],['keydown',{key:'Enter'}],['wheel',{deltaX:0,deltaY:20}] ] as const){const event=new Event(kind);Object.assign(event,extra);Object.defineProperty(event,'composedPath',{value:()=>[node]});doc.dispatchEvent(event)}
 await renderer.inputChain;assert.equal(actions.length,4);assert.equal(renderer.documentBindings.size,1);assert.equal(renderer.documentBindings.get(doc).length,8);
 renderer.releaseDocuments(new Set());assert.equal(renderer.documentBindings.size,0);const event=new Event('click');Object.defineProperty(event,'composedPath',{value:()=>[node]});doc.dispatchEvent(event);await renderer.inputChain;assert.equal(actions.length,4);
});

test('MP-08/MP-10: scalar paint patches preserve layout and removed keys force a clean reset',()=>{
 const properties=new Map<string,string>(),writes:string[]=[];
 const element={removeAttribute(){properties.clear();writes.push('reset')},style:{setProperty(key:string,value:string){properties.set(key,value);writes.push(key)}}} as unknown as HTMLElement;
 const renderer=Object.create(BrowserMirrorRenderer.prototype) as any;renderer.resources=new Map();
 const previous={all:'initial',color:'rgb(1, 2, 3)',width:'100px'};renderer.style(element,previous);writes.length=0;
 renderer.style(element,{...previous,color:'rgb(4, 5, 6)'},previous);assert.deepEqual(writes,['color']);assert.equal(properties.get('width'),'100px');writes.length=0;
 renderer.style(element,{all:'initial',color:'black'},previous);assert.equal(writes[0],'reset');assert(!properties.has('width'));
});

test('MP-08/MP-11: editable state admits only normalized inert enum values',()=>{
 for(const contenteditable of ['true','false','plaintext-only'])validateMirrorNode({id:'n1',parent:null,children:[],kind:'element',tag:'div',attributes:{contenteditable}});
 for(const contenteditable of ['', 'inherit','javascript:alert(1)'])assert.throws(()=>validateMirrorNode({id:'n1',parent:null,children:[],kind:'element',tag:'div',attributes:{contenteditable}}));
});


test('MP-08/MP-11: keys following native focus progress use the guarded native input bridge',async()=>{
 const listeners=new Map<string,EventListener>(),node={} as Node,actions:unknown[]=[];
 const doc={addEventListener(kind:string,fn:EventListener){listeners.set(kind,fn)},removeEventListener(){}};
 const renderer=Object.create(BrowserMirrorRenderer.prototype) as any;
 Object.assign(renderer,{documentBindings:new Map(),disposed:false,applying:false,sequence:1,documentId:'d',inputChain:Promise.resolve(),pendingInputs:0,localFocus:null,nativeFocus:null,doc:null,ids:new WeakMap([[node,'n1']]),records:new Map([['n1',{id:'n1',kind:'element'}]]),input:async(action:unknown)=>{actions.push(action)},failure:(error:unknown)=>{throw error}});
 renderer.bindEvents(doc);
 const event=(kind:string,extra:Record<string,unknown>)=>{const e=new Event(kind);Object.assign(e,extra);Object.defineProperty(e,'composedPath',{value:()=>[node]});listeners.get(kind)!(e)};
 event('keydown',{key:'Tab'});await renderer.inputChain;
 // The host has captured B at sequence2 but its delayed response has not painted.
 event('keydown',{key:'Backspace'});event('keydown',{key:'ArrowLeft'});event('keydown',{key:'Tab'});event('keydown',{key:'Enter'});
 event('beforeinput',{inputType:'insertText',data:'Q',isComposing:false});await renderer.inputChain;
 assert.deepEqual(actions,[{kind:'key',key:'Tab'},...['Backspace','ArrowLeft','Tab','Enter'].map(key=>({kind:'coordinate',input:{kind:'key',key}})),{kind:'coordinate',input:{kind:'text',text:'Q'}}]);
 event('click',{});event('keydown',{key:'ArrowRight'});await renderer.inputChain;
 assert.deepEqual(actions.slice(-2),[{kind:'click',node_id:'n1'},{kind:'key',key:'ArrowRight'}],'explicit pointer progress clears native keyboard mode');
});

test('MP-08/MP-11: Shift+Tab keeps its backward direction in ordinary and native-focus branches',async()=>{
 const listeners=new Map<string,EventListener>(),node={} as Node,actions:unknown[]=[];
 const doc={addEventListener(kind:string,fn:EventListener){listeners.set(kind,fn)},removeEventListener(){}};
 const renderer=Object.create(BrowserMirrorRenderer.prototype) as any;
 Object.assign(renderer,{documentBindings:new Map(),disposed:false,applying:false,sequence:1,documentId:'d',inputChain:Promise.resolve(),pendingInputs:0,localFocus:{node_id:'n1',document_id:'d',through_sequence:2},nativeFocus:null,doc:null,ids:new WeakMap([[node,'n1']]),records:new Map([['n1',{id:'n1',kind:'element'}]]),input:async(action:unknown)=>{actions.push(action)},failure:(error:unknown)=>{throw error}});
 renderer.bindEvents(doc);
 const event=(kind:string,extra:Record<string,unknown>)=>{const e=new Event(kind,{cancelable:true});Object.assign(e,extra);Object.defineProperty(e,'composedPath',{value:()=>[node]});listeners.get(kind)!(e);return e};
 assert.ok(event('keydown',{key:'Tab',shiftKey:true}).defaultPrevented);await renderer.inputChain;
 assert.equal(renderer.localFocus,null,'Shift+Tab moves focus like Tab');
 assert.deepEqual(renderer.nativeFocus,{document_id:'d',through_sequence:2});
 event('keydown',{key:'Tab',shiftKey:true});event('keydown',{key:'Tab'});event('keydown',{key:'ArrowLeft',shiftKey:true});await renderer.inputChain;
 assert.deepEqual(actions,[{kind:'key',key:'Shift+Tab'},{kind:'coordinate',input:{kind:'key',key:'Shift+Tab'}},{kind:'coordinate',input:{kind:'key',key:'Tab'}},{kind:'coordinate',input:{kind:'key',key:'ArrowLeft'}}]);
});

test('MP-08/MP-10: mirror wheel retains fractions and normalizes line/page units per document',async()=>{
 const actions:any[]=[],node={} as Node,listeners=new Map<string,EventListener>();
 const doc={defaultView:{innerHeight:800,getComputedStyle:()=>({lineHeight:'20px'})},addEventListener(kind:string,fn:EventListener){listeners.set(kind,fn)},removeEventListener(){}};
 const renderer=Object.create(BrowserMirrorRenderer.prototype) as any;
 Object.assign(renderer,{documentBindings:new Map(),disposed:false,applying:false,sequence:1,documentId:'d',inputChain:Promise.resolve(),pendingInputs:0,localFocus:null,nativeFocus:null,ids:new WeakMap([[node,'n1']]),records:new Map([['n1',{id:'n1',kind:'element'}]]),input:async(action:unknown)=>actions.push(action),failure:(error:unknown)=>{throw error}});
 renderer.bindEvents(doc);
 const wheel=(deltaY:number,deltaMode=0)=>{const event=new Event('wheel');Object.assign(event,{deltaX:0,deltaY,deltaMode});Object.defineProperty(event,'composedPath',{value:()=>[node]});listeners.get('wheel')!(event)};
 for(let i=0;i<10;i++)wheel(.4);await renderer.inputChain;
 assert.equal(actions.reduce((sum,action)=>sum+action.delta_y,0),4,'fractional pixel motion accumulates');
 actions.length=0;wheel(.5,1);wheel(.5,2);await renderer.inputChain;
 assert.deepEqual(actions.map(action=>action.delta_y),[10,400]);
 actions.length=0;wheel(-.4);wheel(-.4);wheel(-.4);await renderer.inputChain;assert.equal(actions.reduce((sum,action)=>sum+action.delta_y,0),-1);
 renderer.releaseDocuments(new Set());renderer.bindEvents(doc);actions.length=0;wheel(.4);await renderer.inputChain;assert.equal(actions.length,0,'released document drops its old remainder');
});
