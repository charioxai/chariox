import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { createInputCallers } from './browser-app-input-callers.mjs';

test('input callers match only the stamped trusted event and retire after handlers settle', async () => {
  const inputs = vm.runInNewContext(`(${createInputCallers.toString()})()`, {performance:{timeOrigin:100000}});
  inputs.begin('caller', 86400);
  assert.equal(inputs.token({isTrusted:true,timeStamp:86300000}), 'caller');
  assert.equal(inputs.token({isTrusted:true,timeStamp:100}), undefined);
  assert.equal(inputs.token({isTrusted:false,timeStamp:86300000}), undefined);
  let release, settled = false;
  inputs.track('caller', new Promise(resolve => { release = resolve; }));
  const ending = inputs.end('caller').then(() => { settled = true; });
  await Promise.resolve();
  assert.equal(settled,false);
  assert.equal(inputs.token({isTrusted:true,timeStamp:86300000}),undefined);
  release(); await ending;
  assert.equal(settled,true);
  await inputs.end('caller');
});
