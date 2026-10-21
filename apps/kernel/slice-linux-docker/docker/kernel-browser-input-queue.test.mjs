import assert from 'node:assert/strict';
import test from 'node:test';
import { BusyInputQueue } from './kernel-browser-input-queue.mjs';

const busy = () => Object.assign(new Error('browser CDP method Page.getFrameTree timed out after 5000ms'), { code: 'browser_cdp_timeout' });

// r55: a page input parked behind a busy renderer is the head of its own lane. The person's address-bar keys need no
// renderer (they are admitted from the kernel's own record), so they must not wait behind it (r54: typed URL stalled ~38 s).
test('MP-08/MP-10/MP-11 a parked page input does not hold the same observer\'s band input behind it', async () => {
 const ran = []; let renderer = false;
 const run = async command => { if (command.lane === 'page' && !renderer) throw busy(); ran.push(command.id); return { ok: true }; };
 const queue = new BusyInputQueue(run, () => ({ parked: true }), { retryMs: 5, lane: command => command.lane });
 assert.deepEqual(await queue.submit('terminal:a', { id: 'wheel', lane: 'page' }), { parked: true });
 assert.deepEqual(await queue.submit('terminal:a', { id: 'key-1', lane: 'band' }), { ok: true }, 'the band key runs at once');
 assert.deepEqual(await queue.submit('terminal:a', { id: 'key-2', lane: 'band' }), { ok: true });
 assert.deepEqual(ran, ['key-1', 'key-2']);
 assert.deepEqual(await queue.submit('terminal:a', { id: 'wheel-2', lane: 'page' }), { parked: true }, 'later page input still queues in order behind the page head');
 assert.equal(queue.parked('terminal:a'), 2);
 assert.equal(queue.parked('terminal:b'), 0);
 renderer = true;
 for (let n = 0; n < 100 && ran.length < 4; n++) await new Promise(resolve => setTimeout(resolve, 5));
 assert.deepEqual(ran, ['key-1', 'key-2', 'wheel', 'wheel-2'], 'the page lane drains in order once the renderer answers');
 assert.equal(queue.parked('terminal:a'), 0);
});

test('MP-08/MP-10/MP-11 a retirement drops the parked input of every lane of the observer', async () => {
 const run = async () => { throw busy(); };
 const queue = new BusyInputQueue(run, () => ({ parked: true }), { retryMs: 5, lane: command => command.lane });
 await queue.submit('terminal:a', { id: 'w', lane: 'page' }); await queue.submit('terminal:a', { id: 'k', lane: 'band' }); await queue.submit('terminal:b', { id: 'w', lane: 'page' });
 assert.equal(queue.parked('terminal:a'), 2);
 queue.retire(key => key === 'terminal:a');
 assert.equal(queue.parked('terminal:a'), 0); assert.equal(queue.parked('terminal:b'), 1);
 queue.stop();
});

// Review 1037 P1: the first attempt is not in a queue yet; a retirement or cancel that lands while its fence waits must still win.
test('MP-08/MP-10/MP-11 a retirement or cancel during the first busy attempt keeps the input from parking and replaying', async () => {
 for (const cancel of [false, true]) {
  const ran = []; let release; const gate = new Promise(resolve => { release = resolve; });
  const run = async (command, options) => { ran.push(command.id); if (ran.length === 1) { await gate; throw busy(); } return { ok: true }; };
  const queue = new BusyInputQueue(run, () => ({ parked: true }), { retryMs: 5 });
  const controller = new AbortController();
  const submitted = queue.submit('terminal:a', { id: 'k' }, { signal: controller.signal });
  if (cancel) controller.abort(); else queue.retire(key => key === 'terminal:a');
  release();
  await assert.rejects(submitted, { code: 'browser_cdp_timeout' });
  await new Promise(resolve => setTimeout(resolve, 30));
  assert.deepEqual(ran, ['k'], 'the renderer answering later dispatches nothing');
  assert.equal(queue.queues.size, 0);
 }
});

test('MP-08/MP-10/MP-11 a retirement of another observer does not stop a first attempt from parking', async () => {
 const ran = []; let release; const gate = new Promise(resolve => { release = resolve; });
 const run = async command => { ran.push(command.id); if (ran.length === 1) { await gate; throw busy(); } return { ok: true }; };
 const queue = new BusyInputQueue(run, () => ({ parked: true }), { retryMs: 5 });
 const submitted = queue.submit('terminal:a', { id: 'k' });
 queue.retire(key => key === 'terminal:b'); release();
 assert.deepEqual(await submitted, { parked: true });
 for (let n = 0; n < 100 && ran.length < 2; n++) await new Promise(resolve => setTimeout(resolve, 5));
 assert.deepEqual(ran, ['k', 'k']);
});

// Review 1037 P2: stop() ends every wait, including a retry that has not started yet.
test('MP-08/MP-10/MP-11 retireAll drops parked and running retries and the queue works again afterwards', async () => {
 const ran = []; let busyNow = true;
 const run = async command => { ran.push(command.id); if (busyNow) throw busy(); return { ok: true }; };
 const queue = new BusyInputQueue(run, () => ({ parked: true }), { retryMs: 20 });
 await queue.submit('terminal:a', { id: 'a' }); await queue.submit('terminal:b', { id: 'b' });
 queue.retireAll(); busyNow = false;
 await new Promise(resolve => setTimeout(resolve, 80));
 assert.deepEqual(ran, ['a', 'b'], 'no retry runs after retireAll');
 assert.equal(queue.queues.size, 0);
 assert.deepEqual(await queue.submit('terminal:a', { id: 'later' }), { ok: true });
});

// r67: a wheel that timed out against a renderer busy with a 3 s snapshot parks in its own lane; the first key after it
// needs no renderer read (its focus is proven) and is admitted at once, not behind the wheel's retries.
test('MP-08/MP-10 r67 the first key after a parked wheel runs at once', async () => {
 const ran = []; let renderer = false;
 const run = async command => { if (command.input.kind === 'scroll' && !renderer) throw busy(); ran.push(command.input.kind); return { ok: true }; };
 const lane = command => command.input.kind === 'scroll' ? 'wheel' : 'page';
 const queue = new BusyInputQueue(run, () => ({ parked: true }), { retryMs: 5, lane });
 assert.deepEqual(await queue.submit('terminal:a', { input: { kind: 'scroll' } }), { parked: true });
 const at = performance.now();
 assert.deepEqual(await queue.submit('terminal:a', { input: { kind: 'key' } }), { ok: true });
 assert.ok(performance.now() - at < 150, 'the key does not wait for the wheel');
 assert.deepEqual(ran, ['key']);
 renderer = true;
 await new Promise(resolve => setTimeout(resolve, 60));
 assert.deepEqual(ran, ['key', 'scroll'], 'the wheel still lands once the renderer answers');
 queue.stop();
});
