// MP-08 / MP-09 / MP-10 / MP-11: real public pages through the production
// browser host. Supplementary to the hosted client/kernel/provider card drill.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

test('MP-08/MP-10/MP-11 real public browser regions follow scroll/resize and retire on navigation', { timeout: 120_000 }, async () => {
  assert(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, 'explicit installed Chromium required');
  const root = await mkdtemp(path.join(tmpdir(), 'public-card-region-'));
  const host = new KernelBrowserHost(root, { appStorageRoot: path.join(root, 'apps') });
  const scope = 'agent:public-card-region';
  try {
    for (const url of ['https://en.wikipedia.org/wiki/Computer', 'https://developer.mozilla.org/en-US/docs/Web/API/Element/getBoundingClientRect']) {
      const opened = await host.request({ op: 'open', url, observed_by: scope });
      const binding = { tab_id: opened.tab_id, generation: opened.generation, observed_by: scope };
      let snapshot, heading;
      for (let attempt = 0; attempt < 50; attempt++) {
        snapshot = await host.request({ op: 'snapshot', ...binding });
        heading = snapshot.snapshot.dom_nodes.find(n => n.node_name === 'H1' && n.document_index === 0 && n.bounds?.height > 0);
        if (heading) break;
        await delay(100);
      }
      assert(heading, `real public heading must be observed at ${new URL(url).hostname}`);
      const command = { op: 'popup_region', ...binding, ref: heading.node_ref };
      const first = await host.request(command);
      assert.equal(first.state, 'present');
      assert.equal(first.region.origin, new URL(url).origin);
      assert(first.region.rect.width > 0 && first.region.rect.height > 0);
      const box = region => ({ rect: region.rect, viewport_width: region.viewport_width, viewport_height: region.viewport_height });
      const pending = host.request({ ...command, document_id: first.region.document_id, after: box(first.region) });
      await host.request({ op: 'input', ...binding, document_id: first.region.document_id, input: { kind: 'scroll', x: 300, y: 300, delta_x: 0, delta_y: 360 } });
      const scrolled = await pending;
      assert.equal(scrolled.state, 'present');
      assert.notDeepEqual(scrolled.region.rect, first.region.rect, 'the live region must follow actual public-page scroll');
      await host.request({ op: 'resize', ...binding, width: 800, height: 600 });
      const resized = await host.request({ ...command, document_id: first.region.document_id, after: box(scrolled.region) });
      assert.equal(resized.region.viewport_width, 800);
      assert.equal(resized.region.viewport_height, 600);
      // Another observed Tab cannot supply this Tab's reference, even when both pages are public.
      const other = await host.request({ op: 'open', url: 'https://www.wikipedia.org/', observed_by: scope });
      await assert.rejects(host.request({ ...command, tab_id: other.tab_id }), { code: 'user_domain_stale_reference' });
      await host.request({ op: 'navigate', ...binding, url: 'https://www.wikipedia.org/' });
      const ended = await host.request({ ...command, document_id: first.region.document_id });
      assert.equal(ended.state, 'condition_met');
      assert.equal(ended.reason, 'Browser document changed');
      console.log(`MP-08/MP-09/MP-10/MP-11 public browser region passed: ${new URL(url).hostname}`);
    }
  } finally {
    await host.stop();
    if (host.chromium.browserUser) await rm(host.chromium.browserUser.root, { recursive: true, force: true });
    await rm(root, { recursive: true, force: true });
  }
});
