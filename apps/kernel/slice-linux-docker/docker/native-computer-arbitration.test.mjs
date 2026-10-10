// MP-08 / MP-11: fail-first persistent keys cannot cross actor/grant retirement.
import test from 'node:test';import assert from 'node:assert/strict';
import {NativeComputer} from './native-computer.mjs';
const binding={surface_id:'s',generation:'g',width:1280,height:800,environment:{}};
test('MP-11 physical holds exclude other actors and grant retirement releases before acknowledgement',async()=>{
 const events=[];const native=new NativeComputer({placement:'host',binding:()=>binding,execute:async request=>{events.push(request);return {};}});
 const input=(owner,state)=>native.request({op:'input',surface_id:'s',generation:'g',input:{kind:'keycode',keycode:38,state},observed_by:owner},{});
 await input('old-grant:agent:a','down');
 await assert.rejects(input('human:b','down'),/owner/);
 await native.retire('old-grant');
 assert.deepEqual(events.at(-1),{op:'release',codes:[38]});
 await input('human:b','down');await input('human:b','up');
});
test('MP-11 agent repeats are refused; human bounded holds and agent single chords remain available',async()=>{
 const events=[];const native=new NativeComputer({placement:'host',binding:()=>binding,execute:async request=>{events.push(request);return {};}});
 await assert.rejects(native.request({op:'input',surface_id:'s',generation:'g',_agent_input:true,input:{kind:'keycode',keycode:38,state:'down'}},{}),/human/);
 assert.equal(events.length,0);
 await assert.rejects(native.request({op:'input',surface_id:'s',generation:'g',_agent_input:true,input:{kind:'hold',key:'a',duration_ms:50}},{}),error=>error.code==='user_domain_sensitive_requires_focus');
 await native.request({op:'input',surface_id:'s',generation:'g',input:{kind:'hold',key:'a',duration_ms:50}},{});
 await native.request({op:'input',surface_id:'s',generation:'g',_agent_input:true,input:{kind:'key',key:'a'}},{});
 assert.equal(events.length,2);
});
