import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createFeed, CHANGE_LIMIT, CHANGE_BYTES } from '../src/feed.js';

// The kernel's state contract: values are JSON text, versions bump per write, a stale check conflicts.
function kernel() {
  const records = new Map();
  return {
    records,
    get: async key => records.get(key) ?? null,
    transact: async ({ checks, writes }) => {
      for (const check of checks) {
        if ((records.get(check.key)?.version ?? null) !== check.version) throw { code: 'CONFLICT', message: check.key };
      }
      for (const write of writes) {
        if (write.tag === 'put') records.set(write.val[0], { value: write.val[1], version: (records.get(write.val[0])?.version ?? 0n) + 1n });
        else records.delete(write.val);
      }
      return { revision: 1n, receipts: [] };
    },
  };
}
const tx = (key, value) => ({ schemaVersion: 0, checks: [], writes: [{ tag: 'put', val: [key, value] }], occurrences: [], wakes: [] });

test('a change commits with the App writes and a reader sees it by sequence', async () => {
  const state = kernel();
  const feed = createFeed(state);
  assert.deepEqual(await feed.changes({}), { latest: 0, dropped: false, changes: [] });
  await feed.transact(tx('a', '1'), { op: 'one' });
  await feed.transact(tx('b', '2'), { op: 'two' });
  assert.equal(state.records.get('a').value, '1');
  assert.deepEqual(await feed.changes({ since: 0 }), { latest: 2, dropped: false, changes: [{ seq: 1, delta: { op: 'one' } }, { seq: 2, delta: { op: 'two' } }] });
  assert.deepEqual((await feed.changes({ since: 1 })).changes, [{ seq: 2, delta: { op: 'two' } }]);
  assert.deepEqual(await feed.changes({ since: 2 }), { latest: 2, dropped: false, changes: [] });
});

test('the feed keeps the last changes and tells an older reader to read the state again', async () => {
  const state = kernel();
  const feed = createFeed(state);
  for (let i = 1; i <= CHANGE_LIMIT + 2; i++) await feed.transact(tx('k', String(i)), { i });
  const answer = await feed.changes({ since: 0 });
  assert.equal(answer.latest, CHANGE_LIMIT + 2);
  assert.equal(answer.dropped, true);
  assert.equal(answer.changes.length, CHANGE_LIMIT);
  assert.equal(answer.changes[0].seq, 3);
  assert.equal((await feed.changes({ since: 2 })).dropped, false);
  assert.equal(state.records.has('chariox.view.change.2'), false, 'only the last changes are stored');
});

test('a change too large for the feed is refused and nothing is written', async () => {
  const state = kernel();
  const feed = createFeed(state);
  await assert.rejects(feed.transact(tx('a', '1'), { text: 'x'.repeat(CHANGE_BYTES) }), { code: 'CHANGE_TOO_LARGE' });
  assert.equal(state.records.size, 0);
});

test('a conflicting writer leaves the feed head where it was', async () => {
  const state = kernel();
  const feed = createFeed(state);
  const stale = { ...tx('a', '1'), checks: [{ key: 'a', version: 7n }] };
  await assert.rejects(feed.transact(stale, {}), { code: 'CONFLICT' });
  assert.equal((await feed.changes({})).latest, 0);
});
