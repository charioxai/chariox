import test from 'node:test';
import assert from 'node:assert/strict';
import { DisplayStream, PortableEncoder, safeChildPid, dirtyTiles } from './kernel-browser-display.mjs';
import { encodePng, decodePng, maskPng } from './kernel-browser-pixels.mjs';
function fixture(value = 255) {
  const pixels = Buffer.alloc(256 * 256 * 4, 255); pixels[0] = value;
  return { generation: 1, data_base64: encodePng(256, 256, pixels) };
}
const binding = { subscription_id: 's', tab_id: 't', device_scale_factor: 2, bitrate: 2_000_000, codec: 'vp09.00.10.08' };
test('MD-DISPLAY unsafe process IDs rejected without signals', () => {
  for (const pid of [undefined, NaN, 0, 1, -1, -50, 2.5]) assert.throws(() => safeChildPid({ pid }));
  assert.equal(safeChildPid({ pid: 42 }), 42);
});
test('MD-DISPLAY video, exact settle, dirty patches and lost base recovery', async () => {
  const waits = [];
  const stream = new DisplayStream(binding, { encoder: { encode: async () => 'YWJj', close: async () => {} }, now: () => 0, wait: async ms => waits.push(ms) });
  const video = await stream.frame(fixture(), 'd', 0); assert.equal(video.kind, 'video');
  const exact = await stream.frame(fixture(), 'd', 1); assert.equal(exact.kind, 'png');
  assert.equal(await stream.frame(fixture(), 'd', 2), null);
  const patch = await stream.frame(fixture(0), 'd', 2); assert.equal(patch.kind, 'tiles'); assert.equal(patch.base_sequence, 2);
  assert.equal(patch.tiles.length, 1); assert.equal(decodePng(patch.tiles[0].data_base64).pixels[0], 0);
  const recover = await stream.frame(fixture(0), 'd', 0); assert.equal(recover.kind, 'video');
  const newDocument = await stream.frame(fixture(0), 'new-document', 4); assert.equal(newDocument.kind, 'video');
  stream.invalidate(); assert.equal((await stream.frame(fixture(), 'new-document', 5)).kind, 'video');
  assert.ok(waits.every(ms => ms > 0));
});
test('MD-DISPLAY DPR2 protected pixel masking scales coordinates', () => {
  const data = encodePng(2560, 1600, Buffer.alloc(2560 * 1600 * 4, 255));
  assert.throws(() => decodePng(data));
  const result = decodePng(maskPng(data, [[200, 200, 40, 40]], 2), 2);
  assert.equal(result.pixels[(210 * 2560 + 210) * 4], 0);
  assert.equal(result.pixels[(100 * 2560 + 100) * 4], 255);
});
test('MD-DISPLAY async encoder launch failure rejects into cleanup', async () => {
  const original = process.env.CHARIOX_BROWSER_DISPLAY_PYTHON;
  process.env.CHARIOX_BROWSER_DISPLAY_PYTHON = '/nonexistent/md-display-python';
  const encoder = new PortableEncoder();
  try { await assert.rejects(encoder.encode(fixture().data_base64, 2_000_000)); assert.equal(encoder.child, null); }
  finally { await encoder.close(); if (original === undefined) delete process.env.CHARIOX_BROWSER_DISPLAY_PYTHON; else process.env.CHARIOX_BROWSER_DISPLAY_PYTHON = original; }
});
test('MD-DISPLAY disabled commands fail before launching host Chromium', async () => {
  const { KernelBrowserHost } = await import('./kernel-browser-host.mjs');
  const host = new KernelBrowserHost('/unused-md-display-profile', { chromium: { start: async () => { throw new Error('must not launch'); } } });
  const original = process.env.CHARIOX_KERNEL_BROWSER_DISPLAY; delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;
  try {
    for (const op of ['display_subscribe','display_next','display_input']) await assert.rejects(host.request({op}), /experimental display disabled/);
  } finally { if(original !== undefined) process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=original; }
});
