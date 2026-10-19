// MP-08 / MP-11: fail-first scoped AT-SPI targets, fallback and stale denial.
import test from 'node:test';
import assert from 'node:assert/strict';
import { NativeAccessibility } from './native-accessibility.mjs';
const binding={surface_id:'s',generation:'g',width:1280,height:800,environment:{},ownedProcesses:async()=>[{pid:200,started:'1'}]};
const tree={available:true,complete:true,nodes:[{pid:200,started:'1',path:[0],role:'push button',name:'Public',bounds:[1,2,3,4],actions:['click'],states:[]}],protected:false};
test('MP-11 #904 review 1 agent AT-SPI actions need no clipboard-owner gate',async()=>{
 for(const context of [{observer:'agent:a',command:{}},{observer:'terminal:a',command:{_agent_input:true}},{observer:'terminal:a',command:{}}]) {
  const ops=[];let dispatched;
  const native=new NativeAccessibility({binding:()=>binding,execute:async request=>{
   ops.push(request.op);
   if(request.op==='clipboard_read')return {text:'[protected]'};
   if(request.op==='accessibility_action'){dispatched=request;return {applied:true};}
   return {...tree,nodes:[{...tree.nodes[0],name:'Paste'}]};
  }});
  const seen=await native.snapshot(context.observer,{});
  assert.equal((await native.action(context.observer,{...context.command,target_id:seen.nodes[0].target_id,tree_revision:seen.tree_revision,action:'click'},{})).applied,true);
  assert(!ops.includes('clipboard_read'));
  assert.equal(dispatched.agent_input,undefined);
 }
});
test('MP-08 / MP-11 foreground application survives browser action and traversal budgets',async()=>{
 const current={...tree,active_window:{pid:300,started:'2',path:[0]},nodes:Array.from({length:600},(_,i)=>({...tree.nodes[0],path:[i],name:'Browser panel '+i,states:['showing','focused'],actions:['doDefault']}))};
 current.nodes.push({...tree.nodes[0],pid:300,started:'2',path:[0],role:'frame',name:'meeting.odt - LibreOffice Writer',states:['showing'],actions:[]});
 current.nodes.push({...tree.nodes[0],pid:300,started:'2',path:[0,1],name:'File',states:['showing','enabled'],actions:['click']});
 const native=new NativeAccessibility({binding:()=>binding,execute:async()=>structuredClone(current)});
 const seen=await native.snapshot('agent:a',{});
 assert(seen.nodes.some(n=>n.name==='File'&&n.actions.includes('click')));
 assert(seen.nodes.some(n=>n.name==='meeting.odt - LibreOffice Writer'));
 assert.equal(seen.active_window,undefined,'MP-11 private process/frame selector never enters public protocol');
 const file=seen.nodes.find(n=>n.name==='File');current.active_window={pid:200,started:'1',path:[0]};
 await assert.rejects(native.action('agent:a',{target_id:file.target_id,tree_revision:seen.tree_revision,action:'click'},{}),e=>e.code==='user_domain_stale_reference');
});
test('MP-11 handles are scoped to observer, surface, app identity and current revision',async()=>{
 let current=structuredClone(tree),actions=0;
 const native=new NativeAccessibility({binding:()=>binding,execute:async request=>{if(request.op==='accessibility_action'){actions++;return {applied:true};}return current;}});
 const observed=await native.snapshot('agent:a',{});const target=observed.nodes[0].target_id;
 await assert.rejects(native.action('agent:b',{target_id:target,tree_revision:observed.tree_revision,action:'click'},{}),error=>error.code==='user_domain_stale_reference');
 current.nodes[0].started='2';
 await assert.rejects(native.action('agent:a',{target_id:target,tree_revision:observed.tree_revision,action:'click'},{}),error=>error.code==='user_domain_stale_reference');
 assert.equal(actions,0);
});
test('MP-08 unavailable accessibility explicitly falls back to protected OCR',async()=>{
 const native=new NativeAccessibility({binding:()=>binding,execute:async()=>({available:false,complete:false,nodes:[]})});
 const result=await native.snapshot('agent:a',{});assert.equal(result.fallback,'ocr');assert.deepEqual(result.nodes,[]);
});
test('MP-11 password nodes and policy values never enter target names',async()=>{
 const native=new NativeAccessibility({binding:()=>binding,execute:async()=>({...tree,nodes:[{...tree.nodes[0],role:'password text',protected:true,name:'private-canary'}]})});
 const result=await native.snapshot('agent:a',{values:['private-canary']});assert(!JSON.stringify(result).includes('private-canary'));assert.equal(result.nodes[0].name,'[protected]');assert.deepEqual(result.nodes[0].actions,[]);
});

