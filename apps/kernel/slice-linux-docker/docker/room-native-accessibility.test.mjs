// MP-08 / MP-10 / MP-11: same native observer behind the Room controller.
import test from 'node:test';
import assert from 'node:assert/strict';
import {handleBrowserControllerRequest} from './browser-controller.mjs';
test('MP-08 Room snapshots dispatch through the live controller, retaining observer scope', async()=>{
 const calls=[];const browser={roomComputer:{request:async(params)=>{calls.push(params);return {tree_revision:1,nodes:[],fallback:'ocr'};}}};
 const result=await handleBrowserControllerRequest({id:1,method:'computer.snapshot',params:{observer:'agent:a',policy:{unknown:false,values:[],targets:[]}}},{browser});
 assert.equal(result.ok,true);assert.equal(result.result.tree_revision,1);assert.equal(calls[0].observer,'agent:a');
});
test('MP-11 Room target actions dispatch under the existing controller mutation barrier', async()=>{
 const browser={roomComputer:{request:async(params)=>({applied:params.action==='click'})}};
 const result=await handleBrowserControllerRequest({id:2,method:'computer.target_action',params:{observer:'agent:a',action:'click'}},{browser});
 assert.equal(result.ok,true);assert.equal(result.result.applied,true);
});
import {RoomNativeAccessibility} from './room-native-accessibility.mjs';
test('MP-11 native Room handles refuse cross-observer reuse, stale trees and protection changes',async()=>{
 const binding={surface_id:'slice:s',generation:'epoch',environment:{},ownedProcesses:async()=>[{pid:42,started:'1'}]};
 let name='File';let applied=0;
 const native=new RoomNativeAccessibility({binding:()=>binding,execute:async request=>{
   if(request.op==='accessibility')return {available:true,complete:true,nodes:[{pid:42,started:'1',path:[0],role:'menu item',name,states:['showing'],actions:['click']}]};
   applied++;return {applied:true};
 }});
 const policy={unknown:false,values:[],targets:[]};
 const snapshot=await native.request({op:'snapshot',observer:'agent:a',policy});
 const action={op:'target_action',observer:'agent:a',policy,tree_revision:snapshot.tree_revision,target_id:snapshot.nodes[0].target_id,action:'click'};
 await assert.rejects(native.request({...action,observer:'agent:b'}),e=>e.code==='user_domain_stale_reference');
 name='Edit';await assert.rejects(native.request(action),e=>e.code==='user_domain_stale_reference');
 const fresh=await native.request({op:'snapshot',observer:'agent:a',policy});
 const current={...action,tree_revision:fresh.tree_revision,target_id:fresh.nodes[0].target_id};
 await assert.rejects(native.request({...current,policy:{...policy,targets:[{kind:'native'}]}}),e=>e.code==='user_domain_not_granted');
 assert.equal(applied,0);assert.equal((await native.request(current)).applied,true);
 await assert.rejects(native.request(current),e=>e.code==='user_domain_stale_reference');
 assert.equal(applied,1);
});
import {PassThrough} from 'node:stream';
import {BrowserControllerStdioServer} from './browser-controller.mjs';
test('MP-11 cancel queued/running native target actions through the existing controller cancellation',async()=>{
 const input=new PassThrough(),output=new PassThrough();let text='';output.on('data',data=>text+=data);
 const started=Promise.withResolvers();let applied=0;
 const browser={roomComputer:{request:async(params,{signal})=>{
   started.resolve();await new Promise(resolve=>signal.addEventListener('abort',resolve,{once:true}));
   assert.equal(signal.aborted,true);return {applied:false};
 }},close:async()=>{}};
 const server=new BrowserControllerStdioServer({input,output,browser});const run=server.run();
 input.write(JSON.stringify({id:1,method:'computer.target_action',params:{observer:'agent:a'}})+'\n');
 await Promise.race([started.promise,new Promise((_,reject)=>setTimeout(()=>reject(Error('native action did not start')),1000))]);
 input.write(JSON.stringify({id:2,method:'browser.cancel',params:{request_id:1}})+'\n');
 const end=Date.now()+1000;while(!text.includes('"accepted":true')&&Date.now()<end)await new Promise(resolve=>setTimeout(resolve,10));
 assert.match(text,/"accepted":true/);input.end();await run;assert.equal(applied,0);
});
