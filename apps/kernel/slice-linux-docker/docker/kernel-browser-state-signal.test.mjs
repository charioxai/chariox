// MP-08/MP-10 (protocol 556): the host wakes a parked state read on change and bounds it otherwise.
import test from 'node:test';
import assert from 'node:assert/strict';
import {StateSignal,stateChangingEvent} from './kernel-browser-state-signal.mjs';
test('MP-08/MP-10 a parked state read wakes on a signal, at its deadline or when cancelled',async()=>{
 const signal=new StateSignal();
 assert.equal(await signal.wait(null,5000),0,'the first read does not wait');
 const woke=signal.wait(0,5000);let done=false;woke.then(()=>done=true);
 await new Promise(r=>setTimeout(r,20));assert.equal(done,false,'still parked');
 signal.notify();assert.equal(await woke,1);assert.equal(signal.waiters.size,0);
 assert.equal(await signal.wait(0,5000),1,'a signal before the read arrived is not lost');
 const started=performance.now();assert.equal(await signal.wait(1,60),1);assert(performance.now()-started>=50,'idle waits last the deadline');
 const abort=new AbortController(),cancelled=signal.wait(1,5000,abort.signal);abort.abort();assert.equal(await cancelled,1);assert.equal(signal.waiters.size,0);
});
test('MP-08/MP-10 only page Tab and top-level navigation events wake a state read',()=>{
 const page={targetInfo:{type:'page'}},worker={targetInfo:{type:'service_worker'}};
 for(const message of [{method:'Target.targetCreated',params:page},{method:'Target.targetInfoChanged',params:page},{method:'Target.targetDestroyed',params:{targetId:'t'}},
  {method:'Page.frameNavigated',params:{frame:{id:'f'}}},{method:'Page.javascriptDialogOpening'},{method:'Page.javascriptDialogClosed'}, {method:'Target.detachedFromTarget',params:{}},{method:'Inspector.detached'},{method:'Chariox.browserDisconnected'}])
  assert.equal(stateChangingEvent(message),true,message.method);
 for(const message of [{method:'Target.targetInfoChanged',params:worker},{method:'Target.targetCreated',params:{targetInfo:{type:'iframe'}}},{method:'Page.frameNavigated',params:{frame:{id:'f',parentId:'p'}}},
  {method:'Page.frameStartedLoading',params:{}},{method:'Page.frameStoppedLoading',params:{}},{method:'Page.loadEventFired',params:{}},
  {method:'Page.screencastFrame'},{method:'Network.requestWillBeSent'},{method:'Runtime.consoleAPICalled'},{method:'DOM.childNodeInserted'},{method:'Page.frameResized'}])
  assert.equal(stateChangingEvent(message),false,message.method);
});
