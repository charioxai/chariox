// MP-08/MP-10/MP-11: empty-only shortcut retains every delivery/refusal fence.
import test from 'node:test';import assert from 'node:assert/strict';
import {nativeCreditEmpty} from './kernel-browser-native-credit.mjs';
function fixture(){
 const policy={},sample={raw:{},serial:1,document_id:'d',tab_id:'t'};
 const source={attested:true,policy,changedAt:100,valid:()=>true,allowed:()=>true,sample:()=>sample};
 const stream={tab_id:'t',document_id:'d',previous:{},compositorSerial:1,creditEpoch:0,acceptsCredit:()=>true,
  producer:{source,frames:[]},refiner:{quietMs:300}};
 return {source,stream,policy,sample,empty:()=>nativeCreditEmpty(stream,source,policy,0,-Infinity,1,200)};
}
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
