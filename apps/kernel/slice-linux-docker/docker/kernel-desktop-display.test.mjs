// MP-08 / MP-10 / MP-11: desktop viewer authority/lifetime regressions.
import test from 'node:test';
import assert from 'node:assert/strict';
import { DesktopDisplay } from './kernel-desktop-display.mjs';
const target={surface_id:'desktop-one',generation:'native-one',width:1280,height:800};
function fixture(){
 let captures=0,closed=0;
 const source={closed:false,subscribe:()=>()=>{},sample:()=>null,start:async()=>{},close:async()=>{closed++;source.closed=true},wake:()=>{},valid:()=>!source.closed};
 const host={generation:1,protection:{values:[],targets:[],unknown:false},displays:new Map(),chromium:{desktop:{binding:()=>target}},armDisplayExpiry:()=>{},timing:()=>{}};
 const display=new DesktopDisplay(host,{createSource:async()=>{captures++;return source}});
 return {host,display,counts:()=>({captures,closed})};
}
const command={op:'display_subscribe',...target,codecs:['avc1.420033'],bitrate:8000000,device_scale_factor:1};
test('MP-11 stale desktop, agent video and unsupported codec fail before capture',async()=>{
 const {display,counts}=fixture();
 for(const request of [{...command,generation:'old'}, {...command,_agent_input:true}, {...command,codecs:['png']}, {...command,device_scale_factor:3}])await assert.rejects(display.subscribe(request,'terminal'));
 assert.equal(counts().captures,0);
});
test('MP-11 two desktop viewers share one source and last close releases it',async()=>{
 const {display,host,counts}=fixture();
 const first=await display.subscribe(command,'a'),second=await display.subscribe(command,'b');
 assert.equal(counts().captures,1);
 await assert.rejects(display.request({op:'display_attach',subscription_id:first.subscription_id,generation:1},'b'));
 await display.request({op:'unsubscribe',subscription_id:first.subscription_id,generation:1},'a');
 assert.equal(counts().closed,0);assert.equal(host.displays.size,1);
 await display.request({op:'unsubscribe',subscription_id:second.subscription_id,generation:1},'b');
 assert.equal(counts().closed,1);assert.equal(host.displays.size,0);
});
test('MP-11 protection retirement closes producers before returning and blocks old credits',async()=>{
 const {display,host,counts}=fixture();const subscribed=await display.subscribe(command,'a');
 await display.close();assert.equal(counts().closed,1);assert.equal(host.displays.size,0);
 await assert.rejects(display.request({op:'screenshot',display_subscription_id:subscribed.subscription_id,generation:1,after_sequence:0},'a'));
});

test('MP-11 simultaneous viewers await one protected source',async()=>{
 const {host}=fixture();let captures=0,release;
 const started=new Promise(resolve=>{release=resolve});
 const source={closed:false,close:async()=>{},valid:()=>true};
 const display=new DesktopDisplay(host,{createSource:async()=>{captures++;await started;return source}});
 const first=display.subscribe(command,'a'),second=display.subscribe(command,'b');
 await new Promise(resolve=>setImmediate(resolve));assert.equal(captures,1);
 release();await Promise.all([first,second]);await display.close();
});
test('MP-11 canceling one credit preserves producer lifetime, retiring policy closes it',async()=>{
 const {host}=fixture();let valid,closed=0;
 const source={closed:false,close:async()=>{source.closed=true},valid:()=>!source.closed};
 const display=new DesktopDisplay(host,{createSource:async()=>source,createProducer:(_source,_encoder,options)=>{
  valid=options.valid;return {waitReady:async()=>{},take:()=>null,close:async()=>{closed++}};
 }});
 const subscription=await display.subscribe(command,'a'),controller=new AbortController();
 const request={op:'screenshot',display_subscription_id:subscription.subscription_id,generation:1,after_sequence:0};
 await display.request(request,'a',{signal:controller.signal});controller.abort();assert.equal(valid(),true);
 await display.request(request,'a');assert.equal(valid(),true);
 await display.retireSource();assert.equal(closed,1);assert.equal(valid(),false);
 await display.close();
});
