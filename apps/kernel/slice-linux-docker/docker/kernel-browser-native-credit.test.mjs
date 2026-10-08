// MP-08/MP-10/MP-11: empty-only shortcut retains every delivery/refusal fence.
import test from 'node:test';import assert from 'node:assert/strict';
import {nativeCreditEmpty,nativeRegionBaseCurrent,nativeRegionPending} from './kernel-browser-native-credit.mjs';
import {displayMaskRegions} from './kernel-browser-pixels.mjs';
function fixture(){
 const policy={},sample={raw:{},serial:1,document_id:'d',tab_id:'t'};
 const source={attested:true,policy,changedAt:100,valid:()=>true,allowed:()=>true,sample:()=>sample};
 const stream={tab_id:'t',document_id:'d',previous:{},compositorSerial:1,creditEpoch:0,acceptsCredit:()=>true,
  producer:{source,frames:[]},refiner:{quietMs:300}};
 return {source,stream,policy,sample,empty:()=>nativeCreditEmpty(stream,source,policy,0,-Infinity,1,200)};
}
test('MP-08/MP-10 busy native encoding of a fresh serial skips repeated CDP inventories',()=>{
 for(const work of ['active','pending']){
  const f=fixture();f.sample.serial=2;f.stream.producer[work]={};f.stream.canPatchNative=()=>false;
  assert.equal(f.empty(),true);
  f.stream.canPatchNative=()=>true;assert.equal(f.empty(),false,'a ready exact patch still gets admitted');
  f.stream.canPatchNative=()=>false;f.source.regionRevision=1;assert.equal(f.empty(),false,'mask retirement still takes the full fence');
 }
});
test('MP-10 recent native serial avoids an empty CDP poll but never samples pixels',()=>{
 const f=fixture();Object.defineProperty(f.sample.raw,'pixels',{get(){throw Error('must not observe pixels')}});assert(f.empty());
 assert.equal(nativeCreditEmpty(f.stream,f.source,f.policy,0,-Infinity,1,400),false,'exact deadline still uses protected capture');
 Object.assign(f.stream,{exact:true});Object.assign(f.stream.refiner,{latest:{},wanted:{native:true,source:f.source,document:'d',policy:f.policy,epoch:0,serial:1}});
 assert(nativeCreditEmpty(f.stream,f.source,f.policy,0,-Infinity,1,1000));
});
for(const [name,mutate] of Object.entries({
 revoked:f=>f.source.valid=()=>false,policy:f=>f.source.allowed=()=>false,retired:f=>f.source.closed=true,
 lossy:f=>f.source.attested=false,document:f=>f.sample.document_id='next',tab:f=>f.sample.tab_id='other',serial:f=>f.sample.serial=2,
 epoch:f=>f.stream.creditEpoch=1,cursor:f=>f.stream.acceptsCredit=()=>false,repair:f=>f.stream.repair=[{}],
 queued:f=>f.stream.producer.frames=[{}],encoding:f=>f.stream.producer.active={},pending:f=>f.stream.producer.pending={},
 encoderFailure:f=>f.stream.producer.failure=Error('fixture'),exactFailure:f=>f.stream.refiner.failure=Error('fixture'),
 policyIdentity:f=>f.source.policy={},sourceIdentity:f=>f.stream.producer.source={},
 }))test('MP-11 '+name+' retains the full observation/refusal path',()=>{const f=fixture();mutate(f);assert.equal(f.empty(),false)});

test('MP-08/MP-10 native exact60ms deadline cannot be hidden by empty credits',()=>{
 const f=fixture();f.stream.refiner.quietNativeMs=60;
 assert.equal(nativeCreditEmpty(f.stream,f.source,f.policy,0,-Infinity,1,159),true);
 assert.equal(nativeCreditEmpty(f.stream,f.source,f.policy,0,-Infinity,1,160),false);
});

test('MP-08/MP-10/MP-11 a fully exact contiguous native base needs no repeated full refinement',()=>{
 const f=fixture();f.stream.exact=true;
 assert.equal(nativeCreditEmpty(f.stream,f.source,f.policy,0,-Infinity,1,1000),true);
 f.source.regionRevision=1;
 assert.equal(nativeCreditEmpty(f.stream,f.source,f.policy,0,-Infinity,1,1000),false,'a retired protection binding cannot reuse exact bytes');
});

test('MP-11 region refresh reuses only post-fence identical masks on the exact displayed native base',()=>{
 function ready(){const f=fixture();f.stream.exact=true;f.stream.compositorMasks='[]';f.sample.raw={nativeExact(){},base_serial:1};return f}
 const f=ready();assert(nativeRegionBaseCurrent(f.stream,f.source,'d',f.policy));
 for(const mutate of [f=>f.stream.exact=false,f=>f.sample.raw.base_serial=2,f=>f.source.valid=()=>false,f=>f.source.policy={},f=>f.source.allowed=()=>false,f=>f.source.attested=false,f=>f.stream.producer.source={},f=>f.sample.document_id='next',f=>f.sample.tab_id='other',f=>f.sample.raw[displayMaskRegions]=[{x:0,y:0,width:10,height:10}],f=>f.source.sample=()=>null]){
  const r=ready();mutate(r);assert.equal(nativeRegionBaseCurrent(r.stream,r.source,'d',r.policy),false);
 }
});
test('MP-11 a region fence emits nothing while preserving only exact-canvas bookkeeping',()=>{
 const f=fixture();f.source.tab={tab_id:'t',document_id:'d'};f.source.sample=()=>null;
 assert(nativeRegionPending(f.stream,f.source,'d',f.policy));
 f.source.valid=()=>false;assert.equal(nativeRegionPending(f.stream,f.source,'d',f.policy),false);
 f.source.valid=()=>true;f.source.tab.document_id='next';assert.equal(nativeRegionPending(f.stream,f.source,'d',f.policy),false);
 f.source.tab.document_id='d';f.source.policy={};assert.equal(nativeRegionPending(f.stream,f.source,'d',f.policy),false);
});
// MP-08/MP-10/MP-11: a protection refresh readback proved identical to the
// delivered frame keeps the exact canvas; other serials or masks do not.
test('MP-08/MP-10/MP-11 identical refresh readbacks keep an exact canvas only when adjacent to the delivered frame',async()=>{
  const {identicalToDelivered}=await import('./kernel-browser-native-credit.mjs');
  const stream={compositorSerial:7};
  assert.equal(identicalToDelivered(stream,{serial:8,raw:{identical:true}}),true);
  assert.equal(identicalToDelivered(stream,{serial:9,raw:{identical:true}}),false,'a skipped readback breaks the proof');
  assert.equal(identicalToDelivered(stream,{serial:8,raw:{identical:false}}),false);
  assert.equal(identicalToDelivered({compositorSerial:null},{serial:1,raw:{identical:true}}),false);
});

test('MP-08/MP-10/MP-11 attested CDP fallback cannot enter native region reuse or crash display credits',async()=>{
 const {CompositorSource}=await import('./kernel-browser-compositor.mjs');
 const policy={},source=new CompositorSource({tab:{tab_id:'t',document_id:'d'},policy,allowed:()=>true});
 source.attested=true;
 const stream={tab_id:'t',document_id:'d',exact:true,producer:{source},compositorMasks:'[]'};
 assert.equal(nativeRegionPending(stream,source,'d',policy),false);
 source.latest={tab_id:'t',document_id:'d',raw:{nativeExact(){},base_serial:1}};stream.compositorSerial=1;
 assert.equal(nativeRegionBaseCurrent(stream,source,'d',policy),false,'native proof is mandatory even with otherwise matching bookkeeping');
});
