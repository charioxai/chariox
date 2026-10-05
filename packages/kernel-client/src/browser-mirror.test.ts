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
test('MP-08: protocol 432/unknown rejects before allocating a subscription or frame',async()=>{
 assert.equal(browserMirrorMinimumProtocolVersion,433)
 for(const protocolVersion of [432,427,0,NaN])await assert.rejects(attachBrowserMirror({protocolVersion,request:async()=>assert.fail('must not request')},{} as HTMLElement,{tab_id:'t',generation:1,device_scale_factor:1},()=>{}),/protocol 433/)
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
