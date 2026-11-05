import test from 'node:test';
import assert from 'node:assert/strict';
import {captureBackgroundRenderer} from './kernel-browser-background.mjs';

for(const failed of [false,true])test(`MP-08/MP-09/MP-10/MP-11 background capture settles its own session (start failed ${failed})`,async()=>{
  const sent=[],listeners=new Set();
  const connection={subscribe:handler=>{listeners.add(handler);return()=>listeners.delete(handler)},send:async(method,params,session)=>{
    sent.push({method,params,session});
    if(method==='Target.attachToTarget')return {sessionId:'background'};
    if(method==='Page.startScreencast'&&failed)throw Error('target closed');
    return {};
  }};
  if(failed)await assert.rejects(captureBackgroundRenderer(connection,'target'),/target closed/);
  else{
    const close=await captureBackgroundRenderer(connection,'target');
    for(const handler of listeners)handler({method:'Page.screencastFrame',sessionId:'viewer',params:{sessionId:1,data:'private pixels'}});
    assert.ok(!sent.some(c=>c.method==='Page.screencastFrameAck'),'never acknowledge the viewer capture');
    for(const handler of listeners)handler({method:'Page.screencastFrame',sessionId:'background',params:{sessionId:2,data:'private pixels'}});
    await Promise.resolve();
    assert.ok(sent.some(c=>c.method==='Page.screencastFrameAck'&&c.params.sessionId===2));
    assert.ok(!JSON.stringify(sent).includes('private pixels'));
    await close();await close();
  }
  assert.equal(listeners.size,0);
  assert.equal(sent.filter(c=>c.method==='Page.stopScreencast').length,1);
  assert.equal(sent.filter(c=>c.method==='Target.detachFromTarget').length,1);
  assert.ok(!sent.some(c=>c.method==='Target.activateTarget'));
});