test('MP-11 target protection invalidates old handles without hiding public app controls', async () => {
  let actions = 0;
  const native = new NativeAccessibility({ binding: () => binding, execute: async request => {
    if (request.op === 'accessibility_action') { actions++; return { applied: true }; }
    return structuredClone(tree);
  } });
  const before = await native.snapshot('agent:a', {});
  const policy = { targets: [{ target_id: 'password-field' }] };
  await assert.rejects(native.action('agent:a', {
    target_id: before.nodes[0].target_id, tree_revision: before.tree_revision, action: 'click',
  }, policy));
  const after = await native.snapshot('agent:a', policy);
  assert.notEqual(after.tree_revision, before.tree_revision);
  assert.equal(after.nodes[0].name, 'Public');
  assert.deepEqual(after.nodes[0].actions, ['click']);
  assert.equal(actions, 0);
  assert.equal((await native.action('agent:a',{target_id:after.nodes[0].target_id,tree_revision:after.tree_revision,action:'click'},policy)).applied,true);
  assert.equal(actions,1);
});

test('MP-08 stale handles expose the existing typed refusal instead of a generic backend failure', async () => {
 const native=new NativeAccessibility({binding:()=>binding,execute:async()=>structuredClone(tree)});
 const observed=await native.snapshot('agent:a',{});const command={target_id:observed.nodes[0].target_id,tree_revision:observed.tree_revision,action:'click'};
 await native.action('agent:a',command,{});
 await assert.rejects(native.action('agent:a',command,{}), error=>error.code==='user_domain_stale_reference');
});


test('MP-08 OpenCode observation budget retains visible actionable targets and fences the full tree', async () => {
 let current={...tree,nodes:Array.from({length:319},(_,i)=>({...tree.nodes[0],path:[i],name:'Hidden '+i+' x'.repeat(100),states:[],actions:[]}))};
 current.nodes.push({...tree.nodes[0],path:[319],name:'File',states:['showing','enabled'],actions:['click']});
 const native=new NativeAccessibility({binding:()=>binding,execute:async request=>request.op==='accessibility_action'?{applied:true}:structuredClone(current)});
 const seen=await native.snapshot('agent:a',{});
 assert(Buffer.byteLength(JSON.stringify(seen))<=16384,'MP-08 provider observation must fit below harness truncation');
 assert.equal(seen.complete,false,'MP-08 pruned observation explicitly requires rediscovery/OCR');
 assert.equal(seen.fallback,'ocr');
 const file=seen.nodes.find(n=>n.name==='File');assert(file?.target_id&&file.actions.includes('click'));
 current.nodes[0].name='hidden tree changed';
 await assert.rejects(native.action('agent:a',{target_id:file.target_id,tree_revision:seen.tree_revision,action:'click'},{}),e=>e.code==='user_domain_stale_reference');
});

