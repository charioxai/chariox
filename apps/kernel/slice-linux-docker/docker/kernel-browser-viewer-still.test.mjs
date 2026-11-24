// Row 341 (A): at a DPR2 viewer of a DPR1 page, the whole-page video's exact still at rest is taken at the viewer's
// density. Live video stays at the page's own density.
import test from 'node:test';
import assert from 'node:assert/strict';
import { DisplayStream } from './kernel-browser-display.mjs';
import { NativeRefiner } from './kernel-browser-refiner.mjs';
import { encodePng } from './kernel-browser-pixels.mjs';
import { randomBytes } from 'node:crypto';
import { viewerDensityStill, viewerStillPending } from './kernel-browser-viewer-still.mjs';

const png = (width, height, value = 255) => encodePng(width, height, Buffer.alloc(width * height * 4, value));
const binding = { subscription_id: 's', tab_id: 't', device_scale_factor: 1, viewer_scale: 2, css_width: 128, css_height: 64, bitrate: 2_000_000, codec: 'vp09.00.10.08', dependencies: true };

test('row 341 A: a viewer-density still goes out as one PNG at the viewer density, and video returns to the page density', async () => {
  const resets = [];
  const stream = new DisplayStream(binding, { encoder: { encode: async (_png, _bitrate, reset) => { resets.push(reset); return 'YWJj'; }, close: async () => {} }, now: () => 0, wait: async () => {} });
  try {
    const video = await stream.frame({ generation: 1, data_base64: png(128, 64, 200) }, 'd', 0);
    assert.equal(video.kind, 'video'); assert.equal(video.device_scale_factor, 1); assert.equal(video.width, 128);
    const still = await stream.frame({ generation: 1, data_base64: png(256, 128), viewer_density: 2, settled_verified: true }, 'd', 1);
    assert.equal(still.kind, 'png', 'a full PNG: tiles cannot change the canvas size');
    assert.equal(still.device_scale_factor, 2); assert.equal(still.width, 256); assert.equal(still.height, 128);
    assert.equal(still.css_width, 128); assert.equal(still.css_height, 64);
    assert.equal(stream.exact, true);
    assert.equal(await stream.frame({ generation: 1, data_base64: png(256, 128), viewer_density: 2, settled_verified: true }, 'd', 2), null, 'the same still is not sent twice');
    stream.compositorSerial = 10; stream.compositorMasks = '[]';
    const raw = { nativeExact: async () => {}, format: 'bgr0', width: 128, height: 64, length: 128 * 64 * 4, pixels: Buffer.alloc(128 * 64 * 4), damage: [0, 0, 128, 64], base_serial: 10, serial: 11, adjacent_damage_tiles: [[0, 0, 8, 8]] };
    assert.equal(stream.canPatchNative({ serial: 11, raw }), false, 'page-density patches never land on the viewer-density canvas');
    const next = await stream.frame({ generation: 1, data_base64: png(128, 64, 100) }, 'd', 2);
    assert.equal(next.kind, 'video'); assert.equal(next.device_scale_factor, 1); assert.equal(next.width, 128);
    assert.equal(resets.at(-1), true, 'the first video frame after the still is independent');
  } finally { await stream.close(); }
});

test('row 341 A: a page-density exact still keeps the page density (DPR1 viewers unchanged)', async () => {
  const stream = new DisplayStream({ ...binding, viewer_scale: 1 }, { encoder: { encode: async () => 'YWJj', close: async () => {} }, now: () => 0, wait: async () => {} });
  try {
    await stream.frame({ generation: 1, data_base64: png(128, 64, 200) }, 'd', 0);
    const exact = await stream.frame({ generation: 1, data_base64: png(128, 64), settled_verified: true }, 'd', 1);
    assert.equal(exact.kind, 'png'); assert.equal(exact.device_scale_factor, 1); assert.equal(exact.width, 128);
  } finally { await stream.close(); }
});

function stillHost({ regions = [], origins = [{ pageX: 0, pageY: 300, scale: 1 }], size = 256 } = {}) {
  const shots = [], metrics = [...origins];
  const host = { scales: new Map([['t', 1]]), screenshot: async (tab, clip, protectedCapture, format, optimizeForSpeed, mask) => {
    shots.push({ clip, protectedCapture, format, optimizeForSpeed, mask });
    return { data_base64: png(clip.width * clip.scale, clip.height * clip.scale).padEnd(size, 'A'), protected_regions: regions };
  } };
  const compositor = { sessionId: 's', connection: { send: async (method) => { assert.equal(method, 'Page.getLayoutMetrics'); const v = metrics.length > 1 ? metrics.shift() : metrics[0]; return { cssVisualViewport: v }; } } };
  return { host, compositor, shots };
}

