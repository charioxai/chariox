// MP-08/MP-10/MP-11: supplementary shared producer lifetime regressions.
import test from 'node:test';
import assert from 'node:assert/strict';
import { DisplaySessions, displayResumeGraceMs } from './kernel-display-resumption.mjs';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

function fixture(kind='browser') {
  let now=1000,invalidations=0,closed=0;
  const policy={},binding={},stream={subscription_id:'old',source_kind:kind,observed_by:'owner',
    sequence:9,expires:61000,lastCreditAt:1000,document_id:'doc',
    invalidate(){invalidations++},async close(){closed++}};
  const host={generation:7,protection:policy,displays:new Map([['old',stream]]),armDisplayExpiry(){}};
  const sessions=new DisplaySessions(host,{now:()=>now,clock:()=>now,id:()=> 'new'});
  return {host,stream,sessions,binding,advance(ms){now+=ms},get invalidations(){return invalidations},get closed(){return closed}};
}
for(const kind of ['browser','desktop'])test(`MP-08/MP-10 ${kind} resumes the same producer with a fresh delivery identity`,async()=>{
  const f=fixture(kind),producer={};f.stream.producer=producer;
  await f.sessions.suspend('old','owner',7,()=>f.binding);
  assert.equal(f.stream.expires,1000+displayResumeGraceMs);
  assert.equal(f.invalidations,1,'prepare an independent frame while detached');
  f.advance(700);
  const resumed=await f.sessions.resume('old','owner',7,()=>f.binding);
  assert.equal(resumed.subscription_id,'new');assert.equal(resumed.resumed,true);
  assert.equal(f.host.displays.has('old'),false);assert.equal(f.host.displays.get('new'),f.stream);
  assert.equal(f.stream.producer,producer);assert.equal(f.closed,0);
  assert.equal(f.invalidations,1,'retain the prepared keyframe and warm encoder');
  assert.equal(f.stream.sequence,0);
});
test('MP-11 a resume hint never grants a different observer or generation',async()=>{
  const f=fixture();await f.sessions.suspend('old','owner',7,()=>f.binding);
  for(const [owner,generation] of [['other',7],['owner',8]])await assert.rejects(f.sessions.resume('old',owner,generation,()=>f.binding));
  assert.equal(f.host.displays.get('old'),f.stream);assert.equal(f.closed,0,'a forged request cannot retire the legitimate owner');
});
test('MP-11 changed source or protection revokes retained producer pixels',async()=>{
  for(const change of ['binding','policy']){
    const f=fixture();await f.sessions.suspend('old','owner',7,()=>f.binding);
    if(change==='policy')f.host.protection={};
    await assert.rejects(f.sessions.resume('old','owner',7,()=>change==='binding'?{}:f.binding));
    assert.equal(f.host.displays.size,0);assert.equal(f.closed,1);
  }
});
test('MP-11 grace expiry cannot be extended by repeated suspend or late resume',async()=>{
  const f=fixture();await f.sessions.suspend('old','owner',7,()=>f.binding);
  f.advance(1000);await f.sessions.suspend('old','owner',7,()=>f.binding);
  assert.equal(f.stream.expires,1000+displayResumeGraceMs);
  f.advance(displayResumeGraceMs);
  await assert.rejects(f.sessions.resume('old','owner',7,()=>f.binding));assert.equal(f.closed,1);
});
test('MP-08/MP-11 an unannounced transport loss resumes only within the last-credit grace',async()=>{
  const f=fixture();f.advance(4000);
  assert.equal((await f.sessions.resume('old','owner',7,()=>f.binding)).resumed,true);
  const late=fixture();late.advance(displayResumeGraceMs+1);
  await assert.rejects(late.sessions.resume('old','owner',7,()=>late.binding));assert.equal(late.closed,1);
});
test('MP-11 wall-clock rollback cannot extend a parked producer grace',async()=>{
  const f=fixture();let clock=1000;
  f.sessions.clock=()=>clock;
  await f.sessions.suspend('old','owner',7,()=>f.binding);
  f.advance(-500);clock+=displayResumeGraceMs;
  await assert.rejects(f.sessions.resume('old','owner',7,()=>f.binding));assert.equal(f.closed,1);
});
test('MP-11 browser resume identity fences region revisions and replaced capture sources',async()=>{
  const source={regionRevision:1,closed:false,allowed:()=>true};
  const host=Object.create(KernelBrowserHost.prototype);
  Object.assign(host,{generation:7,protection:{},compositors:new Map([['tab',{source}]]),
    displayTarget:async()=>({tab_id:'tab',target_id:'target',document_id:'doc'}),
    viewport:()=>({width:1280,height:800,device_scale_factor:1})});
  const stream={tab_id:'tab',document_id:'doc'};
  const first=await host.displayResumeBinding(stream);
  assert.equal(await host.displayResumeBinding(stream),first);
  source.regionRevision++;
  const moved=await host.displayResumeBinding(stream);assert.notEqual(moved,first);
  host.compositors.set('tab',{source:{...source}});
  assert.notEqual(await host.displayResumeBinding(stream),moved);
  host.displayTarget=async()=>({document_id:'different'});
  await assert.rejects(host.displayResumeBinding(stream));
});

