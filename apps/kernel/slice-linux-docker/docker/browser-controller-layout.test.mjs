import assert from "node:assert/strict";
import test from "node:test";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";

const canonical = { css_width: 1280, css_height: 800, device_scale_factor: 2, desktop_pixel_width: 2560, desktop_pixel_height: 1600 };

function client() {
  const sent = [];
  const connection = { async send(method, params, sessionId) { sent.push({ method, params, sessionId }); return {}; } };
  const proto = BrowserCdpClient.prototype;
  const self = { connection, canonical, sent, layoutByTarget: new Map(), viewportByTarget: new Map(),
    ensureTargetSession: async (_c, targetId) => `s-${targetId}`,
    layoutMetrics: proto.layoutMetrics, layoutViewport: proto.layoutViewport };
  self.setLayout = proto.setLayout.bind(self);
  return self;
}

test("the person's display sets one Tab's layout size and null restores the canonical page", async () => {
  const c = client();
  await c.setLayout("t1", { width: 400, height: 760 });
  const [first] = c.sent;
  assert.equal(first.method, "Emulation.setDeviceMetricsOverride");
  assert.equal(first.sessionId, "s-t1");
  assert.deepEqual([first.params.width, first.params.height, first.params.deviceScaleFactor], [400, 760, 2]);
  // Same layout again: no second override. The Tab keeps its size across a canonical reconcile (metrics key).
  await c.setLayout("t1", { width: 400, height: 760 });
  assert.equal(c.sent.length, 1);
  assert.deepEqual(c.layoutViewport("t1", canonical), { ...canonical, css_width: 400, css_height: 760 });
  assert.equal(c.layoutViewport("t2", canonical), canonical);
  await c.setLayout("t1", null);
  assert.deepEqual([c.sent[1].params.width, c.sent[1].params.height], [1280, 800]);
  assert.equal(c.layoutByTarget.size, 0);
});
