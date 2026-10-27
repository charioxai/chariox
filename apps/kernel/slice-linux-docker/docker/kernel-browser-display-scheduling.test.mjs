// MD-DISPLAY-02/04: input does not wait on capture/encode/pacing settlement.
import test from 'node:test';
import assert from 'node:assert/strict';
import {PassThrough} from 'node:stream';
import {BrowserControllerStdioServer} from './browser-controller.mjs';
import {scheduleHostRequest} from './kernel-browser-host.mjs';
for(const method of ['host.browser','host.computer'])test('MP-08/MP-10/MP-11 '+method+' input progresses while display work is held; shutdown awaits ownership',async()=>{
 const input=new PassThrough(),output=new PassThrough();let release,started;
 const captureStarted=new Promise(r=>started=r),hold=new Promise(r=>release=r);
 const replies=[];let stop;
 const stopped=new Promise(r=>stop=r);
 output.on('data',b=>{for(const line of b.toString().trim().split('\n'))if(line){const reply=JSON.parse(line);replies.push(reply.id);if(reply.id===2)stop()}});
 const server=new BrowserControllerStdioServer({input,output,scheduleRequest:scheduleHostRequest,handleRequest:async request=>{
  if(request.id===1){started();await hold}return {id:request.id,ok:true,result:{}};
 }});
 const running=server.run();
 input.write(JSON.stringify({id:1,method,params:{op:'screenshot',display_subscription_id:'s'}})+'\n');await captureStarted;
 input.write(JSON.stringify({id:2,method,params:{op:'input',tab_id:'t'}})+'\n');
 const early=await Promise.race([stopped.then(()=>true),new Promise(r=>setTimeout(()=>r(false),50))]);
 // Always release ownership before asserting, so RED tests clean up as well.
 input.end();release();await running;assert.equal(early,true);assert.deepEqual(replies,[2,1]);
});

// MP-08/MP-10/MP-11: a resource-heavy mirror read must not park native input.
test('MP-08/MP-10/MP-11 Computer input progresses during held Browser mirror observation',async()=>{
 const input=new PassThrough(),output=new PassThrough();let release,started,completed;
 const began=new Promise(r=>started=r),hold=new Promise(r=>release=r),inputDone=new Promise(r=>completed=r);
 output.on('data',b=>{for(const line of b.toString().trim().split('\n'))if(line&&JSON.parse(line).id===2)completed()});
 const server=new BrowserControllerStdioServer({input,output,scheduleRequest:scheduleHostRequest,handleRequest:async request=>{
  if(request.id===1){started();await hold}return {id:request.id,ok:true,result:{}};
 }});
 const running=server.run();
 input.write(JSON.stringify({id:1,method:'host.browser',params:{op:'mirror_next',subscription_id:'mirror',generation:1}})+'\n');await began;
 input.write(JSON.stringify({id:2,method:'host.computer',params:{op:'input',surface_id:'s',generation:'g'}})+'\n');
 const early=await Promise.race([inputDone.then(()=>true),new Promise(r=>setTimeout(()=>r(false),50))]);
 input.end();release();await running;assert.equal(early,true,'mirror observation blocked native input');
});


test('MP-08/MP-10/MP-11 an owned visible-page startup never parks native input',async()=>{
 const input=new PassThrough(),output=new PassThrough();let release,started,completed;
 const began=new Promise(r=>started=r),hold=new Promise(r=>release=r),inputDone=new Promise(r=>completed=r);
 const browser={canReconcileConcurrently:()=>true};
 output.on('data',b=>{for(const line of b.toString().trim().split('\n'))if(line&&JSON.parse(line).id===2)completed()});
 const server=new BrowserControllerStdioServer({input,output,scheduleRequest:request=>scheduleHostRequest(request,browser,{},true),handleRequest:async request=>{
  if(request.id===1){started();await hold}return {id:request.id,ok:true,result:{}};
 }});
 const running=server.run();
 input.write(JSON.stringify({id:1,method:'host.browser',params:{op:'display_subscribe',tab_id:'t',generation:1}})+'\n');await began;
 input.write(JSON.stringify({id:2,method:'host.browser',params:{op:'input',tab_id:'t'}})+'\n');
 const early=await Promise.race([inputDone.then(()=>true),new Promise(r=>setTimeout(()=>r(false),100))]);
 input.end();release();await running;assert.equal(early,true,'input waited behind the first protected native sample');
});
test('MP-08/MP-10/MP-11 legacy resize and unapplied viewport startup retain their barrier',()=>{
 const request={method:'host.browser',params:{op:'display_subscribe',tab_id:'t'}};
 assert.deepEqual(scheduleHostRequest(request,{canReconcileConcurrently:()=>true},{},false),{kind:'barrier'});
 assert.deepEqual(scheduleHostRequest(request,{canReconcileConcurrently:()=>false},{},true),{kind:'barrier'});
});

for(const method of ['host.browser.retire_input','host.browser.retire_observer'])test('MP-08/MP-10/MP-11 '+method+' is not queued behind a running browser operation',async()=>{
 const input=new PassThrough(),output=new PassThrough();let release,started,completed;
 const began=new Promise(r=>started=r),hold=new Promise(r=>release=r),retired=new Promise(r=>completed=r);
 output.on('data',b=>{for(const line of b.toString().trim().split('\n'))if(line&&JSON.parse(line).id===2)completed()});
 const server=new BrowserControllerStdioServer({input,output,scheduleRequest:scheduleHostRequest,handleRequest:async request=>{
  if(request.id===1){started();await hold}return {id:request.id,ok:true,result:{}};
 }});
 const running=server.run();
 input.write(JSON.stringify({id:1,method:'host.browser',params:{op:'navigate',tab_id:'t'}})+'\n');await began;
 input.write(JSON.stringify({id:2,method,params:{observer:'terminal:a',tab_id:'t',except:'terminal:b',epoch:1}})+'\n');
 const early=await Promise.race([retired.then(()=>true),new Promise(r=>setTimeout(()=>r(false),100))]);
 input.end();release();await running;assert.equal(early,true,'the retirement waited behind the running operation');
});
