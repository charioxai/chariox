import test from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';
import * as browserMemory from './browser-memory.mjs';
import { BrowserMemory, BrowserMemoryPressureError, requireBrowserMemory } from './browser-memory.mjs';
import { BrowserCdpClient } from './browser-controller-cdp.mjs';
import { handleBrowserControllerRequest } from './browser-controller.mjs';

const MiB = 1024 * 1024;
test('MP-08/MP-10: unknown or exhausted memory refuses allocation; the reserve is usable', () => {
  for (const value of [NaN, -1, 0, 511 * MiB]) assert.throws(() => requireBrowserMemory(value), { code: 'browser_memory_pressure' });
  requireBrowserMemory(512 * MiB);
  requireBrowserMemory(process.availableMemory());
});

const vmStat = ({ free, inactive, speculative, purgeable }) => `Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                               ${free}.
Pages active:                           400000.
Pages inactive:                           ${inactive}.
Pages speculative:                        ${speculative}.
Pages throttled:                               0.
Pages wired down:                         200000.
Pages purgeable:                          ${purgeable}.
`;

test('MP-08/MP-10: macOS counts reclaimable pages, so a normally loaded Mac admits the browser', () => {
  // 2026-10-30 Mac: 3653 strictly free pages (57 MiB), 39 % free, pressure level 1.
  const normal = vmStat({ free: 3653, inactive: 190000, speculative: 20000, purgeable: 15000 });
  assert.equal(browserMemory.availableMemory({ platform: 'darwin', vmStat: () => normal }), (3653 + 190000 + 20000 + 15000) * 16384);
  requireBrowserMemory(browserMemory.availableMemory({ platform: 'darwin', vmStat: () => normal }));
  const memory = new BrowserMemory({ availableMemory: () => browserMemory.availableMemory({ platform: 'darwin', vmStat: () => normal }) });
  return memory.requireHeadroom();
});

test('MP-08/MP-10: macOS under real pressure still refuses with the typed error', () => {
  const pressed = vmStat({ free: 800, inactive: 4000, speculative: 100, purgeable: 50 });
  const available = browserMemory.availableMemory({ platform: 'darwin', vmStat: () => pressed });
  assert.throws(() => requireBrowserMemory(available), { code: 'browser_memory_pressure' });
});

test('MP-08/MP-10: an unreadable vm_stat fails with its own reason, not a pressure refusal', () => {
  assert.throws(() => browserMemory.availableMemory({ platform: 'darwin', vmStat: () => 'garbage' }), /vm_stat did not report a page size/);
  assert.throws(() => browserMemory.availableMemory({ platform: 'darwin', vmStat: () => 'page size of 16384 bytes\nPages free: 1.' }), /Pages inactive/);
});

test('MP-08/MP-10: other platforms keep Node availableMemory', () => {
  assert.equal(browserMemory.availableMemory({ platform: 'linux', vmStat: () => assert.fail('no vm_stat on linux') }), process.availableMemory());
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
