import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { mayRestart, recordRestart, MAX_RESTARTS, WINDOW_MS } from './kernel-browser-controller-restart.mjs';

test('row 548: a host that cannot run Chrome is not restarted in a loop', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'row548-restarts-'));
  try {
    const now = 1_000_000;
    for (let n = 0; n < MAX_RESTARTS; n++) assert.equal(await recordRestart(root, now + n), true);
    assert.equal(await recordRestart(root, now + 10), false, 'the fourth restart inside the window is refused');
    assert.equal(await recordRestart(root, now + WINDOW_MS + MAX_RESTARTS), true, 'after the window it may restart again');
    assert.equal(mayRestart([], now), true);
  } finally { await rm(root, { recursive: true, force: true }); }
});
