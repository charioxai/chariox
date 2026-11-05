import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ResourceScope, createHandlers } from '../src/core.js';

test('an abandoned pending response is closed when its import settles', async () => {
  let complete;
  let live = 1;
  const scope = new ResourceScope();
  const pending = scope.own(new Promise(resolve => { complete = resolve; }), async body => {
    await body.return();
  });
  const rejected = assert.rejects(pending, e => e.code === 'CANCELLED');
  await scope.close();
  assert.equal(live, 1, 'pending import has not delivered its handle yet');
  complete({ return() { live--; } });
  await rejected;
  assert.equal(live, 0);
});

test('scope exit closes owned streams exactly once', async () => {
  let drops = 0;
  const scope = new ResourceScope();
  await scope.own(Promise.resolve({}), () => { drops++; });
  await scope.close();
  await scope.close();
  assert.equal(drops, 1);
});

test('cancelled handler returns before the import and disposes the late response', async () => {
  let finish, cancel;
  let live = 1;
  const handlers = createHandlers({ tools: {
    wait: async (_input, ctx) => {
      await ctx.resources.own(new Promise(resolve => { finish = resolve; }), () => { live--; });
      return {};
    },
  } }, { registry: { tool() {} } });
  const result = handlers.callTool('wait', '{}', {}, { read: () => new Promise(resolve => { cancel = resolve; }) });
  const rejected = assert.rejects(result, e => e.code === 'CANCELLED');
  cancel('stopped');
  await rejected;
  assert.equal(live, 1);
  finish({});
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(live, 0);
});

test('handler errors and success both release streams', async () => {
  for (const fail of [false, true]) {
    let drops = 0;
    const handlers = createHandlers({ tools: { run: async (_, ctx) => {
      await ctx.resources.own(Promise.resolve({}), () => { drops++; });
      if (fail) throw new Error('bad');
      return { ok: true };
    } } }, {});
    const result = handlers.callTool('run', '{}', {}, { read: () => new Promise(() => {}) });
    if (fail) await assert.rejects(result, e => e.code === 'HANDLER_FAILED');
    else assert.deepEqual(JSON.parse(await result), { ok: true });
    assert.equal(drops, 1);
  }
});

test('normal completion cancels and awaits the pending cancellation read before drop', async () => {
  let reject, acknowledged = false, dropped = false;
  const pending = new Promise((_, fail) => { reject = fail; });
  const handlers = createHandlers({ tools: { run: () => true } }, {});
  assert.equal(await handlers.callTool('run', '{}', {}, {
    read: () => pending,
    cancelRead() { Promise.resolve().then(() => { acknowledged = true; reject(Error('read cancelled')); }); },
    drop() { assert.equal(acknowledged, true); dropped = true; },
  }), 'true');
  assert.equal(dropped, true);
});

test('even an import handed to an already closed scope releases its late result', async () => {
  let finish, released = 0;
  const pending = new Promise(resolve => { finish = resolve; });
  const scope = new ResourceScope();
  await scope.close();
  const result = scope.own(pending, () => { released++; });
  const rejection = assert.rejects(result, error => error.code === 'CANCELLED');
  finish({});
  await rejection;
  assert.equal(released, 1);
});

test('concurrent scope closes await the same complete disposal', async () => {
  let finish, disposed = false;
  const scope = new ResourceScope();
  await scope.own(Promise.resolve({}), () => new Promise(resolve => { finish = () => { disposed = true; resolve(); }; }));
  const first = scope.close();
  const second = scope.close();
  await Promise.resolve();
  let returned = false;
  second.then(() => { returned = true; });
  await Promise.resolve();
  assert.equal(returned, false);
  finish();
  await Promise.all([first, second]);
  assert.equal(disposed, true);
});

test('an already completed cancellation read is dropped without cancelling it again', async () => {
  let dropped = false;
  const handlers = createHandlers({ tools: { run: () => new Promise(() => {}) } }, {});
  await assert.rejects(handlers.callTool('run', '{}', {}, {
    read: () => Promise.resolve('stop'),
    cancelRead() { throw Error('cannot cancel a completed future read'); },
    drop() { dropped = true; },
  }), error => error.code === 'CANCELLED');
  assert.equal(dropped, true);
});

test('disposal failures return a typed non-retryable error', async () => {
  const handlers = createHandlers({ tools: { run: async (_, ctx) => {
    await ctx.resources.own(Promise.resolve({}), () => { throw Error('stream drop failed'); });
    return true;
  } } }, {});
  await assert.rejects(handlers.callTool('run', '{}', {}, { read: () => new Promise(() => {}) }), error => error.code === 'RESOURCE_CLOSE_FAILED' && error.retryable === false);
});