for(const kind of ['region','browser_chrome'])test(`MP-08/MP-11 ${kind} resume retains the admitted source and input epoch`,async()=>{
 const f=fixture('region'),source={closed:false,valid:()=>current,allowed:()=>true};let current=true;
 const host=Object.create(KernelBrowserHost.prototype);
 Object.assign(host,{generation:7,protection:f.host.protection,displayTarget:async()=>{throw Error('MP-11: region resume must use its lease')}});
 Object.assign(f.stream,{regionTarget:{kind},...(kind==='region'?{regionPlacement:Object.freeze({x:12.5,y:30.25})}:{}),regionLease:{isAlive:()=>current,document_id:'region-doc',sequence:9},regionSource:source,regionWindow:{isCurrent:()=>current},regionCurrent:async()=>current,binding_id:'binding',document_id:'region-doc',css_width:640,css_height:80,device_scale_factor:2});
 const binding=()=>host.displayResumeBinding(f.stream);
 assert.equal(await binding(),source);
 await f.sessions.suspend('old','owner',7,binding);
 const resumed=await f.sessions.resume('old','owner',7,binding);
 assert.equal(resumed.source_kind,'region');assert.equal(resumed.binding_id,'binding');assert.equal(resumed.document_id,'region-doc');assert.equal(resumed.sequence,9);assert.equal(resumed.css_width,640);assert.equal(resumed.css_height,80);
 if(kind==='region')assert.deepEqual(resumed.region_offset,{x:12.5,y:30.25});else assert(!Object.hasOwn(resumed,'region_offset'));
 current=false;await assert.rejects(host.displayResumeBinding(f.stream));
});

test('MP-08/MP-11 cancellation of suspend or resume preserves the browser and retained producer',async()=>{
 for(const op of ['display_suspend','display_resume']){
  const host=Object.create(KernelBrowserHost.prototype);let stopped=0;
  Object.assign(host,{nativeComputer:{held:new Set()},timing:()=>{},protection:{values:[]},request:async()=>{throw Object.assign(Error('cancelled'),{code:'browser_action_cancelled'})},stop:async()=>{stopped++}});
  const reply=await host.handle({id:'resume-cancelled',method:'host.browser',params:{op}});
  assert.equal(reply.ok,false);assert.equal(reply.error.code,'browser_action_cancelled');assert.equal(stopped,0,'viewer cancellation does not stop the source');
 }
});


test('MP-08/MP-10 retired display resume hints are races while foreign owners stay denied', async()=>{
 const f=fixture();
 await assert.rejects(f.sessions.resume('missing','owner',7,()=>f.binding),error=>error.code==='user_domain_stale_reference'&&error.race==='display_retired');
 await assert.rejects(f.sessions.resume('old','owner',6,()=>f.binding),error=>error.code==='user_domain_stale_reference'&&error.race==='display_retired');
 await assert.rejects(f.sessions.resume('old','foreign',7,()=>f.binding),{code:'user_domain_not_granted'});
 assert.equal(f.host.displays.get('old'),f.stream);
});
