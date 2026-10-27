import test from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';
import { BrowserMemory, BrowserMemoryPressureError, requireBrowserMemory } from './browser-memory.mjs';
import { BrowserCdpClient } from './browser-controller-cdp.mjs';
import { handleBrowserControllerRequest } from './browser-controller.mjs';

const MiB = 1024 * 1024;
test('MP-08/MP-10: unknown or exhausted memory refuses allocation; the reserve is usable', () => {
  for (const value of [NaN, -1, 0, 511 * MiB]) assert.throws(() => requireBrowserMemory(value), { code: 'browser_memory_pressure' });
  requireBrowserMemory(512 * MiB);
  requireBrowserMemory(process.availableMemory());
});

test('MP-08/MP-10: cgroup pressure asks Chromium to reclaim once, escalates, and recovers', async () => {
  let free = 700 * MiB;
  const calls = [], connection = { isOpen: () => true, send: async (method, params) => calls.push([method, params.level]) };
  const memory = new BrowserMemory({ availableMemory: () => free });
  memory.watch(connection);
  try {
    await memory.reclaim(); await memory.reclaim();
    assert.deepEqual(calls, [['Memory.simulatePressureNotification', 'moderate']]);
    free = 128 * MiB;
    await assert.rejects(memory.requireHeadroom(), { code: 'browser_memory_pressure' });
    assert.deepEqual(calls[1], ['Memory.simulatePressureNotification', 'critical']);
    free = 2048 * MiB;
    await memory.requireHeadroom(); await memory.reclaim();
    free = 700 * MiB; await memory.reclaim();
    assert.equal(calls.length, 3, 'a new pressure interval can reclaim again');
  } finally { memory.close(); }
});

test('MP-08/MP-11: closing the controller while reclamation is pending stops its watcher', async () => {
  const entered = Promise.withResolvers(), release = Promise.withResolvers();
  let calls = 0;
  const connection = { isOpen: () => true, async send() { calls++; entered.resolve(); await release.promise; } };
  const memory = new BrowserMemory({ availableMemory: () => 700 * MiB, interval: 5 });
  memory.watch(connection);
  await Promise.race([entered.promise, delay(1000).then(() => { throw Error('pressure watcher did not run'); })]);
  memory.close(); release.resolve(); await delay(30);
  assert.equal(calls, 1); assert.equal(memory.connection, null);
});

test('MP-08/MP-10: the real controller returns a definitive memory error before opening CDP', async () => {
  const browser = new BrowserCdpClient({
    memory: new BrowserMemory({ availableMemory: () => 0 }),
    connectionFactory: () => assert.fail('a refused snapshot must not open CDP'),
  });
  try {
    const result = await handleBrowserControllerRequest({ id: 1, method: 'browser.snapshot', params: { target_id: 't', document_id: 'd' } }, { browser });
    assert.equal(result.ok, false); assert.equal(result.error.code, 'browser_memory_pressure');
    assert.equal(result.error.message, new BrowserMemoryPressureError().message);
  } finally { await browser.close(); }
});
