import test from 'node:test';
import assert from 'node:assert/strict';
import { BrowserRendererRecovery } from './browser-renderer-recovery.mjs';

for (const repeatedCrash of [true, false]) test(`MP-08/MP-10/MP-11: acknowledged reload retries until commit (second crash=${repeatedCrash})`, async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let reloads = 0;
  const connection = { send: async method => { assert.equal(method, 'Page.reload'); reloads++; } };
  const browser = { connection, memory: { requireHeadroom: async () => {} },
    targetsBySession: new Map([['s', 't']]), sessionsByTarget: new Map([['t', 's']]),
    documentIdsByTarget: new Map([['t', 'old']]), pageFramesByTarget: new Map(),
    focusWorldsByTarget: new Map(), snapshotStateByTarget: new Map() };
  const recovery = new BrowserRendererRecovery(browser);
  const crash = () => recovery.observe({ method: 'Inspector.targetCrashed', sessionId: 's' });
  try {
    crash(); t.mock.timers.tick(1000); await new Promise(setImmediate);
    assert.equal(reloads, 1);
    assert.equal(recovery.crashed.has('t'), true, 'acknowledgment is not a document commit');
    if (repeatedCrash) crash();
    t.mock.timers.tick(5000); await new Promise(setImmediate);
    assert.equal(reloads, 2, 'an uncommitted replacement renderer must get another bounded attempt');
    recovery.observe({ method: 'Page.frameNavigated', sessionId: 's', params: { frame: { loaderId: 'new' } } });
    t.mock.timers.tick(60000); await new Promise(setImmediate);
    assert.equal(reloads, 2, 'a committed replacement retires recovery');
    assert.equal(recovery.crashed.size, 0);
  } finally { recovery.close(); }
});