test('MP-08 / MP-11 visible window context survives menus and the wrapped provider receipt budget', async () => {
 const current={...tree,nodes:Array.from({length:120},(_,i)=>({...tree.nodes[0],path:[i],name:'Menu '+i,states:['showing','enabled'],actions:['click']}))};
 current.nodes.push({...tree.nodes[0],path:[120],role:'frame',name:'Chariox pixel fixture',states:['showing'],actions:[]});
 const native=new NativeAccessibility({binding:()=>binding,execute:async()=>structuredClone(current)});
 const snapshot=await native.snapshot('agent:a',{});
 assert(snapshot.nodes.some(n=>n.name==='Chariox pixel fixture'));
 const payload={...snapshot,agent_id:'agent-2',session_id:'1234567890123456',slice_id:'slice-1',source:'computer_controller'};
 const receipt={_meta:null,content:[{type:'text',text:JSON.stringify(payload)}],structuredContent:payload};
 assert(Buffer.byteLength(JSON.stringify(receipt,null,2))<=12*1024,'MP-08 complete MCP receipt must remain parseable in provider history');
 assert.equal(snapshot.fallback,'ocr');
});

test('MP-08 / MP-11 failed or cancelled dispatch consumes handles even with an unchanged tree', async () => {
 for (const reason of ['helper failed after effect', 'action cancelled after effect']) {
  let effects=0;
  const native=new NativeAccessibility({binding:()=>binding,execute:async request=>{
   if(request.op==='accessibility_action'){effects++;throw Error(reason);}
   return structuredClone(tree);
  }});
  const seen=await native.snapshot('agent:a',{}),command={target_id:seen.nodes[0].target_id,tree_revision:seen.tree_revision,action:'click'};
  await assert.rejects(native.action('agent:a',command,{}),new RegExp(reason));
  await assert.rejects(native.action('agent:a',command,{}),e=>e.code==='user_domain_stale_reference');
  assert.equal(effects,1,'MP-11 uncertain dispatch cannot be retried with the same handle');
 }
});
test('MP-08 snapshot announces masked windows without accessibility so agents can still act on them',async()=>{
 const current={...tree,uncovered:[[20,30,246,152]],masks:[[20,30,246,50],[20,80,80,102]]};
 const native=new NativeAccessibility({binding:()=>binding,execute:async()=>structuredClone(current)});
 const result=await native.snapshot('agent:a',{});
 assert.deepEqual(result.masked,[[20,30,246,50],[20,80,80,102]]);
 assert.equal(result.complete,false);assert.equal(result.fallback,'ocr');
 const plain=await new NativeAccessibility({binding:()=>binding,execute:async()=>structuredClone(tree)}).snapshot('agent:a',{});
 assert.deepEqual(plain.masked,[]);
});

// MP-08/MP-10/MP-11 #904 scale2: public bounds and pointer input share desktop pixels.
import {NativeComputer} from './native-computer.mjs';
import {RoomNativeAccessibility} from './room-native-accessibility.mjs';
test('MP-08 / MP-11 host and Room scaled snapshot centers reach the physical pointer unchanged',async()=>{
 for(const placement of ['host','slice']) {
  const scoped={...binding,width:2560,height:1600};let clicked;
  const execute=async request=>request.op==='accessibility'
   ? {...tree,nodes:[{...tree.nodes[0],bounds:[100,50,20,10],desktop_bounds:[200,100,40,20]}]}
   : (clicked=request.input,{applied:true});
  const native=placement==='host'?new NativeAccessibility({binding:()=>scoped,execute}):new RoomNativeAccessibility({binding:()=>scoped,execute});
  const policy={unknown:false,values:[],targets:[]};
  const seen=placement==='host'?await native.snapshot('agent:a',policy):await native.request({op:'snapshot',observer:'agent:a',policy});
  const [x,y,w,h]=seen.nodes[0].bounds;
  assert.deepEqual(seen.nodes[0].bounds,[200,100,40,20]);
  assert.equal(seen.nodes[0].desktop_bounds,undefined,'private geometry metadata stays below clients');
  const computer=new NativeComputer({placement,binding:()=>scoped,execute});
  await computer.request({op:'input',surface_id:scoped.surface_id,generation:scoped.generation,input:{kind:'click',x:x+w/2,y:y+h/2}},policy);
  assert.deepEqual(clicked,{kind:'click',x:220,y:110});
 }
});

