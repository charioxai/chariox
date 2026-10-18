// MP-08/MP-10/MP-11: DOM mirror v2 service (local protocol 489). One snapshot
// per document, then pushed deltas: each `mirror_next` credit with `wait_ms`
// is answered when the page changes (credit-gated long poll, at most a few in
// flight per viewer). Resources are hash-addressed and follow the packet.
import { createHash } from 'node:crypto';
import { createDeflateRaw, constants as zlib } from 'node:zlib';
import { mirrorExceptionReason } from './kernel-browser-error-label.mjs';
import { timestamp } from './kernel-browser-timing.mjs';
import { mirror2ObserverExpression, sanitizeMirrorCss } from './kernel-browser-mirror2-observer.mjs';
import { assertCurrentDocument, assertNotCancelled, BrowserActionError } from './browser-controller-actions.mjs';
import { captureRegionMasks } from './kernel-browser-region-protection.mjs';
import { decodePng, maskPixels } from './kernel-browser-pixels.mjs';
import { losslessRegion } from './kernel-browser-pixels.mjs';
import { MirrorFrames, slotOf, localId, rebaseId } from './kernel-browser-mirror2-frames.mjs';
import { MirrorCssEvents } from './kernel-browser-mirror2-css.mjs';
import { discoverClosedHosts } from './kernel-browser-mirror2-closed.mjs';
// Protocol 530: mirrored undo/redo as named mirror keys (the renderer never sends raw modifiers here).
const HISTORY_CHORDS = new Map([['Ctrl+Z', 2], ['Ctrl+Shift+Z', 10]]);
import { Mirror2Regions } from './kernel-browser-mirror2-regions.mjs';
import {waitForNavigationPreview} from './kernel-browser-mirror2-preview.mjs';

const TARGET_ATTRIBUTES = new Set(['type', 'contenteditable']);
// MP-08/MP-10: packet age does not establish loss on a shaped link. Only an
// explicit viewer recovery credit resends history, preserving its original bytes.
const HISTORY_BYTES = 8 * 1024 * 1024;
// GATE-bytes: sheets from this size (normalized text) are pruned to their used rules for a first paint.
const FIRST_PAINT_SHEET = 32 * 1024;
const MAX_WAIT_MS = 2000, RESOURCE_PACKET_BYTES = 256 * 1024, RESOURCE_BYTES = 4 * 1024 * 1024, TILE_REFRESH_MS = 1000;
// Protocol 489: resource bytes travel in slices (base64 characters, a multiple
// of 4) of at most a packet budget, smaller while the viewer is typing or
// clicking so an echo never waits behind more than one slice on the socket.
const INPUT_PACKET_BYTES = 64 * 1024, MIN_SLICE = 16 * 1024;
// Protocol 489: a body from this size travels deflated in the subscription's
// context (sync flush per packet, a fresh context per reset); smaller ones
// (echoes) stay plain and outside the context.
const DEFLATE_MIN_BYTES = 512;
// MP-10 wheel bytes: element images within one viewport of the view travel;
// while the view moves (and SETTLE_MS after) a large one travels as a preview,
// then its exact bytes.
// MP-08/MP-10 (owner A/B 2026-10-14): like Chrome's paint holding, a new document's
// first snapshot waits for its first contentful paint (bounded), so viewers keep the
// previous page instead of a white, half-parsed one.
const PAINT_HOLD_MS = 2500;
// A page parsing a huge document answers renderer calls only between its tasks. The first snapshot's calls wait for it
// (the native preview covers the viewer meanwhile) instead of timing out, re-queueing and restarting.
// The kernel gives the controller 20 s per request (kernel_browser_host.rs, queue wait included): a credit that outlasts it
// reaches the viewer as a transport failure, not as a busy page. A busy page's answer stays well inside it.
export const KERNEL_CONTROLLER_DEADLINE_MS = 20_000;
export const BUSY_RENDERER_MS = 14_000;
const CREDIT_PATIENCE_MS = 6_500;
// A hung renderer still ends: the one snapshot in flight is abandoned after this.
const SNAPSHOT_MS = 120_000;
export const paintHoldExpression = ms => `new Promise(done => { const end = Date.now() + ${ms}; const check = () => document.readyState === 'complete' || performance.getEntriesByType('paint').some(entry => entry.name === 'first-contentful-paint') || Date.now() >= end ? done(true) : setTimeout(check, 25); check(); })`;
const NEAR_PX = 800, SETTLE_MS = 300, QUIET_MS = 1000, PREVIEW_MIN_BASE64 = 8 * 1024, PREVIEW_TYPES = new Set(['image/png', 'image/jpeg', 'image/webp', 'image/avif']);
// Trusted admission error: never constructed from page/CDP error strings.
export class MirrorInputEpochRefusal extends Error { constructor() { super('MP-11: stale mirror input epoch'); } }

