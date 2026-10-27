// MP-08/MP-10/MP-11: the kernel starts the viewer's native navigation display when its mirror reports the navigation.
import test from 'node:test';import assert from 'node:assert/strict';
import {NavigationDisplays} from './kernel-browser-navigation-display.mjs';
import {Mirror2} from './kernel-browser-mirror2.mjs';
const offer={op:'display_subscribe',tab_id:'t',generation:1,codecs:['avc1.420033','png','chariox-relay-binary-v96'],bitrate:8_000_000,device_scale_factor:2};
function fixture({document='d2'}={}){
 const removed=[],subscribed=[],tab={tab_id:'t',document_id:document};
 const host={generation:1,displays:new Map(),timing(){},mirrorTarget:async command=>{assert.equal(command.tab_id,'t');return {...tab}},
  removeDisplay:async stream=>{removed.push(stream.subscription_id);host.displays.delete(stream.subscription_id)}};
 let resolve;
 const subscribe=(h,command,scope)=>{assert.equal(h,host);subscribed.push([command,scope]);const id=`host-display-${subscribed.length}`;
  return new Promise(r=>{resolve=()=>{host.displays.set(id,{subscription_id:id,regionLease:{document_id:'d2',isAlive:()=>true}});r({subscription_id:id,generation:1,codec:'avc1.420033'})}})};
 const displays=new NavigationDisplays(host,{subscribe,unclaimedMs:50});
 return {host,displays,removed,subscribed,tab,finish:()=>resolve()};
}
test('MP-08/MP-10 the next display_subscribe of the observer adopts the display started at the navigation report',async()=>{
 const {displays,subscribed,finish}=fixture();
 displays.start('scope','t','d2');assert.equal(subscribed.length,0,'no negotiation known yet: nothing is started');
 displays.remember(offer,'scope');displays.start('scope','t','d2');
 assert.equal(subscribed.length,1);assert.deepEqual(subscribed[0],[{...offer},'scope'],'the same negotiation, for the same observer');
 const adopted=displays.adopt(offer,'scope');finish();
 assert.equal((await adopted).subscription_id,'host-display-1');
 assert.equal(await displays.adopt(offer,'scope'),null,'adopted once');
});
test('MP-08/MP-11 a started display is retired when the request differs, the document moved on, or nobody claims it',async()=>{
 for(const [name,command,scope,document] of [['negotiation',{...offer,bitrate:4_000_000},'scope','d2'],['observer',offer,'other','d2'],['document',offer,'scope','d3'],['generation',{...offer,generation:2},'scope','d2']]){
  const {displays,removed,finish}=fixture({document});
  displays.remember(offer,'scope');displays.start('scope','t','d2');finish();
  assert.equal(await displays.adopt(command,scope),null,name);
  if(scope==='scope')assert.deepEqual(removed,['host-display-1'],name);
 }
 const {displays,removed,finish}=fixture();
 displays.remember(offer,'scope');displays.start('scope','t','d2');finish();
 await new Promise(r=>setTimeout(r,80));
 assert.deepEqual(removed,['host-display-1'],'unclaimed past its bound');
 assert.equal(await displays.adopt(offer,'scope'),null);
});
test('MP-08/MP-10 a newer navigation report retires the display of the older one',async()=>{
 const {displays,removed,subscribed,finish}=fixture();
 displays.remember(offer,'scope');displays.start('scope','t','d1');finish();displays.start('scope','t','d2');finish();
 await new Promise(r=>setImmediate(r));
 assert.equal(subscribed.length,2);assert.deepEqual(removed,['host-display-1']);
});
test('MP-08/MP-10 a mirror navigation preview starts the observer display before the packet leaves',()=>{
 const started=[];
 const mirror=new Mirror2({host:{generation:1,pageSize:()=>({width:1440,height:900}),scales:new Map(),navigationDisplays:{start:(...args)=>started.push(args)}}});
 const stream={issued:3};mirror.service={require:()=>stream};mirror.remember=()=>{};
 const packet=mirror.previewPacket(stream,{subscription_id:'m',generation:1},'scope','t','d2',0);
 assert.equal(packet.fallback,'navigation_preview');assert.deepEqual(started,[['scope','t','d2']]);
});
