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
