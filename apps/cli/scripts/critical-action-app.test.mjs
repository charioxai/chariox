import assert from 'node:assert/strict';
import test from 'node:test';
import register from './fixtures/critical-action-app/bundle/runtime/main.mjs';

function fixture({ failHeaders = false, failOpen = false } = {}) {
  let handler;
  const active = new Set();
  const logs = [];
  let next = 0;
  const cancelled = [];
  register({
    events: { register(_name, callback) { handler = callback; } },
    validation: { async request() { return { operationId: `op-${++next}` }; } },
    log: { async write(_level, _message, fields) { logs.push(fields); } },
    http: {
      async open({ operationId }) {
        if (failOpen) throw { code: 'VALIDATION_REQUIRED' };
        if (active.size >= 4) throw { code: 'APP_BUSY' };
        active.add(operationId);
        return { streamId: operationId };
      },
      async headers() {
        if (failHeaders) throw { code: 'HTTP_FAILED' };
        return { pending: false, status: 200 };
      },
      async cancel(id) { cancelled.push(id); active.delete(id); },
    },
  });
  return { active, logs, cancelled, run: () => handler({ payload: { step: 'unapproved' } }) };
}

test('repeated critical-action drill calls release all four HTTP admission slots', async () => {
  const app = fixture();
  for (let i = 0; i < 6; i++) await app.run();
  assert.deepEqual(app.logs.map(({ result }) => result), Array(6).fill({ status: 200 }));
  assert.equal(app.active.size, 0);
  assert.equal(app.cancelled.length, 6);
});

test('header failure still releases the opened stream', async () => {
  const app = fixture({ failHeaders: true });
  await app.run();
  assert.deepEqual(app.logs[0].result, { error: 'HTTP_FAILED' });
  assert.equal(app.active.size, 0);
  assert.deepEqual(app.cancelled, ['op-1']);
});

test('refused open never cancels an unallocated stream', async () => {
  const app = fixture({ failOpen: true });
  await app.run();
  assert.deepEqual(app.logs[0].result, { error: 'VALIDATION_REQUIRED' });
  assert.deepEqual(app.cancelled, []);
});