test('row 341 A: the viewer-density still is the protected, masked viewport through a clip at viewer/page scale', async () => {
  const { host, compositor, shots } = stillHost();
  const still = await viewerDensityStill({ host, tab: { tab_id: 't' }, compositor, stream: binding });
  assert.deepEqual(shots, [{ clip: { x: 0, y: 300, width: 128, height: 64, scale: 2 }, protectedCapture: true, format: 'png', optimizeForSpeed: false, mask: true }]);
  assert.equal(still.viewer_density, 2); assert.equal(still.width, 256); assert.equal(still.height, 128); assert.equal(still.device_scale_factor, 2);
});

test('row 341 A: no viewer-density still for DPR1 viewers, protected fields, a zoomed or moving viewport, or one too big for a frame', async () => {
  const tab = { tab_id: 't' };
  assert.equal(await viewerDensityStill({ ...stillHost(), tab, stream: { ...binding, viewer_scale: 1 } }), null, 'DPR1 viewer');
  { const s = stillHost(); s.host.scales.set('t', 2); assert.equal(await viewerDensityStill({ ...s, tab, stream: binding }), null, 'page already at the viewer density'); }
  assert.equal(await viewerDensityStill({ ...stillHost({ regions: [{ x: 1, y: 1, width: 4, height: 4 }] }), tab, stream: binding }), null, 'a Vault-protected field keeps the page-density path');
  assert.equal(await viewerDensityStill({ ...stillHost({ origins: [{ pageX: 0, pageY: 0, scale: 1.5 }] }), tab, stream: binding }), null, 'zoomed');
  await assert.rejects(viewerDensityStill({ ...stillHost({ origins: [{ pageX: 0, pageY: 0, scale: 1 }, { pageX: 0, pageY: 40, scale: 1 }] }), tab, stream: binding }), error => error.busy === true, 'moved during the capture: retried later');
  assert.equal(await viewerDensityStill({ ...stillHost({ size: 1024 * 1024 }), tab, stream: binding }), null, 'over one frame');
});

test('row 341 A: the refiner uses a viewer-density still instead of the native page-density exact', async () => {
  let nativeExactCalls = 0;
  const still = { data_base64: png(256, 128), width: 256, height: 128, viewer_density: 2, device_scale_factor: 2 };
  const raw = { retain() {}, release() {}, nativeExact: async () => { nativeExactCalls++; return { native_exact: true }; }, width: 128, height: 64, pixels: Buffer.alloc(128 * 64 * 4) };
  const refiner = new NativeRefiner(async () => still, { now: () => 1000, quietNativeMs: 0, quietMs: 0 });
  try {
    const request = { source: {}, document: 'd', policy: {}, epoch: 0, serial: 1, native: true, sample: { raw }, scale: 1 };
    assert.equal(refiner.request(request, 0, () => true), null);
    await refiner.active;
    const latest = refiner.request(request, 0, () => true);
    assert.equal(nativeExactCalls, 0);
    assert.equal(latest.viewer_density, 2); assert.equal(latest.pixels.width, 256); assert.equal(latest.settled_verified, true);
  } finally { await refiner.close(); }
});

test('row 341 A: one capture per page picture, a refusal is kept for that picture, and a failing capture falls back', async () => {
  const { displayCreditViewerStill } = await import('./kernel-browser-display-credit.mjs');
  const { host, compositor, shots } = stillHost();
  const stream = { ...binding };
  const request = { source: compositor, document: 'd', serial: 4, pixels: 'aaaa', epoch: 0, policy: {} };
  const first = await displayCreditViewerStill(host, { tab_id: 't' }, stream, request);
  assert.equal(first.viewer_density, 2);
  assert.equal(await displayCreditViewerStill(host, { tab_id: 't' }, stream, { ...request, serial: 9 }), first);
  assert.equal(shots.length, 1, 'a new frame of the same pixels (the capture\'s own repaint) reuses the still');
  await displayCreditViewerStill(host, { tab_id: 't' }, stream, { ...request, pixels: 'bbbb' });
  assert.equal(shots.length, 2, 'changed pixels are captured again');
  host.screenshot = async () => { throw Error('target gone'); };
  assert.equal(await displayCreditViewerStill(host, { tab_id: 't' }, stream, { ...request, pixels: 'cccc' }), null, 'the page-density exact takes over');
  assert.equal(viewerStillPending(stream, { ...request, sample: { signature: 'cccc' } }), false, 'that picture keeps the page density');
  assert.equal(viewerStillPending(stream, { ...request, sample: { signature: 'dddd' } }), true, 'a changed page asks again');
});

