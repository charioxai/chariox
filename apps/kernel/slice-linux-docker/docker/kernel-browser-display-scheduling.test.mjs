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
