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