// The headed whole-page video (a native region display) at rest, as one credit after another sees it.
function restingRegion({ regions = [] } = {}) {
  const { host: stillParts, compositor: cdp, shots } = stillHost({ regions });
  const policy = {};
  const raster = (value = 7) => ({ identical: false, width: 128, height: 64, pixels: Buffer.alloc(128 * 64 * 4, value), retain() {}, release() {}, nativeExact: async () => ({ native_exact: true }) });
  const sample = { raw: raster(), serial: 1, signature: '0000000000000001', document_id: 'd', tab_id: 't' };
  let current = sample;
  const source = { attested: true, native: true, policy, regionRevision: 0, changedAt: performance.now() - 1000, valid: () => true, allowed: () => true,
    sample: () => current, tab: { tab_id: 't', target_id: 'target', document_id: 'd' }, connection: cdp.connection, sessionId: 's' };
  const frames = [];
  const stream = { ...binding, tab_id: 't', document_id: 'd', sequence: 1, previous: {}, previousDensity: 1, exact: true, compositorSerial: 1, compositorMasks: '[]',
    compositorRegionRevision: 0, creditEpoch: 0, encoder: {}, acceptsCredit: () => true, canPatchNative: () => false, shiftKind: () => null,
    producer: { source, frames: [], feedback() {}, take: () => null, waitReady: async () => {} },
    regionSource: source, regionLease: { isAlive: () => true }, regionWindow: { isCurrent: () => true }, regionCurrent: async () => true,
    frame: async (frameSource, _document, _after, validate) => { const ok = await validate(); frames.push({ ok, viewer_density: frameSource.viewer_density }); return ok ? { sequence: 2, kind: 'png' } : null; } };
  const host = { ...stillParts, generation: 1, protection: policy, inputEpochs: new Map(), inputChangedAt: new Map(), timing() {}, compositorFor: async () => source };
  const credit = async () => { const { displayCredit } = await import('./kernel-browser-display-credit.mjs'); const r = await displayCredit(host, stream, { generation: 1, after_sequence: 1 }); await stream.refiner?.active; return r; };
  // The native worker's next frame: a new raster and serial, and a signature that is its serial.
  return { stream, shots, frames, credit, raster, next: (raw = raster()) => { const serial = current.serial + 1; current = { ...current, raw, serial, signature: serial.toString(16).padStart(16, '0') }; } };
}

test('row 341 A: a quiet whole-page video delivers its viewer-density still, also after the capture repainted the same pixels', async () => {
  const { stream, shots, frames, credit, next } = restingRegion();
  try {
    await credit();
    assert.equal(shots.length, 1, 'an exact page-density canvas at rest still owes the DPR2 viewer its still');
    next();
    await credit();
    assert.equal(shots.length, 1, 'the repaint with the same pixels (a new frame and signature) does not capture again');
    assert.deepEqual(frames, [{ ok: true, viewer_density: 2 }], 'the still passes delivery\'s fences');
  } finally { await stream.refiner?.close(); }
});

test('row 341 A: a changed raster at rest is captured again; its still is not offered for the old pixels', async () => {
  const { stream, shots, frames, credit, next, raster } = restingRegion();
  try {
    await credit();
    next(raster(9));
    stream.regionSource.changedAt = performance.now() - 1000;
    await credit();
    assert.equal(shots.length, 2, 'new pixels, new still');
    assert.deepEqual(frames, [], 'the first still answered pixels that are gone');
    await credit();
    assert.deepEqual(frames, [{ ok: true, viewer_density: 2 }]);
  } finally { await stream.refiner?.close(); }
});

test('row 341 A: a refused still (a Vault-protected field) returns the quiet page to the page-density path', async () => {
  const { stream, shots, frames, credit } = restingRegion({ regions: [{ x: 1, y: 1, width: 4, height: 4 }] });
  try {
    await credit();
    assert.equal(shots.length, 1);
    await credit(); await credit();
    assert.equal(shots.length, 1, 'no capture loop on a refused picture');
    assert.equal(frames.some(frame => frame.viewer_density), false);
    assert.equal(frames.every(frame => frame.ok), true, 'the canvas already holds the page-density exact: nothing is offered and refused');
  } finally { await stream.refiner?.close(); }
});