test('MP-08/MP-11 public native login controls stay usable after a Vault fill',async()=>{
 const nodes=[{pid:20,started:'10',path:[0],role:'frame',name:'Cuvault Native Login',actions:[],states:['showing']},{pid:20,started:'10',path:[0,1],role:'push button',name:'Sign in',actions:['click'],states:['showing']},{pid:20,started:'10',path:[0,2],role:'password text',name:'synthetic-private-value',protected:true,actions:[],states:['showing','editable']}];
 const binding={surface_id:'s',generation:'g',environment:{},ownedProcesses:async()=>[{pid:20,started:'10'}]};
 const policy={unknown:false,values:['synthetic-private-value'],targets:[{kind:'native',target:{active_window:41}}]};
 const native=new NativeAccessibility({binding:()=>binding,execute:async request=>request.op==='accessibility_action'?{applied:true}:{available:true,complete:true,nodes}});
 const seen=await native.snapshot('agent:a',policy);
 assert(seen.nodes.some(n=>n.role==='frame'&&n.name==='Cuvault Native Login'));
 const button=seen.nodes.find(n=>n.name==='Sign in');assert(button?.actions.includes('click'));
 assert(!JSON.stringify(seen).includes(policy.values[0]));
 assert.equal((await native.action('agent:a',{target_id:button.target_id,tree_revision:seen.tree_revision,action:'click'},policy)).applied,true);
});

test('MP-11 private selection evidence is never projected to Computer observers',async()=>{
 const native=new NativeAccessibility({binding:()=>binding,execute:async()=>({...tree,nodes:[{...tree.nodes[0],text:'private-helper-evidence'}]})});
 const result=await native.snapshot('agent:a',{});
 assert(!JSON.stringify(result).includes('private-helper-evidence'));
 assert(!Object.hasOwn(result.nodes[0],'text'));
});

test('MP-08 / MP-11 protected focus exposes no value and returns fresh action handles',async()=>{
 let actions=0;
 const password={...tree.nodes[0],role:'password text',name:'private-focus-canary',protected:true,states:['showing','enabled','editable'],actions:['focus','activate']};
 const native=new NativeAccessibility({binding:()=>binding,execute:async request=>{
  if(request.op==='accessibility_action'){actions++;assert.equal(request.action,'focus');return {applied:true};}
  return {...tree,nodes:[password]};
 }});
 const before=await native.snapshot('agent:a',{}),target=before.nodes[0];
 assert.deepEqual(target.actions,['focus']);assert.equal(target.name,'[protected]');
 const after=await native.action('agent:a',{target_id:target.target_id,tree_revision:before.tree_revision,action:'focus'},{});
 assert.equal(after.applied,true);assert(after.tree_revision>before.tree_revision);
 assert(after.nodes[0].target_id!==target.target_id);assert(!JSON.stringify(after).includes('private-focus-canary'));
 await assert.rejects(native.action('agent:a',{target_id:target.target_id,tree_revision:before.tree_revision,action:'focus'},{}),e=>e.code==='user_domain_stale_reference');
 await assert.rejects(native.action('agent:a',{target_id:after.nodes[0].target_id,tree_revision:after.tree_revision,action:'activate'},{}),e=>e.code==='user_domain_not_granted');
 assert.equal(actions,1);
});

