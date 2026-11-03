import assert from 'node:assert/strict';
import test from 'node:test';
import { untilOwnerAnswers, OWNER_ASKS } from '../view/owner-wait.mjs';

const late = () => Object.assign(new Error('The App did not answer in time'), { code: 'DEADLINE_EXCEEDED' });

test('asking again after the call deadline waits for the owner', async () => {
  let asks = 0;
  const grants = await untilOwnerAnswers(async () => { if (++asks < 3) throw late(); return { grants: ['grant-1'] }; });
  assert.deepEqual([asks, grants], [3, { grants: ['grant-1'] }]);
});

test('a declined request or another failure ends the wait at once', async () => {
  for (const code of ['DECLINED', 'EXPIRED', 'APP_VIEW_STALE']) {
    let asks = 0;
    await assert.rejects(untilOwnerAnswers(async () => { asks += 1; throw Object.assign(new Error(code), { code }); }), { code });
    assert.equal(asks, 1);
  }
});

test('the wait is bounded by the kernel request lifetime', async () => {
  let asks = 0;
  await assert.rejects(untilOwnerAnswers(async () => { asks += 1; throw late(); }), { code: 'DEADLINE_EXCEEDED' });
  assert.equal(asks, OWNER_ASKS);
});
