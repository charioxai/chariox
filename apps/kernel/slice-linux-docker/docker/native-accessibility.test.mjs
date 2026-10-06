// MP-08 / MP-11: fail-first scoped AT-SPI targets, fallback and stale denial.
import test from 'node:test';
import assert from 'node:assert/strict';
import { NativeAccessibility } from './native-accessibility.mjs';
const binding={surface_id:'s',generation:'g',width:1280,height:800,environment:{},ownedProcesses:async()=>[{pid:200,started:'1'}]};
const tree={available:true,complete:true,nodes:[{pid:200,started:'1',path:[0],role:'push button',name:'Public',bounds:[1,2,3,4],actions:['click'],states:[]}],protected:false};
test('MP-11 handles are scoped to observer, surface, app identity and current revision',async()=>{
 let current=structuredClone(tree),actions=0;
 const native=new NativeAccessibility({binding:()=>binding,execute:async request=>{if(request.op==='accessibility_action'){actions++;return {applied:true};}return current;}});
 const observed=await native.snapshot('agent:a',{});const target=observed.nodes[0].target_id;
 await assert.rejects(native.action('agent:b',{target_id:target,tree_revision:observed.tree_revision,action:'click'},{}),/target/);
 current.nodes[0].started='2';
 await assert.rejects(native.action('agent:a',{target_id:target,tree_revision:observed.tree_revision,action:'click'},{}),/stale/);
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

test('MP-11 target protection invalidates cached snapshots and direct old handles on an unchanged tree', async () => {
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
  assert.equal(after.nodes[0].name, '[protected]');
  assert.deepEqual(after.nodes[0].actions, []);
  assert.equal(actions, 0);
});