// Bounded media type sniffing; decoded size is bounded before any client decoder.
export function mirrorResourceType(bytes, kind) {
  const text = bytes.subarray(0, 512).toString('utf8').replace(/^﻿/, '').trimStart();
  if (kind === 'image') {
    if (bytes.length >= 24 && bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return 'image/png';
    if (bytes.length >= 10 && ['GIF87a', 'GIF89a'].includes(bytes.toString('ascii', 0, 6))) return 'image/gif';
    if (bytes[0] === 255 && bytes[1] === 216 && bytes[2] === 255) return 'image/jpeg';
    if (bytes.length >= 16 && bytes.toString('ascii', 0, 4) === 'RIFF' && bytes.toString('ascii', 8, 12) === 'WEBP') return 'image/webp';
    if (bytes.length >= 12 && bytes.toString('ascii', 4, 12) === 'ftypavif') return 'image/avif';
    if (/^(<\?xml[^>]*>\s*)?(<!--[\s\S]*?-->\s*)*(<!DOCTYPE[^>]*>\s*)?<svg[\s>]/i.test(text)) return 'image/svg+xml';
    return null;
  }
  if (bytes.length >= 48 && ['wOF2', 'wOFF'].includes(bytes.toString('ascii', 0, 4))) return bytes.toString('ascii', 0, 4) === 'wOF2' ? 'font/woff2' : 'font/woff';
  if (bytes.length >= 12 && (bytes.readUInt32BE(0) === 0x00010000 || bytes.toString('ascii', 0, 4) === 'true')) return 'font/ttf';
  if (bytes.length >= 12 && bytes.toString('ascii', 0, 4) === 'OTTO') return 'font/otf';
  return null;
}

// Leading @import rules of a sheet's source (only @charset/@layer statements may
// precede them), with their layer/supports/media conditions as wrapping blocks.
// The wrapped text is parsed by the CSSOM again before it is sanitized.
export function cssImports(text) {
  const out = []; let rest = text, layers = '';
  for (let i = 0; i < 64; i++) {
    // Layer statements may only lead (an @import after a later one is ignored); they fix the layer order first.
    const lead = (i ? /^(?:\s+|\/\*[\s\S]*?\*\/)*/ : /^(?:\s+|\/\*[\s\S]*?\*\/|@charset\s+"[^"]*"\s*;|@layer\s+[^;{}]*;)*/i).exec(rest)[0];
    if (!i) layers = lead.replace(/\/\*[\s\S]*?\*\//g, '').match(/@layer\s+[^;{}]*;/gi)?.join('') ?? '';
    rest = rest.slice(lead.length);
    const match = /^@import\s+(?:url\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s"']*))\s*\)|"([^"]*)"|'([^']*)')([^;]*);/i.exec(rest);
    if (!match) break;
    rest = rest.slice(match[0].length);
    let condition = match[6].trim(), open = '', close = '';
    if (/[{}]/.test(condition)) continue;
    const layer = /^layer(?:\(([^()]*)\))?(?=\s|$)/i.exec(condition);
    if (layer) { open += `@layer ${layer[1]?.trim() ?? ''}{`; close += '}'; condition = condition.slice(layer[0].length).trim(); }
    if (/^supports\(/i.test(condition)) {
      let depth = 0, end = -1;
      for (let j = 9; j < condition.length && end < 0; j++) if (condition[j] === '(') depth++; else if (condition[j] === ')') { if (depth) depth--; else end = j; }
      if (end < 0) continue;
      open += `@supports (${condition.slice(9, end)}){`; close += '}'; condition = condition.slice(end + 1).trim();
    }
    if (condition) { open += `@media ${condition}{`; close += '}'; }
    out.push({ url: match[1] ?? match[2] ?? match[3] ?? match[4] ?? match[5], open, close });
  }
  return { layers, imports: out };
}

function dataUrlBytes(url) {
  const match = /^data:([^,]*?)(;base64)?,(.*)$/s.exec(url);
  if (!match) return null;
  try { return match[2] ? Buffer.from(match[3], 'base64') : Buffer.from(decodeURIComponent(match[3]), 'utf8'); } catch { return null; }
}

// Union-free upper bound is enough for the budget: overlap only overstates.
export const regionArea = boxes => boxes.reduce((sum, { box: [x, y, w, h] }) => sum + Math.max(0, Math.min(1280, x + w) - Math.max(0, x)) * Math.max(0, Math.min(800, y + h) - Math.max(0, y)), 0);
// Opaque regions are stills/region video over the mirrored page; only a viewport they mostly cover is the video's page.
export const regionOverBudget = boxes => regionArea(boxes) > 0.6 * 1280 * 800;

// Compact wire records (pre-order): [idDelta, parentBack, tag | kindCode, attrs | 0, extra].
// parentBack 0 = the context parent (null for a snapshot root, the op id for
// children ops); kind codes: 0 text (4th item = text), 1 document, 2 shadow,
// 3 frame, 4 mask, 5 tile. The client decodes and validates independently.
const KIND_CODES = { text: 0, document: 1, shadow: 2, frame: 3, mask: 4, tile: 5 };
export function encodeMirrorRecords(records, contextParent) {
  const index = new Map(), out = []; let previous = 0;
  records.forEach((record, i) => {
    const number = Number(record.id.slice(1));
    const back = index.has(record.parent) ? i - index.get(record.parent) : 0;
    if (!back && record.parent !== contextParent) throw new Error('MP-11: unordered mirror records');
    index.set(record.id, i);
    if (record.kind === 'text') out.push([number - previous, back, 0, record.text]);
    else {
      const { id, parent, kind, tag, attrs, ...extra } = record; void id; void parent;
      if (kind !== 'element' && tag !== undefined) extra.tag = tag;
      const row = [number - previous, back, kind === 'element' ? tag : KIND_CODES[kind]], more = Object.keys(extra).length > 0, named = attrs && Object.keys(attrs).length > 0;
      if (named || more) row.push(named ? attrs : 0);
      if (more) row.push(extra);
      out.push(row);
    }
    previous = number;
  });
  return out;
}

// A large stylesheet text travels once per snapshot epoch (pages repeat the same
// sheet across shadow roots/links); records and ops reference it by digest.
export const sheetDigest = text => createHash('sha256').update(text).digest('hex').slice(0, 24);
export function dedupeMirrorSheets(packet, sent) {
  const sheets = {};
  const ref = text => {
    if (typeof text !== 'string' || text.length < 2048) return null;
    const digest = sheetDigest(text);
    if (!sent.has(digest)) { sent.add(digest); sheets[digest] = text; }
    return digest;
  };
  const record = r => {
    const digest = ref(r.css); if (digest) { delete r.css; r.css_ref = digest; }
    if (r.adopted) r.adopted = r.adopted.map(text => { const d = ref(text); return d ? { ref: d } : text; });
  };
  for (const r of packet.nodes ?? []) record(r);
  for (const op of packet.ops ?? []) {
    if (op.op === 'children') op.nodes.forEach(record);
    else if (op.op === 'css') { const digest = ref(op.css); if (digest) { delete op.css; op.css_ref = digest; } }
    else if (op.op === 'adopted') op.sheets = op.sheets.map(text => { const d = ref(text); return d ? { ref: d } : text; });
  }
  if (Object.keys(sheets).length) packet.sheets = sheets;
}

// The subscription's own observer instance in a (shared) isolated world.
const observerRef = stream => `globalThis.__charioxMirror2.get(${JSON.stringify(String(stream.id))})`;

export class Mirror2 {
  constructor(service) { this.service = service; this.host = service.host; this.frames = new MirrorFrames(this); this.css = new MirrorCssEvents(); this.regions = new Mirror2Regions(this); }
  // MP-08/MP-10/MP-11: older v2 viewers never receive an unknown operation.
  viewerOps(stream, ops) { return stream.fontRequirements === true ? ops : ops.filter(op => op.op !== 'fonts'); }
  parentWorld(stream) { return stream.world; }
  async world(tab, stream, patience) {
    const targetAt = timestamp();
    const cached = this.host.browser.currentPageFrame?.(tab.target_id);
    const { connection, sessionId } = cached ?? await this.host.browser.resolvePageTarget(tab.target_id);
    const frameTree = cached ? { frame: cached.frame } : (await connection.send('Page.getFrameTree', {}, sessionId, { timeoutMs: patience?.() })).frameTree;
    if (frameTree?.frame?.loaderId !== tab.document_id) throw new Error('MP-11: stale mirror document');
    this.host.timing?.('mirror2_world_target', targetAt);
    const installAt = timestamp();
    const world = await this.host.browser.ensureFocusWorld(connection, sessionId, tab.target_id, frameTree.frame, { timeoutMs: patience?.() });
    if (!Number.isSafeInteger(world.contextId) || world.contextId <= 0) throw new Error('MP-11: mirror isolated world unavailable');
    if (!world.mirror2Installed) {
      const installed = await connection.send('Runtime.evaluate', { expression: `typeof globalThis.__charioxMirror2?.get === "function" || ${mirror2ObserverExpression()}`, contextId: world.contextId, returnByValue: true }, sessionId, { timeoutMs: patience?.() });
      if (installed.exceptionDetails || installed.result?.value !== true) throw new Error('MP-11: mirror observer unavailable');
      world.mirror2Installed = true;
    }
    this.host.timing?.('mirror2_world_install', installAt);
    const out = { connection, sessionId, contextId: world.contextId, navigatedAt: cached?.navigatedAt, frame: frameTree.frame, ref: observerRef(stream) };
    const cssAt = timestamp();
    await this.watchCss(out, frameTree.frame.id, stream.id, patience);
    this.host.timing?.('mirror2_world_css', cssAt);
    return out;
  }
  async watchCss(world, frameId, streamId, patience) {
    const large = await this.evaluate(world, "document.readyState !== 'complete' || document.getElementsByTagName('*').length > 10000", false, patience?.());
    world.holdInitialPaint = large;
    const cssAt = timestamp();
    // Activation re-reads sheet owners; ordinary pages need not await resource
    // bodies in DOM.enable/CSS.enable before their first sanitized snapshot.
    this.css.watch(world, frameId, streamId).then(() => this.host.timing?.('mirror2_css_ready', cssAt), () => { world.cssFailed = true; });
  }
  // One snapshot per world in flight. A snapshot the page has not answered keeps running in the renderer; the credit
  // that gave up on it leaves it pending and the next credit takes its answer instead of queueing a second one.
  async snapshotOnce(world, policy, patience) {
    if (world.snapshotPending?.policy !== policy) world.snapshotPending = null;
    const pending = world.snapshotPending ??= { policy, promise: this.evaluate(world, `${world.ref}.snapshot()`, false, SNAPSHOT_MS) };
    pending.promise.catch(() => { if (world.snapshotPending === pending) world.snapshotPending = null; }); // a failed snapshot is not kept
    let timer;
    const late = new Promise((_, reject) => { const ms = patience(); timer = setTimeout(() => reject(Object.assign(new Error(`browser CDP method Runtime.evaluate timed out after ${ms}ms`), { code: 'browser_cdp_timeout' })), ms); });
    try { const snap = await Promise.race([pending.promise, late]); if (world.snapshotPending === pending) world.snapshotPending = null; return snap; }
    finally { clearTimeout(timer); }
  }
  async evaluate(world, expression, awaitPromise = false, timeoutMs) {
    const reply = await world.connection.send('Runtime.evaluate', { expression, contextId: world.contextId, returnByValue: true, awaitPromise }, world.sessionId, { timeoutMs: timeoutMs ?? world.patience?.() });
    if (reply.exceptionDetails) {
      const label = /mirror2:([a-z_]+)/.exec(reply.exceptionDetails.exception?.description ?? '')?.[1];
      const error = new Error('MP-11: mirror observation unavailable'); error.mirrorReason = label ?? mirrorExceptionReason(reply.exceptionDetails.exception?.description); throw error;
    }
    if (!Object.hasOwn(reply.result ?? {}, 'value')) throw new Error('MP-11: mirror observation unavailable');
    return reply.result.value;
  }
  // A closed subscription's observers stop (best effort: a gone world has none).
  // MP-11: nothing built under the old protection (early packet, cached replies, resend history) reaches the viewer.
  invalidate(stream) { stream.policy = null; stream.early = null; stream.replies = null; stream.history = null; stream.historyBytes = 0; stream.resources.clear(); }
  dispose(stream) {
    this.regions.retireStream(stream);
    stream.deflate?.close(); stream.deflate = null; this.css.release(stream.id);
    const drop = `globalThis.__charioxMirror2?.drop(${JSON.stringify(String(stream.id))})`;
    for (const world of [stream.world, ...[...stream.frames.values()].map(entry => entry.child)]) if (world) world.connection.send('Runtime.evaluate', { expression: drop, contextId: world.contextId, returnByValue: true }, world.sessionId).catch(() => {});
  }
  stream(stream) {
    this.host.timing?.(`mirror2_subscribe ${this.service.streams.size}`, timestamp());
    Object.assign(stream, { wire: 2, issued: 0, resetAt: 0, chain: Promise.resolve(), resources: new Map(), attrSequence: new Map(), tilesAt: 0, tileKeys: '', fallback: null, frames: new Map(), frameSlots: new Map(), frameSlot: 0, movedAt: 0, ownScrolls: new Map() });
    return stream;
  }
  // Closed shadow roots (owner decision: opaque regions): inspect only hosts
  // in this chunk before it can be published.
  async markClosedHosts(world, source) { return discoverClosedHosts(this, world, source); }
  // Registered Vault targets are marked by node identity in the observer's world.
  // Fields the kernel filled from the Vault, by node identity: top-level refs
  // are `backend:N`, child-frame refs `frame:<id>:<loader>:backend:N` (prefix).
  async protectTargets(world, tab, policy, prefix = '', patience) {
    await this.evaluate(world, `${world.ref}.resetTargets()`, false, patience?.());
    for (const target of policy.targets) {
      if (target.kind !== 'browser' || target.target_id !== tab.target_id || typeof target.node_ref !== 'string') continue;
      if (prefix ? !target.node_ref.startsWith(prefix) : target.node_ref.startsWith('frame:') || target.document_id && target.document_id !== tab.document_id) continue;
      const match = /^backend:([1-9][0-9]*)$/.exec(target.node_ref.slice(prefix.length));
      if (!match) continue;
      try {
        const { object } = await world.connection.send('DOM.resolveNode', { backendNodeId: Number(match[1]), executionContextId: world.contextId }, world.sessionId);
        const marked = await world.connection.send('Runtime.callFunctionOn', { objectId: object.objectId, functionDeclaration: `function(){return ${world.ref}.protect.call(this,${JSON.stringify(target.value_hash ?? null)},${JSON.stringify(target.fill_revision ?? null)})}`, returnByValue: true }, world.sessionId, { timeoutMs: world.patience?.() });
        if (marked.exceptionDetails || marked.result?.value !== true) throw new Error('MP-11: protected target unavailable');
        await world.connection.send('Runtime.releaseObject', { objectId: object.objectId }, world.sessionId).catch(() => {});
      } catch (error) {
        if (/No node with given id|Could not find node/.test(error?.message ?? '')) continue; // departed node: no pixels
        throw error;
      }
    }
  }
  /** BROWSER-UX-05: the first (reset) packet is built at registration, overlapping the viewer's round trip
   * for its first credit; that credit takes the packet in flight instead of starting one. */
  begin(stream, scope, generation) {
    const early = this.next(stream, { subscription_id: stream.id, generation, after_sequence: 0, wait_ms: 0 }, scope);
    stream.early = early; stream.earlyPolicy = this.host.protection;
    early.catch(() => { if (stream.early === early) stream.early = null; });
  }
  async next(stream, command, scope, { signal } = {}) {
    if (command.known_sheets !== undefined && (!Array.isArray(command.known_sheets) || command.known_sheets.length > 128 || command.known_sheets.some(digest => typeof digest !== 'string' || !/^[a-f0-9]{24}$/.test(digest)))) throw new Error('MP-11: invalid mirror sheet dictionary');
    if (command.wait_ms !== undefined && (!Number.isInteger(command.wait_ms) || command.wait_ms < 0 || command.wait_ms > MAX_WAIT_MS)) throw new Error('MP-11: invalid mirror wait');
    // Credits are answered in order; each packet's base is its predecessor.
    // A queued credit's wait starts when its predecessor is answered (expired
    // credits do not come back together: one heartbeat per wait), bounded from
    // its arrival so it never outlives the client's 8 s request timeout.
    // A credit replayed after a reconnect (same command id) gets its original
    // packet: running it again would issue a new sequence and leave a gap. The
    // viewer is back, so a still-waiting original stops its long poll now.
    const replayKey = typeof command.command_id === 'string' && command.command_id.length <= 128 ? command.command_id : null;
    const replayed = replayKey && stream.replies?.get(replayKey);
    if (replayed) { if (!replayed.done) this.hurry(stream); return replayed.run; }
    // Protocol 501: a resend credit (the viewer saw a gap, or a credit failed with its socket) wants the packet
    // after its base now: from history when it was issued (its reply lost), else once earlier long polls, which
    // it ends, have answered (a failed credit's orphan issues it), else it is an ordinary credit.
    if (command.flush !== undefined && command.flush !== true) throw new Error('MP-11: invalid mirror flush');
    if (command.flush && command.wait_ms !== 0) throw new Error('MP-11: invalid mirror flush wait');
    if (command.resend !== undefined && command.resend !== true) throw new Error('MP-11: invalid mirror resend');
    const resend = command.resend === true && command.after_sequence > 0;
    const lost = this.lost(stream, command, scope, resend);
    if (lost) return lost;
    // MP-11: a packet built before a protection change never reaches the viewer.
    if (stream.early && stream.earlyPolicy !== this.host.protection) stream.early = null;
    if (stream.early && command.after_sequence === 0 && command.resend === undefined) {
      const run = stream.early; stream.early = null;
      if (replayKey) { const entry = { run, done: false }; run.finally(() => { entry.done = true; }).catch(() => {}); (stream.replies ??= new Map()).set(replayKey, entry); }
      return run;
    }
    const arrival = Date.now(), wait = command.wait_ms ?? 0;
    // A reset credit (lost base) must not queue behind a waiting long poll.
    if (command.after_sequence === 0 || resend || command.flush) this.hurry(stream);
    const hurry = stream.hurry;
    const run = stream.chain.then(() => resend && this.lost(stream, command, scope, true) || this.packet(stream, command, scope, signal, Math.min(arrival + 3 * wait, Date.now() + wait), hurry));
    stream.chain = run.catch(() => {});
    if (replayKey) { const entry = { run, done: false }; run.finally(() => { entry.done = true; }).catch(() => {}); (stream.replies ??= new Map()).set(replayKey, entry); if (stream.replies.size > 8) stream.replies.delete(stream.replies.keys().next().value); }
    return run;
  }
  kept(stream, generation) {
    const stored = this.host.tabs?.get(stream.tab_id), world = stream.world, browser = this.host.browser;
    if (!stored || !world || generation !== this.host.generation || world.connection !== browser.connection || !world.connection.isOpen()) return null;
    const document = browser.documentIdsByTarget?.get(stored.target_id);
    return document === stored.document_id && world.frame?.loaderId === document && browser.sessionsByTarget?.get(stored.target_id) === world.sessionId ? { tab: { ...stored }, world } : null;
  }
  // Credits that arrived before this point stop waiting (their hurry count is older).
  hurry(stream) { stream.hurry = (stream.hurry ?? 0) + 1; if (stream.world) this.evaluate(stream.world, `${stream.world.ref}.wake()`).catch(() => {}); }
  // The viewer keeps its native preview of a document until the DOM mirror can be built. This reset only says "still preparing".
  previewPacket(stream, command, scope, tabId, documentId, started) {
    if (this.service.require(command.subscription_id, scope, command.generation) !== stream || this.host.generation !== command.generation) throw new Error('MP-11: stale mirror subscription');
    const sequence = ++stream.issued;
    stream.resetAt = sequence + 1; // old page epochs can no longer dispatch
    const packet = { wire: 2, subscription_id: command.subscription_id, tab_id: tabId, generation: command.generation,
      document_id: documentId, sequence, base_sequence: null, reset: true, fallback: 'navigation_preview',
      snapshot_complete: false, css_width: 1280, css_height: 800, device_scale_factor: this.host.scales.get(tabId) ?? 1,
      scroll: [0, 0], focused: null, selection: null, resources: [], tiles: [] };
    this.remember(stream, sequence, packet, true);
    this.host.timing?.('mirror2_navigation_preview', started);
    return packet;
  }
  // A renderer that stays busy past the bounded wait is not an observer verdict: while a native preview covers the document
  // the viewer is told to keep waiting (the credit stays on this subscription) instead of being sent to a video fallback.
  async packet(stream, command, scope, signal, deadline, hurry) {
    try { return await this.build(stream, command, scope, signal, deadline, hurry); }
    catch (error) {
      if (error?.code !== 'browser_cdp_timeout' || !stream.navigationPreview || !stream.previewDocument || stream.previewDocument === stream.document_id) throw error;
      return this.previewPacket(stream, command, scope, stream.tab_id, stream.previewDocument, timestamp());
    } finally { if (stream.world) stream.world.patience = null; }
  }
  async build(stream, command, scope, signal, deadline = Date.now(), hurry = stream.hurry) {
    const started = timestamp(); let stage = started;
    const mark = name => { this.host.timing?.(`mirror2_${name}`, stage); stage = timestamp(); };
    const busyMs = this.busyMs ?? BUSY_RENDERER_MS, patience = () => Math.max(Math.min(1000, busyMs), busyMs - (timestamp() - started));
    const delta = () => Math.max(1000, Math.min(busyMs, CREDIT_PATIENCE_MS) - (timestamp() - started));
    this.service.require(command.subscription_id, scope, command.generation);
    // MP-08/MP-10: between packets the page session's navigation events keep the document current, so an
    // unchanged document reuses its world without renderer round trips (a busy page answers those only after
    // its task, holding the next long poll and its echo behind it). Anything else resolves afresh.
    const kept = this.kept(stream, command.generation);
    const tab = kept?.tab ?? await (this.host.mirrorTarget ?? this.host.displayTarget).call(this.host, { tab_id: stream.tab_id, generation: command.generation });
    assertNotCancelled(signal);
    // MP-08/MP-10/MP-11: negotiate native preview before any observer/CSS work.
    // It is an ordinary protected display, never a raster in the mirror packet.
    const previousDocument = stream.document_id ?? stream.initialDocument;
    if (stream.navigationPreview && previousDocument && stream.previewDocument !== tab.document_id && previousDocument !== tab.document_id) {
      stream.previewDocument = tab.document_id;
      this.regions.retireStream(stream);
      return this.previewPacket(stream, command, scope, tab.tab_id, tab.document_id, started);
    }
    if(stream.navigationPreview&&stream.previewDocument===tab.document_id&&stream.document_id!==tab.document_id&&stream.previewWaitedDocument!==tab.document_id){
      const previewAt=timestamp(),document=tab.document_id;
      await waitForNavigationPreview(this.host,{tab_id:tab.tab_id,document_id:document,generation:command.generation,scope},signal);
      assertNotCancelled(signal);this.service.require(command.subscription_id,scope,command.generation);
      if(this.host.generation!==command.generation||tab.document_id!==document)throw new Error('MP-11: navigation preview document retired');
      stream.previewWaitedDocument=document;this.host.timing?.('mirror2_preview_wait',previewAt);
    }
    const browser = this.host.browser;
    const world = kept?.world ?? await this.world(tab, stream, patience), policy = this.host.protection;
    stream.world = world;
    if (world.cssFailed) throw new Error('MP-11: mirror CSS observation unavailable');
    mark('world');
    const after = command.after_sequence;
    // The viewer's credits name what it applied: resource completion counts only once acknowledged.
    if (after > 0 && after <= stream.issued) stream.acked = Math.max(stream.acked ?? 0, after);
    // Bytes the viewer acknowledged are kept by it for the document's lifetime: the kernel drops its copy.
    if (stream.acked > (stream.prunedAt ?? 0)) { stream.prunedAt = stream.acked; for (const entry of stream.resources.values()) if (entry.sent && entry.sentAt <= stream.acked && entry.resource) { entry.resource = entry.preview = null; entry.url = ''; } }
    // A viewer whose base predates the newest reset has that reset on its way (a large one crossing a slow link
    // while small credits are answered): a second reset only queues behind it (r47: a 420 KB reset storm).
    const resetReason = after === 0 ? 'client' : after > stream.issued ? 'ahead' : after < stream.issued - 8 && after >= stream.resetAt ? 'behind' : stream.document_id !== tab.document_id ? 'document' : stream.policy !== policy ? 'policy' : stream.resetAt > stream.issued ? 'snapshot' : stream.fallback !== null ? 'fallback' : null;
    let reset = resetReason !== null;
    // A snapshot waits for a busy renderer in every call it makes (the viewer's reset has 30 s). Any other credit waits
    // somewhat longer than the default, but answers before the viewer's own 8 s request timeout.
    world.patience = reset ? patience : delta;
    if (reset) this.host.timing?.(`mirror2_reset ${resetReason}`, started);
    // The hold follows the document transition itself: a credit that also resets (a resynchronizing viewer)
    // still keeps the previous page. A new subscription has no previous page to keep.
    // A document the native preview already covers needs no hold: the viewer keeps the preview until its own handover gate.
    if (stream.previewDocument !== tab.document_id && (stream.document_id && stream.document_id !== tab.document_id || !stream.document_id && world.holdInitialPaint)) { await this.evaluate(world, paintHoldExpression(Math.max(0, PAINT_HOLD_MS - (world.navigatedAt ? timestamp() - world.navigatedAt : 0))), true).catch(() => {}); mark('paint_hold'); assertNotCancelled(signal); }
    if (stream.document_id !== tab.document_id) { stream.acked = 0; stream.prunedAt = 0; stream.resources.clear(); stream.attrSequence.clear(); stream.ownScrolls.clear(); stream.tilesAt = 0; stream.loadedCount = -1; }
    // Rebased ids/keys bound the frame slot; a long session re-snapshots instead.
    if (stream.frameSlot >= 900) reset = true;
    // Frame slots restart with a snapshot, so rebased child keys are re-learned.
    // A slice in flight may be lost with the base: a partial resource restarts.
    let source, fallback = null;
    const moved = () => { const live = this.host.browser.documentIdsByTarget?.get(tab.target_id); return !!live && live !== tab.document_id; };
    // Ordered navigation events retire this credit before any old-world retry.
    // A renderer query here would queue behind the newly loading document.
    const assertDocument = () => {
      if (this.host.browser !== browser || world.connection.isOpen?.() === false ||
          'connection' in browser && browser.connection !== world.connection ||
          browser.sessionsByTarget && browser.sessionsByTarget.get(tab.target_id) !== world.sessionId || moved())
        throw new BrowserActionError('stale_document_reference', 'browser mirror document retired');
    };
    const read = async () => {
      assertDocument();
      if (reset) {
        // A snapshot may reuse local ids. Refuse old inputs/region leases
        // before touching the observer, including resyncs found during drain.
        // If cancelled, resetAt > issued forces the next credit to rebuild.
        stream.resetAt = stream.issued + 1;
        stream.ownScrolls.clear(); stream.frames.clear(); stream.frameSlots.clear(); stream.frameSlot = 0;
        for (const [key, entry] of stream.resources) if (entry.slot) stream.resources.delete(key); else if (!entry.sent || entry.sentAt > (stream.acked ?? 0)) { entry.sent = false; entry.offset = 0; }
        if (world.snapshotPending?.policy !== policy) await this.protectTargets(world, tab, policy, '', patience);
        const snapshotAt = timestamp();
        const snap = await this.snapshotOnce(world, policy, patience);
        this.host.timing?.('mirror2_snapshot', snapshotAt);
        if (snap.resync) return snap;
        await this.markClosedHosts(world, snap);
        // Sheets this CSSOM cannot read come through CDP.
        const sheetsAt = timestamp();
        const sheetOps = await this.sheetOps(world, snap.sheets, true);
        // A pruned sheet's full text follows on a quiet packet; a viewer that retains it gets it by reference now.
        const known = new Set(command.known_sheets ?? []);
        stream.deferredSheets = [];
        for (const op of sheetOps) if (op.full) { if (known.has(sheetDigest(op.full))) op.css = op.full; else stream.deferredSheets.push({ op: 'css', id: op.id, css: op.full }); delete op.full; }
        stream.deferredAt = Date.now(); stream.fullPending = snap.full_pending ?? 0;
        this.host.timing?.('mirror2_sheets', sheetsAt);
        const framesAt = timestamp();
        // Cross-origin frames: child DOM under the owner, or an opaque region.
        const frames = await this.frames.attach(world, stream, snap.nodes.filter(r => r.foreign).map(r => r.id), policy, tab);
        this.host.timing?.('mirror2_frames', framesAt);
        this.opaque(snap.nodes, frames.opaque);
        return this.frames.project(stream, { ...snap, sheets: [], nodes: [...snap.nodes, ...frames.records], ops: this.viewerOps(stream, [...(snap.ops ?? []), ...frames.ops, ...sheetOps]), resources: [...snap.resources, ...(frames.resources ?? [])] });
      }
      return processDelta(await this.evaluate(world, `${world.ref}.drain()`));
    };
    const processDelta = async delta => {
      assertDocument();
      if (Number.isInteger(delta.full_pending)) stream.fullPending = delta.full_pending;
      // New/upgraded custom hosts trigger trusted discovery before their final projection. Steady-state credits add no round trip.
      if (delta.closed_candidates && await this.markClosedHosts(world, delta)) {
        delta = this.merge(delta, await this.evaluate(world, `${world.ref}.drain()`));
      }
      if (delta.resync) { this.host.timing?.(`mirror2_reset drain_${delta.resync}`, started); return delta; }
      const ops = [], resources = [...delta.resources];
      if (JSON.stringify(delta.scroll) !== stream.pageScroll) { stream.pageScroll = JSON.stringify(delta.scroll); stream.movedAt = Date.now(); }
      for (const op of delta.ops) {
        if (op.op === 'scroll' && stream.ownScrolls.get(op.id) === JSON.stringify(op.scroll)) continue; // this viewer's own scroll
        if (op.op === 'children') {
          const frames = await this.frames.attach(world, stream, op.nodes.filter(r => r.foreign).map(r => r.id), policy, tab);
          this.opaque(op.nodes, frames.opaque);
          ops.push({ ...op, nodes: [...op.nodes, ...frames.records] }, ...frames.ops); resources.push(...(frames.resources ?? []));
        } else ops.push(op);
      }
      const children = await this.frames.drain(stream, policy, tab);
      return this.frames.project(stream, { ...delta, ops: this.viewerOps(stream, [...ops, ...children.ops]), resources: [...resources, ...children.resources], changed: [...delta.changed, ...children.changed] });
    };
    let resources = [];
    try {
      source = await read();
      assertDocument();
      if (source.resync) { reset = true; source = await read(); }
      this.register(stream, source);
      // A pointer press's focus/selection alone waits for the rest of its click (the page's next frame).
      if (!reset && source.pressed > 0 && !source.ops?.length) {
        const more = await processDelta(await this.evaluate(world, `${world.ref}.waitDrain(${source.pressed})`, true));
        if (more.resync) { reset = true; source = await read(); this.register(stream, source); } else { this.register(stream, more); source = this.merge(source, more); }
      }
      // An opted-in viewer can prepare DOM/CSS before the font bytes arrive;
      // its positive native-font requirements still fence the paint handover.
      // Legacy viewers retain first-packet fonts. Resource bytes otherwise
      // travel only without DOM changes, so an echo never queues behind them.
      const fontsAt = timestamp();
      resources = reset ? stream.fontRequirements === true ? [] : await this.materialize(world, stream, 'font', source.loaded) : this.empty(stream, source) ? await this.materialize(world, stream, null, source.loaded) : [];
      if (reset) this.host.timing?.('mirror2_fonts', fontsAt);
      // Long poll: nothing to send yet -> wait for the page (or newly loaded bytes).
      // A navigation ends the wait at once (MP-08/MP-10): the next credit resets to the new document.
      while (!reset && !resources.length && this.empty(stream, source) && !this.sheetsDue(stream) && Date.now() < deadline && stream.hurry === hurry && !moved()) {
        const settle = stream.movedAt + SETTLE_MS - Date.now(), refine = settle > 0 && this.refinePending(stream) ? settle + 1 : 500;
        const more = await processDelta(await this.evaluate(world, `${world.ref}.waitDrain(${Math.max(1, Math.min(500, refine, deadline - Date.now()))})`, true));
        assertNotCancelled(signal);
        if (more.resync) { reset = true; source = await read(); this.register(stream, source); break; }
        this.register(stream, more); source = this.merge(source, more);
        if (this.empty(stream, source)) resources = await this.materialize(world, stream, null, more.loaded);
      }
    } catch (error) {
      assertDocument();
      // A renderer that does not answer is busy, not an observer verdict: no video fallback and no further renderer fence.
      // Whatever it was asked may still run, so the next credit rebuilds the snapshot.
      if (error?.code === 'browser_cdp_timeout') { stream.resetAt = stream.issued + 1; throw error; }
      await assertCurrentDocument(world.connection, world.sessionId, tab.target_id, tab.document_id);
      // Over-budget/unavailable DOM: the client shows the protected video path.
      fallback = error.mirrorReason ?? 'observer_unavailable';
      this.host.timing?.(`mirror2_fallback_${fallback}`, timestamp());
      source = null; reset = true;
    }
    if (fallback) resources = [];
    mark('observe_resources');
    assertDocument();
    const sheets = await this.sheetOps(world, source?.sheets);
    // A pruned first-paint sheet's full text follows without viewer input, once the page is quiet (or after 1 s).
    if (!reset && !fallback) {
      const replaced = new Set([...source.ops ?? [], ...sheets].filter(op => op.op === 'css').map(op => op.id));
      stream.deferredSheets = (stream.deferredSheets ?? []).filter(op => !replaced.has(op.id));
      if (!resources.length && this.sheetsDue(stream) && (this.emptyDom(stream, source) || Date.now() - stream.deferredAt > 1000)) {
        let full = stream.deferredSheets.shift();
        if (!full) { stream.fullPending--; full = await this.evaluate(world, `${world.ref}.fullSheet()`); }
        if (full && !replaced.has(full.id)) sheets.push({ op: 'css', id: full.id, css: full.css });
      }
    }
    let tiles = [];
    // Stills, like image bytes, never ride on a DOM change or follow viewer input
    // within QUIET_MS: an echo must not queue behind a lossless region.
    const still = reset || (this.emptyDom(stream, source) && !this.recentInput(stream));
    if (!fallback) try { tiles = await this.tiles(world, tab, stream, reset, still, source?.opaque === false); } catch (error) {
      if (!['region_area', 'region_transform'].includes(error.mirrorReason)) throw error;
      fallback = error.mirrorReason; this.host.timing?.(`mirror2_fallback_${fallback}`, timestamp()); source = null; reset = true; resources = [];
    }
    mark('tiles');
    // The page session's navigation events arrive in order with the replies above: a cached document equal to the
    // tab's needs no renderer round trip (which waits for the page's next task ahead of an echo). Any other
    // answer asks the renderer.
    assertDocument();
    if (this.host.browser.documentIdsByTarget?.get(tab.target_id) !== tab.document_id) await assertCurrentDocument(world.connection, world.sessionId, tab.target_id, tab.document_id);
    assertNotCancelled(signal);
    if (this.host.protection !== policy || this.host.generation !== command.generation || this.service.streams.get(command.subscription_id) !== stream) {
      stream.policy = null; throw new Error('MP-11: stale mirror protection policy or subscription');
    }
    const sequence = stream.issued + 1;
    // Protocol 489 compact delta: the binding, base (sequence - 1) and every
    // header field equal to the base packet's are implied; empty lists omitted.
    const header = { snapshot_complete: source?.snapshot_complete === true, font_failures: source?.font_failures ?? [], scroll: source?.scroll ?? [0, 0], focused: source?.focused ?? null, selection: source?.selection ?? null, fragment: source?.fragment ?? null }, sent = Object.fromEntries(Object.entries(header).map(([name, value]) => [name, JSON.stringify(value)]));
    const packet = reset ? { wire: 2, subscription_id: command.subscription_id, tab_id: tab.tab_id, generation: command.generation, document_id: tab.document_id,
      sequence, base_sequence: null, reset, css_width: 1280, css_height: 800, device_scale_factor: this.host.scales.get(tab.tab_id) ?? 1, ...header }
      : { wire: 2, sequence, ...Object.fromEntries(Object.entries(header).filter(([name]) => sent[name] !== stream.header?.[name])) };
    if (resources.length) packet.resources = resources;
    this.host.timing?.event?.('mirror2_resource_batch', { reset: Number(reset), font_base64_bytes: resources.filter(r => r.mime_type.startsWith('font/')).reduce((n, r) => n + r.data_base64.length, 0), other_base64_bytes: resources.filter(r => !r.mime_type.startsWith('font/')).reduce((n, r) => n + r.data_base64.length, 0) });
    if (tiles.length) packet.tiles = tiles;
    if (fallback) packet.fallback = fallback;
    else if (reset) { packet.root = source.root; packet.nodes = source.nodes; packet.ops = [...(source.ops ?? []), ...sheets]; }
    else packet.ops = [...source.ops, ...sheets];
    // Morphed nodes (a new page node under a viewer id) count as changed targets.
    for (const id of [...(packet.ops ?? []).filter(op => op.op === 'attr').map(op => op.id), ...(reset ? [] : source?.changed ?? [])]) stream.attrSequence.set(id, sequence);
    // Text/key targets: when each node reached the viewer, and when its identity (morph, type,
    // editability) last changed. Ordinary attribute churn (aria-* per keystroke) does not refuse typing.
    if (reset) { stream.introduced = new Map(); stream.identity = new Map(); }
    for (const record of [...(packet.nodes ?? []), ...(packet.ops ?? []).flatMap(op => op.op === 'children' ? op.nodes : [])]) if (!stream.introduced.has(record.id)) stream.introduced.set(record.id, sequence);
    for (const id of [...(packet.ops ?? []).filter(op => op.op === 'attr' && TARGET_ATTRIBUTES.has(op.name)).map(op => op.id), ...(reset ? [] : source?.changed ?? [])]) stream.identity.set(id, sequence);
    this.regions.record(stream, packet);
    if (reset) stream.sheetRefs = new Set(command.known_sheets ?? []);
    dedupeMirrorSheets(packet, stream.sheetRefs ??= new Set());
    if (packet.nodes) packet.nodes = encodeMirrorRecords(packet.nodes, null);
    if (packet.ops) packet.ops = packet.ops.map(op => op.op === 'children' ? { ...op, nodes: encodeMirrorRecords(op.nodes, op.id) } : op);
    stream.issued = sequence; stream.document_id = tab.document_id; stream.policy = policy; stream.fallback = fallback;
    if (reset) { stream.resetAt = sequence; stream.attrSequence.clear(); }
    stream.header = sent; stream.sentScroll = header.scroll;
    // A position the viewer did not set has reached it: echoes count again.
    if (sent.scroll !== stream.ownScrolls.get(null)) stream.ownScrolls.delete(null);
    for (const op of packet.ops ?? []) if (op.op === 'scroll') stream.ownScrolls.delete(op.id);
    const wire = await this.encode(stream, packet);
    // MP-11: a protection change during the asynchronous deflate retired this packet (invalidate cleared the caches).
    if (this.host.protection !== policy || stream.policy !== policy || this.service.streams.get(command.subscription_id) !== stream) {
      stream.policy = null; throw new Error('MP-11: stale mirror protection policy or subscription');
    }
    this.remember(stream, sequence, wire, reset);
    mark('serialize'); this.host.timing?.('mirror2_total', started);
    return wire;
  }
  // Issued packets by sequence (bounded), for lost-reply resends.
  remember(stream, sequence, wire, reset) {
    if (reset || !stream.history) { stream.history = new Map(); stream.historyBytes = 0;  }
    const bytes = Buffer.byteLength(JSON.stringify(wire)); stream.history.set(sequence, { wire, bytes }); stream.historyBytes += bytes;
    this.host.timing?.event?.('mirror2_packet', { sequence, reset: Number(reset), bytes, raw_bytes: wire.packet_bytes ?? bytes });
    for (const [old, entry] of stream.history) { if (stream.history.size <= 16 && stream.historyBytes <= HISTORY_BYTES || old === sequence) break; stream.history.delete(old); stream.historyBytes -= entry.bytes; }
  }
  // A resend (protocol 501) names the packet it lacks: the one after its base, without the in-flight window.
  lost(stream, command, scope, resend = false) {
    if (!resend) return null;
    const after = command.after_sequence;
    if (!Number.isSafeInteger(after) || after <= 0 || after >= stream.issued || this.service.streams.get(command.subscription_id) !== stream) return null;
    const next = after + 1, entry = stream.history?.get(next);
    if (!entry) return null;
    this.service.require(command.subscription_id, scope, command.generation);
    this.host.timing?.('mirror2_resend', timestamp());
    return entry.wire;
  }
  // Protocol 489: the body (without media) deflates in the subscription's
  // context, so repeated structure (typeahead rows, form ops) costs a few bytes.
  // The kernel issues packets in order and the viewer inflates them in order;
  // a reset packet starts a fresh context. A replayed credit gets these bytes.
  async encode(stream, packet) {
    const { resources, tiles, ...body } = packet, text = Buffer.from(JSON.stringify(body));
    if (packet.reset) { stream.deflate?.close(); stream.deflate = null; }
    if (text.length < DEFLATE_MIN_BYTES) return packet;
    // MP-08/MP-10: level 9 repeatedly searches the large continuation child lists.
    // Level 6 retains the same stream/dictionary with bounded CPU per credit.
    if (!stream.deflate) { const created = stream.deflate = createDeflateRaw({ level: 6, memLevel: 9 }); created.chunks = []; created.on('data', chunk => created.chunks.push(chunk)).on('error', () => {}); }
    const deflate = stream.deflate, deflateAt = timestamp();
    try {
      await new Promise((resolve, reject) => { deflate.once('error', reject); deflate.write(text); deflate.flush(zlib.Z_SYNC_FLUSH, () => { deflate.off('error', reject); resolve(); }); });
    } catch (error) { deflate.close(); if (stream.deflate === deflate) stream.deflate = null; throw error; }
    this.host.timing?.('mirror2_deflate', deflateAt);
    const compressed = Buffer.concat(deflate.chunks); deflate.chunks = [];
    return { wire: 2, sequence: packet.sequence, ...(packet.reset ? { reset: true } : {}), encoding: 'deflate', packet_bytes: text.length, packet_base64: compressed.toString('base64'), ...(resources ? { resources } : {}), ...(tiles ? { tiles } : {}) };
  }
  // The viewer's own scroll position coming back is not news to that viewer.
  empty(stream, source) { return this.emptyDom(stream, source) && !this.tilesDue(stream); }
  emptyDom(stream, source) {
    const scroll = JSON.stringify(source.scroll) === stream.ownScrolls.get(null) ? stream.sentScroll : source.scroll;
    return !source.ops?.length && !source.sheets?.length && JSON.stringify(source.snapshot_complete === true) === stream.header?.snapshot_complete && JSON.stringify(source.font_failures ?? []) === stream.header?.font_failures && JSON.stringify(scroll) === stream.header?.scroll && JSON.stringify(source.focused) === stream.header.focused && JSON.stringify(source.selection) === stream.header.selection && JSON.stringify(source.fragment ?? null) === stream.header.fragment;
  }
  // A packet's resource budget; smaller within a second of viewer input.
  packetBytes(stream) { return this.recentInput(stream) ? INPUT_PACKET_BYTES : RESOURCE_PACKET_BYTES; }
  sheetsDue(stream) { return (stream.deferredSheets?.length > 0 || stream.fullPending > 0) && !this.recentInput(stream); }
  recentInput(stream) { return Date.now() - (stream.inputAt ?? 0) < QUIET_MS; }
  refinePending(stream) { for (const entry of stream.resources.values()) if (entry.previewed && !entry.sent) return true; return false; }
  merge(a, b) { return { ...b, ops: [...(a.ops ?? []), ...(b.ops ?? [])], sheets: [...(a.sheets ?? []), ...(b.sheets ?? [])], resources: [...(a.resources ?? []), ...(b.resources ?? [])], changed: [...(a.changed ?? []), ...(b.changed ?? [])] }; }
  // Unattached foreign frames are opaque regions (masked captures only).
  opaque(records, ids) {
    for (const record of records) if (record.foreign) { delete record.foreign; if (ids.includes(record.id)) { record.kind = 'tile'; record.reason = 'cross_origin_frame'; } }
  }
  // Resource bytes are kept by the renderer for the document's lifetime: a
  // same-document reset (lost base) does not send them again.
  register(stream, source) {
    for (const descriptor of source?.resources ?? []) if (!stream.resources.has(descriptor.key)) stream.resources.set(descriptor.key, { ...descriptor, state: 'new', tries: 0, sent: false });
  }
  tilesDue(stream) { return (stream.tileKeys !== '' || stream.tilesPending) && Date.now() - stream.tilesAt >= TILE_REFRESH_MS && !this.recentInput(stream); }
  // Bytes the page itself loaded: data URLs, the inspector's resource tree, or
  // Chrome's cache (credential-free) for URLs in the page's Resource Timing.
  // A URL a stylesheet merely mentions waits until the page loads it.
  // MP-08/MP-10: the drain just taken reports the main document's count; a separate evaluate would wait for a
  // busy page's next task before the long poll re-arms.
  async materialize(main, stream, kind, drained) {
    const worlds = new Map([[0, main], ...[...stream.frameSlots].map(([slot, entry]) => [slot, entry.child])]);
    let counts = 0; for (const [slot, world] of worlds) counts += !slot && Number.isSafeInteger(drained) ? drained : await this.evaluate(world, `${world.ref}.loadedCount()`).catch(() => 0);
    const recheck = counts !== stream.loadedCount; stream.loadedCount = counts;
    const out = [];
    // All document fonts precede images, including a child document's fonts
    // when main-document images alone would consume the whole packet budget.
    for (const pass of kind ? [kind] : ['font', null]) for (const [slot, world] of worlds) {
      const remaining = this.packetBytes(stream) - out.reduce((n, r) => n + r.data_base64.length, 0);
      if (remaining <= 0) return out;
      const due = [...stream.resources.values()].filter(entry => (entry.slot ?? 0) === slot && (pass ? entry.kind === pass : entry.kind !== 'font') && (entry.state === 'new' || entry.state === 'ok' && !entry.sent || entry.state === 'waiting' && recheck));
      if (due.length) out.push(...await this.materializeWorld(world, due, slot, remaining, stream));
    }
    return out;
  }
  async resourceBody(world, url, maxBytes = RESOURCE_BYTES) {
    return this.host.browser?.responseBodies?.read(world, url, maxBytes) ?? null;
  }
  async materializeWorld(world, due, slot, budget, stream) {
    // Top-document element images: near the view first; the rest wait.
    const fontsFirst = (a, b) => Number(b.kind === 'font') - Number(a.kind === 'font');
    due = [...due].sort(fontsFirst);
    let near = null, moving = false, stamp = null;
    if (!slot && stream && due.some(entry => entry.kind !== 'font')) {
      // The view is re-measured only after a scroll, a packet or a page load:
      // an idle long poll does not walk the page's images twice a second.
      stamp = `${stream.pageScroll}|${stream.issued}|${stream.loadedCount}|${stream.resources.size}|${Date.now() - stream.movedAt < SETTLE_MS}`;
      if (stamp === stream.nearStamp) return [];
      const view = await this.evaluate(world, `${world.ref}.nearImages(${NEAR_PX})`);
      near = new Map(view.near.map(([key, width, height]) => [key, [width, height]])); const elements = new Set(view.all);
      due = due.filter(entry => !elements.has(entry.key) || near.has(entry.key)).sort((a, b) => fontsFirst(a, b) || near.has(b.key) - near.has(a.key));
      moving = Date.now() - stream.movedAt < SETTLE_MS;
    }
    if (!due.length) { if (stamp) stream.nearStamp = stamp; return []; }
    // A completed response already proves the exact frame/document fetched
    // its bytes. Ask the renderer only on the existing fallback cache miss.
    let loaded;
    const loadedByPage = async () => loaded ??= new Set((await this.evaluate(world, `${world.ref}.loaded()`)).map(key => slot ? `r${slot * 1e6 + Number(key.slice(1))}` : key));
    let tree = null;
    const frames = async () => {
      if (tree) return tree;
      const { frameTree } = await world.connection.send('Page.getResourceTree', {}, world.sessionId);
      tree = new Map(); const collect = node => { for (const r of node.resources ?? []) tree.set(r.url, node.frame.id); for (const child of node.childFrames ?? []) collect(child); }; collect(frameTree);
      return tree;
    };
    const out = []; let bytes = 0;
    for (const entry of due) {
      if (bytes >= budget) break;
      if (entry.state !== 'ok') {
        let body = null;
        if (entry.url.startsWith('data:')) body = dataUrlBytes(entry.url);
        else {
          body = await this.resourceBody(world, entry.url);
          // The response index itself proves this exact frame/document fetched
          // the URL. A cache miss retains the existing Resource Timing/tree gate.
          if (!body) {
            if (!(await loadedByPage()).has(entry.key) && !(await frames()).has(entry.url)) { entry.state = 'waiting'; continue; }
            const frameId = (await frames()).get(entry.url);
            if (frameId) try { const reply = await world.connection.send('Page.getResourceContent', { frameId, url: entry.url }, world.sessionId); body = reply.base64Encoded ? Buffer.from(reply.content, 'base64') : Buffer.from(reply.content, 'utf8'); } catch { body = null; }
            body ??= await this.cached(world, entry.url).catch(() => null);
          }
        }
        let mime_type = body && body.length <= RESOURCE_BYTES ? mirrorResourceType(body, entry.kind === 'sprite' ? 'image' : entry.kind) : null;
        // Protocol 501: a sprite travels as its sanitized symbols (the viewer validates them again).
        if (entry.kind === 'sprite') { const clean = mime_type === 'image/svg+xml' ? await this.evaluate(world, `${world.ref}.sprite(${JSON.stringify(body.toString('utf8'))})`).catch(() => null) : null; body = typeof clean === 'string' ? Buffer.from(clean, 'utf8') : null; if (!body) mime_type = null; }
        if (!mime_type) { entry.tries++; entry.state = entry.tries >= 3 ? 'failed' : 'waiting'; continue; }
        entry.state = 'ok'; entry.resource = { key: entry.key, resource_id: createHash('sha256').update(body).digest('hex'), mime_type, data_base64: body.toString('base64') };
      }
      const size = near?.get(entry.key);
      if (size && moving && entry.resource.data_base64.length > PREVIEW_MIN_BASE64 && PREVIEW_TYPES.has(entry.resource.mime_type)) {
        if (entry.previewed) continue; // exact bytes once the view settles
        // Generated once; admitted only within this packet's remaining budget (else a later packet,
        // or the exact bytes in slices once the view settles).
        const svg = entry.preview ??= await this.evaluate(world, `${world.ref}.preview(${JSON.stringify(entry.resource.data_base64)},${JSON.stringify(entry.resource.mime_type)},${size[0]},${size[1]})`, true).catch(() => null);
        if (typeof svg === 'string') {
          const data = Buffer.from(svg, 'utf8'), size64 = Math.ceil(data.length / 3) * 4;
          if (bytes + size64 > budget) continue;
          entry.previewed = true; entry.preview = null; bytes += size64;
          out.push({ key: entry.key, resource_id: createHash('sha256').update(data).digest('hex'), mime_type: 'image/svg+xml', data_base64: data.toString('base64') });
          continue;
        }
      }
      // Within the budget whole; otherwise the next slice (offset/total in base64 characters).
      const data = entry.resource.data_base64, offset = entry.offset ?? 0, room = budget - bytes;
      let end = data.length;
      if (end - offset > room) { if (room < MIN_SLICE) continue; end = offset + Math.floor(room / 4) * 4; }
      bytes += end - offset; entry.offset = end; entry.sent = end === data.length; if (entry.sent) entry.sentAt = stream.issued + 1; // this packet's sequence (packets are serial)
      out.push(!offset && entry.sent ? entry.resource : { ...entry.resource, data_base64: data.slice(offset, end), offset, total: data.length });
    }
    return out;
  }
  async cached(world, url) {
    const { resource } = await world.connection.send('Network.loadNetworkResource', { frameId: world.frame.id, url, options: { disableCache: false, includeCredentials: false } }, world.sessionId);
    if (!resource?.success || !resource.stream || resource.httpStatusCode && (resource.httpStatusCode < 200 || resource.httpStatusCode >= 300)) { if (resource?.stream) await world.connection.send('IO.close', { handle: resource.stream }, world.sessionId).catch(() => {}); return null; }
    const chunks = []; let size = 0;
    try {
      for (;;) {
        const chunk = await world.connection.send('IO.read', { handle: resource.stream, size: 1 << 20 }, world.sessionId);
        const part = chunk.base64Encoded ? Buffer.from(chunk.data, 'base64') : Buffer.from(chunk.data, 'utf8');
        size += part.length; if (size > RESOURCE_BYTES) return null;
        chunks.push(part); if (chunk.eof) break;
      }
    } finally { await world.connection.send('IO.close', { handle: resource.stream }, world.sessionId).catch(() => {}); }
    return Buffer.concat(chunks);
  }
  // `firstPaint` (a reset): a large sheet travels as the rules the document uses now, its full text as `full`.
  async sheetOps(world, sheets = [], firstPaint = false) {
    const ops = [];
    for (let at = 0; at < sheets.length; at += 4) {
      const batch = await Promise.all(sheets.slice(at, at + 4).map(async sheet => {
        const text = await this.crossOriginSheet(world, sheet.url, firstPaint ? sheet.id : null).catch(() => null);
        if (text === null) return [];
        const [css, used] = Array.isArray(text) ? text : [text, null];
        return [typeof used === 'string' && used.length < css.length ? { op: 'css', id: sheet.id, css: used, full: css } : { op: 'css', id: sheet.id, css }];
      }));
      ops.push(...batch.flat());
    }
    return ops;
  }
  // Cross-origin sheets: CDP reads the text regardless of CORS; the page's
  // CSSOM in the isolated world normalizes it, then the same sanitizer runs.
  // A constructed sheet drops @import: imports the page already loaded are read
  // the same way (depth-first, bounded), each under its conditions and sanitized
  // against its own URL, after the importing sheet's leading layer statements.
  async crossOriginSheet(world, url, usedBy = null) {
    let frameOf;
    const frames = async () => {
      if (!frameOf) {
        const { frameTree } = await world.connection.send('Page.getResourceTree', {}, world.sessionId);
        frameOf = new Map(); const collect = node => { for (const r of node.resources ?? []) frameOf.set(r.url, node.frame.id); for (const child of node.childFrames ?? []) collect(child); }; collect(frameTree);
      }
      return frameOf;
    };
    const sheets = []; let bytes = 0;
    const read = async (href, open, close, depth) => {
      if (sheets.length >= 64) return false;
      const body = await this.resourceBody(world, href, 16 * 1024 * 1024);
      let text = body?.toString('utf8');
      if (text === undefined) {
        const frameId = (await frames()).get(href); if (!frameId) return false;
        const reply = await world.connection.send('Page.getResourceContent', { frameId, url: href }, world.sessionId);
        text = reply.base64Encoded ? Buffer.from(reply.content, 'base64').toString('utf8') : reply.content;
      }
      if (typeof text !== 'string' || (bytes += text.length) > 16 * 1024 * 1024) return false;
      const { layers, imports } = cssImports(text);
      // In source order: the sheet's leading layer statements, its imports, then its own rules.
      if (layers) sheets.push({ text: layers, url: href, open, close });
      if (depth < 8) for (const rule of imports) {
        let next; try { next = new URL(rule.url, href).href; } catch { continue; }
        await read(next, open + rule.open, rule.close + close, depth + 1).catch(() => false);
      }
      sheets.push({ text, url: href, open, close });
      return true;
    };
    if (!await read(url, '', '', 0)) return null;
    return this.evaluate(world, `(sheets=>{const parse=text=>{const sheet=new CSSStyleSheet();sheet.replaceSync(text);let out='';for(const rule of sheet.cssRules)out+=rule.cssText+'\\n';return out};let out='';for(const s of sheets){let body=parse(s.text);if(s.open)body=parse(s.open+body+s.close);out+=${world.ref}.sanitize(body,s.url)}return ${usedBy === null ? 'out' : `out.length<${FIRST_PAINT_SHEET}?out:[out,${world.ref}.usedCss(${JSON.stringify(usedBy)},out)]`}})(${JSON.stringify(sheets)})`);
  }
  // Opaque regions as lossless stills, refreshed at most once a second until phase 3
  // binds them to video rows: a canvas or video as its own pixels; plugins, closed
  // roots, unattached frames and tainted media as viewport screenshot crops, which
  // a transformed or filtered region cannot use (labelled video fallback).
  async regionBoxes(stream, tab, signal) {
    assertNotCancelled(signal);
    const world = stream.world;
    if (!world || world.connection !== this.host.browser.connection || !world.connection.isOpen()) throw new Error('MP-11: region document unavailable');
    await assertCurrentDocument(world.connection, world.sessionId, tab.target_id, tab.document_id);
    await this.protectTargets(world, tab, this.host.protection);
    const boxes = [...(await this.evaluate(world, `${world.ref}.regionBoxes()`)).filter(b => !stream.frames.has(b.id)), ...await this.frames.regionBoxes(stream, world, tab, this.host.protection)];
    await assertCurrentDocument(world.connection, world.sessionId, tab.target_id, tab.document_id);
    assertNotCancelled(signal); return boxes;
  }
  async tiles(world, tab, stream, reset, still, none = false) {
    // No opaque region in the page and no attached frame: nothing to place or capture. A DOM echo does not wait
    // for an evaluation in the page's next task (MP-08 click path, 8-12 ms behind a busy source page).
    if (none && !stream.frames.size) { stream.tileKeys = ''; stream.tilesPending = false; return []; }
    // MP-08/MP-10: a DOM packet right after viewer input carries no still; its regions (and the 25% budget)
    // are measured by the next quiet packet, so an echo does not wait for an evaluation in a busy page.
    if (!reset && !still && this.recentInput(stream)) { stream.tilesPending = true; return []; }
    stream.tilesPending = false;
    let boxes = [...(await this.evaluate(world, `${world.ref}.opaqueBoxes()`)).filter(b => !b.foreign || !stream.frames.has(b.id)), ...await this.frames.opaqueBoxes(stream, world)];
    stream.tileKeys = boxes.map(b => b.id).join(',');
    // Plan 3.2 handoff: opaque regions over most of the viewport hand the
    // whole page to protected video (labelled; the client retries later).
    if (regionOverBudget(boxes)) { const error = new Error('MP-10: opaque regions exceed the mirror budget'); error.mirrorReason = 'region_area'; throw error; }
    if (!boxes.length || !still || !reset && Date.now() - stream.tilesAt < TILE_REFRESH_MS) return [];
    stream.tilesAt = Date.now();
    // The viewer keeps a still until replaced: unchanged pixels are not sent again.
    if (reset || !stream.tileHashes) stream.tileHashes = new Map();
    const scale = this.host.scales.get(tab.tab_id) ?? 1, out = [], shots = [];
    const send = (id, tile) => { const digest = createHash('sha256').update(`${tile.x},${tile.y},${tile.width},${tile.height},${tile.data_base64}`).digest('hex'); if (stream.tileHashes.get(id) === digest) return; stream.tileHashes.set(id, digest); out.push({ node_id: id, ...tile }); };
    for (const b of boxes) {
      const slot = slotOf(b.id), owner = slot ? stream.frameSlots.get(slot)?.child : world;
      const own = b.foreign || !owner ? null : await this.evaluate(owner, `${owner.ref}.elementPixels(${JSON.stringify(slot ? localId(b.id) : b.id)},${scale})`, true).catch(() => null);
      if (own) send(b.id, own);
      else if (b.distorted) { const error = new Error('MP-10: a transformed opaque region needs video'); error.mirrorReason = 'region_transform'; throw error; }
      else shots.push(b);
    }
    if (!shots.length) return out;
    boxes = shots;
    const x0 = Math.max(0, Math.floor(Math.min(...boxes.map(b => b.box[0])))), y0 = Math.max(0, Math.floor(Math.min(...boxes.map(b => b.box[1]))));
    const x1 = Math.min(1280, Math.ceil(Math.max(...boxes.map(b => b.box[0] + b.box[2])))), y1 = Math.min(800, Math.ceil(Math.max(...boxes.map(b => b.box[1] + b.box[3]))));
    if (x1 <= x0 || y1 <= y0) return [];
    const clip = { x: x0, y: y0, width: x1 - x0, height: y1 - y0, scale: 1 };
    const masks = await captureRegionMasks(world.connection, world.sessionId);
    const captured = await this.host.screenshot(tab, clip);
    const protectedRegions = await masks.afterCapture({ width: 1280 * scale, height: 800 * scale });
    const frame = maskPixels(decodePng(captured.data_base64, scale), protectedRegions.map(r => [r.x - clip.x * scale, r.y - clip.y * scale, r.width, r.height]));
    for (const { id, box } of boxes) {
      const left = Math.max(box[0], x0), top = Math.max(box[1], y0), right = Math.min(box[0] + box[2], x1), bottom = Math.min(box[1] + box[3], y1);
      const x = Math.floor((left - x0) * scale), y = Math.floor((top - y0) * scale);
      const w = Math.min(frame.width, Math.ceil((right - x0) * scale)) - x, h = Math.min(frame.height, Math.ceil((bottom - y0) * scale)) - y;
      if (w <= 0 || h <= 0) continue;
      send(id, { x: x / scale + x0 - box[0], y: y / scale + y0 - box[1], width: w / scale, height: h / scale, data_base64: losslessRegion(frame, x, y, w, h).data_base64 });
    }
    return out;
  }
  async resolveInput(stream, tab, input, scope, signal) {
    this.service.require(input.subscription_id, scope, this.host.generation);
    if (stream.tab_id !== tab.tab_id || stream.document_id !== tab.document_id) throw new Error('MP-11: stale mirror input document');
    if (stream.policy !== this.host.protection) throw new Error('MP-11: stale mirror protection policy');
    // Epochs are sequences of this document's current snapshot, never a wall clock.
    if (stream.fallback || !Number.isSafeInteger(input.sequence) || input.sequence < stream.resetAt || input.sequence > stream.issued) throw new MirrorInputEpochRefusal();
    const action = input.action, generation = this.host.generation;
    const validId = id => typeof id === 'string' && /^n[1-9][0-9]{0,15}$/.test(id);
    if (!action || typeof action.kind !== 'string') throw new Error('MP-11: invalid mirror input');
    if (['click', 'focus', 'scroll', 'blur', 'set'].includes(action.kind) && !validId(action.node_id) || action.kind === 'selection' && (!validId(action.anchor_id) || !validId(action.focus_id))) throw new Error('MP-11: invalid mirror input');
    if (action.kind === 'text' && (typeof action.text !== 'string' || action.text.length > 16384 || action.node_id !== undefined && !validId(action.node_id))) throw new Error('MP-11: invalid mirror input');
    // A pointer target whose attributes changed after the viewer's view is refused (re-sync). Focus, range and
    // control state refuse only a changed identity (type, editability, morph), like text and keys: a field's own
    // focus handler changing its aria-* must not refuse the range sent with that focus.
    const changed = ['click', 'scroll'].includes(action.kind) ? stream.attrSequence : stream.identity ?? new Map();
    for (const id of [action.node_id, action.anchor_id, action.focus_id]) if (id && (changed.get(id) ?? 0) > input.sequence) throw new Error('MP-11: changed mirror input target');
    stream.inputAt = Date.now();
    // MP-08/MP-10/MP-11: the applied snapshot owns this isolated world. Reuse
    // it only on the same live connection/document; every call still fences the
    // document, and the final guard rechecks the live target after capture.
    const cached = stream.world;
    const world = cached && cached.connection === this.host.browser.connection && cached.connection.isOpen() && cached.frame?.loaderId === tab.document_id
      ? cached : await this.world(tab, stream);
    const assertEpoch = () => { if (this.service.require(input.subscription_id, scope, generation) !== stream || stream.policy !== this.host.protection || stream.document_id !== tab.document_id || stream.fallback) throw new Error('MP-11: stale mirror protection policy or admitted input'); };
    const call = async expression => { assertNotCancelled(signal); assertEpoch(); await assertCurrentDocument(world.connection, world.sessionId, tab.target_id, tab.document_id); assertEpoch(); const value = await this.evaluate(world, expression); assertEpoch(); return value; };
    const m = world.ref;
    // Nodes of a mirrored cross-origin frame live in that frame's own world.
    const frameOf = id => { const slot = id ? slotOf(id) : 0; if (!slot) return null; const entry = stream.frameSlots.get(slot); if (!entry) throw new Error('MP-11: changed mirror input target'); return entry; };
    const inFrame = async (entry, expression) => { assertNotCancelled(signal); assertEpoch(); await assertCurrentDocument(world.connection, world.sessionId, tab.target_id, tab.document_id); const value = await this.evaluate(entry.child, expression); assertEpoch(); return value; };
    if (action.kind === 'selection' && slotOf(action.anchor_id) !== slotOf(action.focus_id)) throw new Error('MP-11: invalid mirror selection');
    if (action.kind === 'click' || action.kind === 'scroll') {
      const entry = frameOf(action.node_id);
      let at, local = null;
      if (entry) {
        local = await inFrame(entry, `${m}.point(${JSON.stringify({ node_id: localId(action.node_id), x: action.x, y: action.y })})`);
        const origin = await call(`${m}.frameOrigin(${JSON.stringify(entry.owner)})`);
        at = { x: Math.floor(local.x + origin[0]), y: Math.floor(local.y + origin[1]) };
        if (at.x < 0 || at.y < 0 || at.x >= 1280 || at.y >= 800) throw new Error('MP-11: changed mirror input target');
      } else at = await call(`${m}.point(${JSON.stringify({ node_id: action.node_id, x: action.x, y: action.y })})`);
      let checked = false;
      // MP-11: re-validated after focus emulation, immediately before the first
      // physical event; release stays paired even if the page reacts.
      const guard = async () => {
        assertEpoch(); assertNotCancelled(signal); if (checked) return;
        if (entry) { await call(`${m}.hitCheck(${JSON.stringify(at)},${JSON.stringify(entry.owner)})`); await inFrame(entry, `${m}.hitCheck(${JSON.stringify(local)},${JSON.stringify(localId(action.node_id))})`); }
        else await call(`${m}.hitCheck(${JSON.stringify(at)},${JSON.stringify(action.node_id)})`);
        checked = true;
      };
      if (action.kind === 'click') return { input: { kind: 'click', ...at }, guard };
      if (!Number.isInteger(action.delta_x) || !Number.isInteger(action.delta_y)) throw new Error('MP-11: invalid mirror input');
      return { input: { kind: 'scroll', ...at, delta_x: action.delta_x, delta_y: action.delta_y }, guard };
    }
    if (action.kind === 'scroll_to') {
      if (action.node_id !== undefined && action.node_id !== null && !validId(action.node_id) || !Number.isFinite(action.x) || !Number.isFinite(action.y) || Math.abs(action.x) > 1e7 || Math.abs(action.y) > 1e7) throw new Error('MP-11: invalid mirror input');
      const entry = frameOf(action.node_id);
      return { perform: async () => {
        stream.movedAt = Date.now();
        if (entry) await inFrame(entry, `${m}.scrollTo(${JSON.stringify({ node_id: localId(action.node_id), x: action.x, y: action.y })})`);
        else stream.ownScrolls.set(action.node_id ?? null, JSON.stringify(await call(`${m}.scrollTo(${JSON.stringify({ node_id: action.node_id ?? null, x: action.x, y: action.y })})`)));
      } };
    }
    // The target (or live focused leaf) and its ancestry must be part of the viewer's applied epoch.
    const seen = ids => { for (const id of ids) { const at = stream.introduced?.get(id); if (!at || at > input.sequence || (stream.identity.get(id) ?? 0) > input.sequence) throw new Error('MP-11: changed mirror input target'); } };
    if (['focus', 'blur', 'set'].includes(action.kind)) {
      const entry = frameOf(action.node_id), request = { ...action, node_id: entry ? localId(action.node_id) : action.node_id }, expression = `${m}.${action.kind}(${JSON.stringify(request)})`;
      return { perform: async () => {
        // Protocol 492: a committed control state names a control the viewer saw, inside a document it saw.
        if (action.kind === 'set') { if (entry) { seen((await inFrame(entry, `${m}.ancestry(${JSON.stringify(request.node_id)})`)).map(id => rebaseId(id, entry.slot))); seen(await call(`${m}.ancestry(${JSON.stringify(entry.owner)})`)); } else seen(await call(`${m}.ancestry(${JSON.stringify(action.node_id)})`)); }
        await (entry ? inFrame(entry, expression) : call(expression));
      } };
    }
    if (action.kind === 'selection') { const entry = frameOf(action.anchor_id); return { perform: async () => { if (entry) await inFrame(entry, `${m}.select(${JSON.stringify({ ...action, anchor_id: localId(action.anchor_id), focus_id: localId(action.focus_id) })})`); else await call(`${m}.select(${JSON.stringify(action)})`); } }; }
    // Live focus inside a mirrored cross-origin frame is validated in that frame.
    const activeTarget = async editable => {
      // MP-08/MP-10/MP-11: read the leaf and its foreign-frame binding in one
      // task. Keep the same live ancestry/epoch proof before physical dispatch.
      const [owner, ids] = await call(`(()=>{const owner=${m}.activeForeign();return [owner,owner?null:${m}.activeTarget(${editable})]})()`);
      const entry = owner && stream.frames.get(owner); if (owner && !entry) throw new Error('MP-11: unavailable native text focus');
      if (!entry) return seen(ids);
      seen((await inFrame(entry, `${m}.activeTarget(${editable})`)).map(id => rebaseId(id, entry.slot))); seen(await call(`${m}.ancestry(${JSON.stringify(owner)})`));
    };
    // The shared text preflight's verdict for focus inside a mirrored cross-origin frame.
    const frameSensitive = async expression => { const owner = await call(`${m}.activeForeign()`); const entry = owner && stream.frames.get(owner); return entry ? inFrame(entry, expression) : true; };
    if (action.kind === 'key') {
      let checked = false;
      const guard = async () => { assertEpoch(); assertNotCancelled(signal); if (checked) return; await activeTarget(false); checked = true; };
      // Protocol 530 (review #1016 P2): undo/redo keep this mirror admission (observed frames, live
      // focus, epoch) as the editor's own Linux Chromium chords: Control=2, Shift=8.
      const history = HISTORY_CHORDS.get(action.key);
      return { input: history ? { kind: 'key', key: 'z', modifiers: history } : { kind: 'key', key: action.key }, guard, observedFrameInput: true, frameSensitive };
    }
    if (action.kind === 'text') {
      const guard = async () => { assertEpoch(); assertNotCancelled(signal); await activeTarget(true); };
      const entry = frameOf(action.node_id);
      return { guard, observedFrameInput: true, frameSensitive, perform: async send => {
        if (action.node_id) { if (entry) await inFrame(entry, `${m}.focus(${JSON.stringify({ node_id: localId(action.node_id) })})`); else await call(`${m}.focus(${JSON.stringify({ node_id: action.node_id })})`); }
        assertNotCancelled(signal); assertEpoch();
        return send('Input.insertText', { text: action.text });
      } };
    }
    throw new Error('MP-08: unsupported mirror action');
  }
}
export { sanitizeMirrorCss };
