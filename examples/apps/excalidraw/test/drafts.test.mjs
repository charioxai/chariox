// The drafts of replaced windows against a fake backend that enforces what the real App and the
// catalog enforce: at most four drafts of other windows, and 512 KiB per call and per result.
import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { openApp, closeEnvironment, fakeBackend, rect, sceneOf, waitUntil } from './window.mjs';

after(closeEnvironment);

/** A rectangle that carries `kib` KiB of data, so a scene of a few of them is near the 192 KiB scene limit. */
const bulky = (id, kib) => rect(id, 100, 100, 120, 80, { customData: { pad: 'x'.repeat(kib * 1024) } });

for (const dpr of [1, 2]) test(`three near-limit drafts are all recovered at DPR${dpr}`, { timeout: 90_000 }, async () => {
  const backend = fakeBackend();
  // Three windows that were replaced with a 174 KiB edit each: together over the catalog's 512 KiB call limit.
  for (const n of [1, 2, 3]) {
    backend.revisions.set(`older-${n}`, n);
    backend.drafts.set(`older-${n}`, sceneOf([bulky('big-a', 58), bulky('big-b', 58), bulky('big-c', 58), rect(`mark-${n}`, 100 * n, 400)]));
  }
  const app = await openApp({ backend, dpr });
  try {
    await app.load();
    await app.until(elements => ['mark-1', 'mark-2', 'mark-3'].every(id => elements.some(element => element.id === id)), { timeout: 15_000, what: 'the marks of all three drafts' });
    await waitUntil(() => backend.live().length === 6, { what: 'the recovered edits to be saved' });
    app.assertClean();
  } finally { await app.close(); }
});
