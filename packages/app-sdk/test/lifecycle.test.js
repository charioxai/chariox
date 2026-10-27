import test from 'node:test';
import assert from 'node:assert/strict';
import { createAppSdk } from '../src/index.js';
import { deferred, fakeTransport, flush, generation, request } from './helpers.js';

const events = ['startup', 'suspend', 'resume', 'configuration_change', 'prepare_update', 'shutdown'];
const paths = { package: '/isolated/package', data: '/isolated/data', temporary: '/isolated/tmp' };

// SDK dispatch tests; kernel notification triggers need their own integration coverage.
test('each lifecycle notification forwards data and completes before the next callback', async () => {
  const transport = fakeTransport();
  const sdk = createAppSdk({ transport, generation, paths });
  const calls = [];
  for (const event of events) sdk.lifecycle.on(event, (data, context) => {
    assert.equal(context.signal.aborted, false);
    calls.push(event);
    return { event, data };
  });
  try {
    for (const event of events) {
      const data = { revision: calls.length + 1 };
      transport.receive(request(event, 'lifecycle.dispatch', { event, data }));
      await flush();
      assert.deepEqual(transport.sent.at(-1).result, { event, data });
    }
    assert.deepEqual(calls, events);
  } finally { sdk.close(); }
});

for (const event of events) test(`${event} deadline retains lifecycle exclusion until the handler settles`, { timeout: 5000 }, async () => {
  const transport = fakeTransport();
  const sdk = createAppSdk({ transport, generation, paths });
  const busy = deferred();
  const aborted = deferred();
  sdk.lifecycle.on(event, (_data, context) => {
    context.signal.addEventListener('abort', aborted.resolve, { once: true });
    return busy.promise;
  });
  let healthCalls = 0;
  sdk.lifecycle.on('health_check', () => { healthCalls++; return null; });
  try {
    transport.receive(request('deadline', 'lifecycle.dispatch', { event }, { deadline_ms: Date.now() + 200 }));
    await aborted.promise;
    assert.equal(transport.sent.find(item => item.id === 'deadline').error.code, 'DEADLINE_EXCEEDED');
    transport.receive(request('overlap', 'lifecycle.dispatch', { event: 'health_check' }));
    await flush();
    assert.equal(transport.sent.find(item => item.id === 'overlap').error.code, 'BUSY');
    assert.equal(healthCalls, 0);
    busy.resolve('late');
    await flush();
    assert.equal(transport.sent.filter(item => item.id === 'deadline').length, 1);
    transport.receive(request('after', 'lifecycle.dispatch', { event: 'health_check' }));
    await flush();
    assert.equal(transport.sent.find(item => item.id === 'after').result, null);
    assert.equal(healthCalls, 1);
    assert.equal(transport.closed, false);
  } finally { busy.resolve(null); sdk.close(); }
});