test('row 341 A: a page-density exact canvas is upgraded by the viewer-density still', async () => {
  const stream = new DisplayStream(binding, { encoder: { encode: async () => 'YWJj', close: async () => {} }, now: () => 0, wait: async () => {} });
  try {
    await stream.frame({ generation: 1, data_base64: png(128, 64, 200) }, 'd', 0);
    const exact = await stream.frame({ generation: 1, data_base64: png(128, 64), settled_verified: true }, 'd', 1);
    assert.equal(exact.kind, 'png'); assert.equal(exact.device_scale_factor, 1); assert.equal(stream.exact, true);
    const still = await stream.frame({ generation: 1, data_base64: png(256, 128), viewer_density: 2, settled_verified: true }, 'd', 2);
    assert.equal(still?.kind, 'png'); assert.equal(still.device_scale_factor, 2); assert.equal(still.width, 256);
  } finally { await stream.close(); }
});

test('row 341 A: only the whole-page video of a viewer that accepts stills at its density asks for them', async () => {
  const { regionViewerScale } = await import('./kernel-browser-region-display.mjs');
  const page = { kind: 'visible_page' }, command = { device_scale_factor: 2, viewer_density_stills: true };
  assert.deepEqual(regionViewerScale(page, command), { viewer_scale: 2 });
  assert.deepEqual(regionViewerScale(page, { device_scale_factor: 2 }), {}, 'a viewer that did not ask keeps the page density');
  assert.deepEqual(regionViewerScale(page, { ...command, tab_display: true }), {}, 'a navigation preview keeps the page density');
  assert.deepEqual(regionViewerScale({ kind: 'region' }, command), {}, 'element crops keep the page density');
});

// #1706 review P2: after a delivered DPR2 still, a changed page can reach the refiner before any video frame. Its page-density
// repair tiles cannot land on the viewer-density canvas (the presenter refuses a repair whose canvas changed).
const noise = (width, height) => encodePng(width, height, randomBytes(width * height * 4));
async function afterStill(options = {}) {
  const resets = [];
  const stream = new DisplayStream({ ...binding, ...options }, { encoder: { encode: async (_png, _bitrate, reset) => { resets.push(reset); return 'YWJj'; }, close: async () => {} }, now: () => 0, wait: async () => {} });
  await stream.frame({ generation: 1, data_base64: png(128, 64, 200) }, 'd', 0);
  const still = await stream.frame({ generation: 1, data_base64: png(256, 128), viewer_density: 2, settled_verified: true }, 'd', 1);
  assert.equal(still.device_scale_factor, 2);
  return { stream, resets };
}

test('#1706 P2: a page-density native repair after a delivered DPR2 still sends no tiles; video restores the page density first', async () => {
  const { stream, resets } = await afterStill();
  try {
    const repair = await stream.frame({ generation: 1, width: 128, height: 64, data_base64: 'native-11', native_exact: true, native_repair: true, settled_verified: true,
      repair_tiles: [{ x: 0, y: 0, width: 64, height: 64, data_base64: png(64, 64, 9) }] }, 'd', 2);
    assert.notEqual(repair?.kind, 'tiles', 'no DPR1 tiles over the DPR2 canvas');
    const next = await stream.frame({ generation: 1, data_base64: png(128, 64, 100) }, 'd', 2);
    assert.equal(next.kind, 'video'); assert.equal(next.device_scale_factor, 1);
    assert.equal(resets.at(-1), true, 'the video frame that restores the page density is independent');
  } finally { await stream.close(); }
});

test('#1706 P2: a whole page-density exact after a DPR2 still ships as one PNG, even above the repair batch size', async () => {
  const { stream } = await afterStill({ bitrate: 100_000 });
  try {
    const picture = noise(128, 64);
    assert.ok(JSON.stringify({ kind: 'png', data_base64: picture }).length > 24_000, 'above the smallest repair batch');
    const exact = await stream.frame({ generation: 1, data_base64: picture, settled_verified: true }, 'd', 2);
    assert.equal(exact.kind, 'png', 'a PNG sets the canvas size; tiles would be refused');
    assert.equal(exact.device_scale_factor, 1); assert.equal(exact.width, 128);
  } finally { await stream.close(); }
});

test('#1706 P2: while the canvas is at the viewer density, the refiner asks the native worker for a whole exact, not repair tiles', async () => {
  const { stream, credit, next, raster } = restingRegion();
  const asked = [];
  try {
    Object.assign(stream, { previousDensity: 2, compositorSerial: 1 });
    const raw = { ...raster(9), nativeExact: async (options) => { asked.push(options.repair_only); return { native_exact: true, data_base64: png(128, 64, 9) }; } };
    next(raw);
    await credit();
    assert.deepEqual(asked, [false]);
    Object.assign(stream, { previousDensity: 1, viewer_scale: 1 }); stream.refiner.invalidate();
    next({ ...raw });
    await credit();
    assert.deepEqual(asked, [false, true], 'at the page density, repairs stay tiles');
  } finally { await stream.refiner?.close(); }
});
