import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { createInputCallers } from './browser-app-input-callers.mjs';

test('input callers match only the stamped trusted event and retire after handlers settle', async () => {
  let capture;
  const inputs = vm.runInNewContext(`(${createInputCallers.toString()})()`, {
    performance:{timeOrigin:100000,now:()=>50},addEventListener:(_type,listener)=>{ capture=listener; },
  });
  const stamped = {isTrusted:true,timeStamp:86300000};
  const human = {isTrusted:true,timeStamp:100};
  const forged = {isTrusted:false,timeStamp:86300000};
  inputs.begin('caller', 86400);
  for(const event of [stamped,human,forged]) capture(event);
  assert.equal(inputs.token(stamped), 'caller');
  assert.equal(stamped.timeStamp,50,'App listeners see the actual dispatch clock');
  assert.equal(inputs.token(human), undefined);
  assert.equal(inputs.token(forged), undefined);
  let release, settled = false;
  inputs.track('caller', new Promise(resolve => { release = resolve; }));
  const ending = inputs.end('caller').then(() => { settled = true; });
  await Promise.resolve();
  assert.equal(settled,false);
  assert.equal(inputs.token(stamped),undefined);
  release(); await ending;
  assert.equal(settled,true);
  await inputs.end('caller');
});

test('controller submission tags only the exact synchronous form event', async () => {
  const listeners=new Map();let seen;
  function Form() {}
  Form.prototype.requestSubmit=function(){const event={target:this};listeners.get(this)?.(event);seen=inputs.token(event);};
  const inputs=vm.runInNewContext(`(${createInputCallers.toString()})()`, {performance:{timeOrigin:0,now:()=>1},addEventListener(){},HTMLFormElement:Form});
  const form={addEventListener(_type,fn){listeners.set(this,fn);},removeEventListener(){listeners.delete(this);}};
  inputs.begin('caller',10);inputs.submit(form,'caller');
  assert.equal(seen,'caller');assert.equal(listeners.size,0);
  assert.equal(inputs.token({target:form}),undefined);
  await inputs.end('caller');assert.throws(()=>inputs.submit(form,'caller'),/expired/);
});