test('MP-11 private selection evidence does not expire focus handles; protection changes do',async()=>{
 let actions=0,current={...tree,nodes:[{...tree.nodes[0],role:'text',states:['showing','enabled','editable'],actions:['focus'],text:'old public entry'}]};
 const native=new NativeAccessibility({binding:()=>binding,execute:async request=>{
  if(request.op==='accessibility_action'){actions++;return {applied:true};}
  return structuredClone(current);
 }});
 const before=await native.snapshot('agent:a',{}),target=before.nodes[0];
 current.nodes[0].text='edited public entry';
 const after=await native.action('agent:a',{target_id:target.target_id,tree_revision:before.tree_revision,action:'focus'},{});
 assert.equal(after.applied,true);assert.equal(actions,1);
 current.nodes[0].protected=true;
 await assert.rejects(native.action('agent:a',{target_id:after.nodes[0].target_id,tree_revision:after.tree_revision,action:'focus'},{}),e=>e.code==='user_domain_stale_reference');
 assert.equal(actions,1);
});

test('MP-08/MP-11 browser navigation remains actionable beside a Vault-filled field without exposing its value',async()=>{
 const policy={values:['test-vault-canary'],targets:[{kind:'browser',target_id:'filled'}]};let applied=0;
 const native=new NativeAccessibility({binding:()=>binding,execute:async request=>{
  if(request.op==='accessibility_action'){applied++;return {applied:true};}
  return {...tree,nodes:[{...tree.nodes[0],name:'Accept cookies'},{...tree.nodes[0],path:[1],role:'text',name:'test-vault-canary',actions:[]},{...tree.nodes[0],path:[2],role:'password text',protected:true,name:'test-password-canary',actions:[]}]};
 }});
 const seen=await native.snapshot('agent:a',policy);assert(!JSON.stringify(seen).includes('canary'));
 const button=seen.nodes.find(node=>node.name==='Accept cookies');assert(button?.actions.includes('click'));
 await native.action('agent:a',{tree_revision:seen.tree_revision,target_id:button.target_id,action:'click'},policy);
 assert.equal(applied,1);
});

// MP-08 / MP-11: Chromium marks structural panels actionable too.
test('MP-08 / MP-11 named browser navigation survives unnamed structural actions',async()=>{
 const nodes=Array.from({length:80},(_,i)=>({...tree.nodes[0],path:[0,i],role:'panel',name:'',states:['showing','enabled'],actions:['doDefault','showContextMenu']}));
 nodes.unshift({...tree.nodes[0],path:[0],role:'frame',name:'Public page',states:['showing'],actions:[]});
 for(const [i,name] of ['Back','Reload','Address and search bar'].entries())nodes.push({...tree.nodes[0],path:[0,100+i],role:i===2?'entry':'button',name,states:['showing','enabled'],actions:['press']});
 const native=new NativeAccessibility({binding:()=>binding,execute:async()=>({...tree,active_window:{pid:200,started:'1',path:[0]},nodes})});
 const seen=await native.snapshot('agent:a',{});
 for(const name of ['Back','Reload','Address and search bar'])assert(seen.nodes.some(n=>n.name===name),name);
 assert.equal(seen.complete,false);
});
// MP-08 / MP-11: tabs and window controls must not crowd out browser toolbar.
test('MP-08 / MP-11 browser toolbar survives named tabs and window controls',async()=>{
 const nodes=Array.from({length:60},(_,i)=>({...tree.nodes[0],browser:true,path:[0,i],name:'Tab control '+i,states:['showing','enabled'],actions:['press']}));
 nodes.push({...tree.nodes[0],browser:true,path:[0,90],role:'tool bar',name:'',states:['showing'],actions:[]});
 for(const [i,name] of ['Back','Reload','Address and search bar'].entries())nodes.push({...tree.nodes[0],browser:true,path:[0,90,i],role:i===2?'entry':'button',name,states:['showing','enabled'],actions:['press']});
 const native=new NativeAccessibility({binding:()=>binding,execute:async()=>({...tree,active_window:{pid:200,started:'1',path:[0]},nodes})});
 const seen=await native.snapshot('agent:a',{});
 for(const name of ['Back','Reload','Address and search bar'])assert(seen.nodes.some(n=>n.name===name),name);
});
