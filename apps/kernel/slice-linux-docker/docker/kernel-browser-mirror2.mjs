// MP-08/MP-10/MP-11: DOM mirror v2 service (local protocol 482). One snapshot
// per document, then pushed deltas: each `mirror_next` credit with `wait_ms`
// is answered when the page changes (credit-gated long poll, at most a few in
// flight per viewer). Resources are hash-addressed and follow the packet.
import { createHash } from 'node:crypto';
import { timestamp } from './kernel-browser-timing.mjs';
import { mirror2ObserverExpression, sanitizeMirrorCss } from './kernel-browser-mirror2-observer.mjs';
import { observationProtectedVariants } from './browser-controller-snapshot.mjs';
import { assertCurrentDocument, assertNotCancelled } from './browser-controller-actions.mjs';
import { captureRegionMasks } from './kernel-browser-region-protection.mjs';
import { decodePng, maskPixels } from './kernel-browser-pixels.mjs';
import { losslessRegion } from './kernel-browser-display.mjs';
import { MirrorFrames, slotOf, localId } from './kernel-browser-mirror2-frames.mjs';

const MAX_WAIT_MS = 2000, RESOURCE_PACKET_BYTES = 256 * 1024, RESOURCE_BYTES = 4 * 1024 * 1024, TILE_REFRESH_MS = 1000;
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

function dataUrlBytes(url) {
  const match = /^data:([^,]*?)(;base64)?,(.*)$/s.exec(url);
  if (!match) return null;
  try { return match[2] ? Buffer.from(match[3], 'base64') : Buffer.from(decodeURIComponent(match[3]), 'utf8'); } catch { return null; }
}

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
export function dedupeMirrorSheets(packet, sent) {
  const sheets = {};
  const ref = text => {
    if (typeof text !== 'string' || text.length < 2048) return null;
    const digest = createHash('sha256').update(text).digest('hex').slice(0, 24);
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

export class Mirror2 {
  constructor(service) { this.service = service; this.host = service.host; this.frames = new MirrorFrames(this); }
  parentWorld(stream) { return stream.world; }
  async world(tab) {
    const { connection, sessionId } = await this.host.browser.resolvePageTarget(tab.target_id);
    const { frameTree } = await connection.send('Page.getFrameTree', {}, sessionId);
    if (frameTree?.frame?.loaderId !== tab.document_id) throw new Error('MP-11: stale mirror document');
    const world = await this.host.browser.ensureFocusWorld(connection, sessionId, tab.target_id, frameTree.frame);
    if (!Number.isSafeInteger(world.contextId) || world.contextId <= 0) throw new Error('MP-11: mirror isolated world unavailable');
    if (!world.mirror2Installed) {
      const installed = await connection.send('Runtime.evaluate', { expression: mirror2ObserverExpression(), contextId: world.contextId, returnByValue: true }, sessionId);
      if (installed.exceptionDetails || installed.result?.value !== true) throw new Error('MP-11: mirror observer unavailable');
      world.mirror2Installed = true;
    }
    return { connection, sessionId, contextId: world.contextId, frame: frameTree.frame };
  }
  async evaluate(world, expression, awaitPromise = false) {
    const reply = await world.connection.send('Runtime.evaluate', { expression, contextId: world.contextId, returnByValue: true, awaitPromise }, world.sessionId);
    if (reply.exceptionDetails) {
      const label = /mirror2:([a-z_]+)/.exec(reply.exceptionDetails.exception?.description ?? '')?.[1];
      const error = new Error('MP-11: mirror observation unavailable'); error.mirrorReason = label ?? 'observer_exception'; throw error;
    }
    if (!Object.hasOwn(reply.result ?? {}, 'value')) throw new Error('MP-11: mirror observation unavailable');
    return reply.result.value;
  }
  stream(stream) {
    Object.assign(stream, { wire: 2, issued: 0, resetAt: 0, chain: Promise.resolve(), resources: new Map(), attrSequence: new Map(), tilesAt: 0, tileKeys: '', fallback: null, frames: new Map(), frameSlots: new Map(), frameSlot: 0 });
    return stream;
  }
  // Registered Vault targets are marked by node identity in the observer's world.
  async protectTargets(world, tab, policy) {
    await this.evaluate(world, 'globalThis.__charioxMirror2.resetTargets()');
    for (const target of policy.targets) {
      if (target.kind !== 'browser' || target.target_id !== tab.target_id || target.document_id && target.document_id !== tab.document_id) continue;
      const match = /^backend:([1-9][0-9]*)$/.exec(target.node_ref ?? '');
      if (!match) continue; // frame-prefixed targets live in opaque frames; region pixels are masked at capture.
      try {
        const { object } = await world.connection.send('DOM.resolveNode', { backendNodeId: Number(match[1]), executionContextId: world.contextId }, world.sessionId);
        const marked = await world.connection.send('Runtime.callFunctionOn', { objectId: object.objectId, functionDeclaration: 'function(){return globalThis.__charioxMirror2.protect.call(this)}', returnByValue: true }, world.sessionId);
        if (marked.exceptionDetails || marked.result?.value !== true) throw new Error('MP-11: protected target unavailable');
        await world.connection.send('Runtime.releaseObject', { objectId: object.objectId }, world.sessionId).catch(() => {});
      } catch (error) {
        if (/No node with given id|Could not find node/.test(error?.message ?? '')) continue; // departed node: no pixels
        throw error;
      }
    }
  }
  async next(stream, command, scope, { signal } = {}) {
    if (command.wait_ms !== undefined && (!Number.isInteger(command.wait_ms) || command.wait_ms < 0 || command.wait_ms > MAX_WAIT_MS)) throw new Error('MP-11: invalid mirror wait');
    // Credits are answered in order; each packet's base is its predecessor.
    // A credit's long-poll deadline counts from its arrival: queued behind
    // others it must not outlive the client's request timeout.
    const deadline = Date.now() + (command.wait_ms ?? 0);
    const run = stream.chain.then(() => this.packet(stream, command, scope, signal, deadline));
    stream.chain = run.catch(() => {});
    return run;
  }
  async packet(stream, command, scope, signal, deadline = Date.now()) {
    const started = timestamp(); let stage = started;
    const mark = name => { this.host.timing?.(`mirror2_${name}`, stage); stage = timestamp(); };
    this.service.require(command.subscription_id, scope, command.generation);
    const tab = await this.host.displayTarget({ tab_id: stream.tab_id, generation: command.generation });
    this.service.assertWebTab(tab); assertNotCancelled(signal);
    const world = await this.world(tab), policy = this.host.protection;
    stream.world = world;
    mark('world');
    const variants = observationProtectedVariants(policy.values);
    const after = command.after_sequence;
    let reset = after === 0 || after > stream.issued || after < stream.issued - 8 || stream.document_id !== tab.document_id || stream.policy !== policy || stream.fallback !== null;
    if (stream.document_id !== tab.document_id) { stream.resources.clear(); stream.attrSequence.clear(); stream.tilesAt = 0; stream.loadedCount = -1; }
    // Rebased ids/keys bound the frame slot; a long session re-snapshots instead.
    if (stream.frameSlot >= 900) reset = true;
    // Frame slots restart with a snapshot, so rebased child keys are re-learned.
    if (reset) { stream.frames.clear(); stream.frameSlots.clear(); stream.frameSlot = 0; for (const [key, entry] of stream.resources) if (entry.slot) stream.resources.delete(key); }
    let source, fallback = null;
    const read = async () => {
      if (reset) {
        await this.protectTargets(world, tab, policy);
        const snap = await this.evaluate(world, `globalThis.__charioxMirror2.snapshot(${JSON.stringify({ variants })})`);
        if (snap.resync) return snap;
        // Cross-origin frames: child DOM under the owner, or an opaque region.
        const frames = await this.frames.attach(world, stream, snap.nodes.filter(r => r.foreign).map(r => r.id), variants, policy, tab);
        this.opaque(snap.nodes, frames.opaque);
        return { ...snap, nodes: [...snap.nodes, ...frames.records], ops: frames.ops, resources: [...snap.resources, ...(frames.resources ?? [])] };
      }
      const delta = await this.evaluate(world, `globalThis.__charioxMirror2.drain(${JSON.stringify({ variants })})`);
      if (delta.resync) return delta;
      const ops = [], resources = [...delta.resources];
      for (const op of delta.ops) {
        if (op.op === 'children') {
          const frames = await this.frames.attach(world, stream, op.nodes.filter(r => r.foreign).map(r => r.id), variants, policy, tab);
          this.opaque(op.nodes, frames.opaque);
          ops.push({ ...op, nodes: [...op.nodes, ...frames.records] }, ...frames.ops); resources.push(...(frames.resources ?? []));
        } else ops.push(op);
      }
      const children = await this.frames.drain(stream, variants);
      return { ...delta, ops: [...ops, ...children.ops], resources: [...resources, ...children.resources] };
    };
    let resources = [];
    try {
      source = await read();
      if (source.resync) { reset = true; source = await read(); }
      this.register(stream, source);
      // The first packet carries DOM, CSS and fonts; images follow it. Resource
      // bytes travel only in packets without DOM changes (and in bounded
      // slices), so an echo never queues behind image bytes on the socket.
      resources = reset ? await this.materialize(world, stream, 'font') : this.empty(stream, source) ? await this.materialize(world, stream, null) : [];
      // Long poll: nothing to send yet -> wait for the page (or newly loaded bytes).
      while (!reset && !resources.length && this.empty(stream, source) && Date.now() < deadline) {
        await this.evaluate(world, `globalThis.__charioxMirror2.wait(${Math.max(1, Math.min(500, deadline - Date.now()))})`, true);
        assertNotCancelled(signal);
        const more = await read();
        if (more.resync) { reset = true; source = await read(); this.register(stream, source); break; }
        this.register(stream, more); source = this.merge(source, more);
        if (this.empty(stream, source)) resources = await this.materialize(world, stream, null);
      }
    } catch (error) {
      await assertCurrentDocument(world.connection, world.sessionId, tab.target_id, tab.document_id);
      // Over-budget/unavailable DOM: the client shows the protected video path.
      fallback = error.mirrorReason ?? 'observer_unavailable';
      this.host.timing?.(`mirror2_fallback_${fallback}`, timestamp());
      source = null; reset = true;
    }
    if (fallback) resources = [];
    mark('observe_resources');
    const sheets = [];
    for (const sheet of source?.sheets ?? []) { const text = await this.crossOriginSheet(world, sheet.url).catch(() => null); if (text !== null) sheets.push({ op: 'css', id: sheet.id, css: text }); }
    const tiles = fallback ? [] : await this.tiles(world, tab, stream, reset);
    mark('tiles');
    await assertCurrentDocument(world.connection, world.sessionId, tab.target_id, tab.document_id); assertNotCancelled(signal);
    if (this.host.protection !== policy || this.host.generation !== command.generation || this.service.streams.get(command.subscription_id) !== stream) {
      stream.policy = null; throw new Error('MP-11: stale mirror protection policy or subscription');
    }
    const sequence = stream.issued + 1;
    const packet = { wire: 2, subscription_id: command.subscription_id, tab_id: tab.tab_id, generation: command.generation, document_id: tab.document_id,
      sequence, base_sequence: reset ? null : stream.issued, reset, css_width: 1280, css_height: 800, device_scale_factor: this.host.scales.get(tab.tab_id) ?? 1,
      scroll: source?.scroll ?? [0, 0], focused: source?.focused ?? null, selection: source?.selection ?? null, resources, tiles };
    if (fallback) packet.fallback = fallback;
    else if (reset) { packet.root = source.root; packet.nodes = source.nodes; packet.ops = sheets; }
    else packet.ops = [...source.ops, ...sheets];
    for (const op of packet.ops ?? []) if (op.op === 'attr') stream.attrSequence.set(op.id, sequence);
    if (reset) stream.sheetRefs = new Set();
    dedupeMirrorSheets(packet, stream.sheetRefs ??= new Set());
    if (packet.nodes) packet.nodes = encodeMirrorRecords(packet.nodes, null);
    if (packet.ops) packet.ops = packet.ops.map(op => op.op === 'children' ? { ...op, nodes: encodeMirrorRecords(op.nodes, op.id) } : op);
    stream.issued = sequence; stream.document_id = tab.document_id; stream.policy = policy; stream.fallback = fallback;
    if (reset) { stream.resetAt = sequence; stream.attrSequence.clear(); }
    stream.lastHeader = JSON.stringify([packet.scroll, packet.focused, packet.selection]);
    mark('serialize'); this.host.timing?.('mirror2_total', started);
    return packet;
  }
  empty(stream, source) {
    return !source.ops?.length && !source.sheets?.length && JSON.stringify([source.scroll, source.focused, source.selection]) === stream.lastHeader && !this.tilesDue(stream);
  }
  merge(a, b) { return { ...b, ops: [...(a.ops ?? []), ...(b.ops ?? [])], sheets: [...(a.sheets ?? []), ...(b.sheets ?? [])] }; }
  // Unattached foreign frames are opaque regions (masked captures only).
  opaque(records, ids) {
    for (const record of records) if (record.foreign) { delete record.foreign; if (ids.includes(record.id)) { record.kind = 'tile'; record.reason = 'cross_origin_frame'; } }
  }
  // Resource bytes are kept by the renderer for the document's lifetime: a
  // same-document reset (lost base) does not send them again.
  register(stream, source) {
    for (const descriptor of source?.resources ?? []) if (!stream.resources.has(descriptor.key)) stream.resources.set(descriptor.key, { ...descriptor, state: 'new', tries: 0, sent: false });
  }
  tilesDue(stream) { return stream.tileKeys !== '' && Date.now() - stream.tilesAt >= TILE_REFRESH_MS; }
  // Bytes the page itself loaded: data URLs, the inspector's resource tree, or
  // Chrome's cache (credential-free) for URLs in the page's Resource Timing.
  // A URL a stylesheet merely mentions waits until the page loads it.
  async materialize(main, stream, kind) {
    const worlds = new Map([[0, main], ...[...stream.frameSlots].map(([slot, entry]) => [slot, entry.child])]);
    let counts = 0; for (const world of worlds.values()) counts += await this.evaluate(world, 'globalThis.__charioxMirror2.loadedCount()').catch(() => 0);
    const recheck = counts !== stream.loadedCount; stream.loadedCount = counts;
    const out = [];
    for (const [slot, world] of worlds) {
      const due = [...stream.resources.values()].filter(entry => (entry.slot ?? 0) === slot && (!kind || entry.kind === kind) && (entry.state === 'new' || entry.state === 'ok' && !entry.sent || entry.state === 'waiting' && recheck));
      if (due.length) out.push(...await this.materializeWorld(world, due, slot, RESOURCE_PACKET_BYTES - out.reduce((n, r) => n + r.data_base64.length, 0)));
    }
    return out;
  }
  async materializeWorld(world, due, slot, budget) {
    const loaded = new Set((await this.evaluate(world, 'globalThis.__charioxMirror2.loaded()')).map(key => slot ? `r${slot * 1e6 + Number(key.slice(1))}` : key));
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
        else if (!loaded.has(entry.key)) { entry.state = 'waiting'; continue; }
        else {
          const frameId = (await frames()).get(entry.url);
          if (frameId) try { const reply = await world.connection.send('Page.getResourceContent', { frameId, url: entry.url }, world.sessionId); body = reply.base64Encoded ? Buffer.from(reply.content, 'base64') : Buffer.from(reply.content, 'utf8'); } catch { body = null; }
          body ??= await this.cached(world, entry.url).catch(() => null);
        }
        const mime_type = body && body.length <= RESOURCE_BYTES ? mirrorResourceType(body, entry.kind) : null;
        if (!mime_type) { entry.tries++; entry.state = entry.tries >= 3 ? 'failed' : 'waiting'; continue; }
        entry.state = 'ok'; entry.resource = { key: entry.key, resource_id: createHash('sha256').update(body).digest('hex'), mime_type, data_base64: body.toString('base64') };
      }
      entry.sent = true; bytes += entry.resource.data_base64.length; out.push(entry.resource);
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
  // Cross-origin sheets: CDP reads the text regardless of CORS; the page's
  // CSSOM in the isolated world normalizes it, then the same sanitizer runs.
  async crossOriginSheet(world, url) {
    const { frameTree } = await world.connection.send('Page.getResourceTree', {}, world.sessionId);
    const frameOf = new Map(); const collect = node => { for (const r of node.resources ?? []) frameOf.set(r.url, node.frame.id); for (const child of node.childFrames ?? []) collect(child); }; collect(frameTree);
    const frameId = frameOf.get(url); if (!frameId) return null;
    const reply = await world.connection.send('Page.getResourceContent', { frameId, url }, world.sessionId);
    const text = reply.base64Encoded ? Buffer.from(reply.content, 'base64').toString('utf8') : reply.content;
    if (typeof text !== 'string' || text.length > 16 * 1024 * 1024) return null;
    return this.evaluate(world, `(()=>{const sheet=new CSSStyleSheet();sheet.replaceSync(${JSON.stringify(text)});let out='';for(const rule of sheet.cssRules)out+=rule.cssText+'\\n';return globalThis.__charioxMirror2.sanitize(out,${JSON.stringify(url)});})()`);
  }
  // Opaque regions (canvas/video/foreign frames) as masked lossless stills,
  // refreshed at most once a second until phase 3 binds them to video rows.
  async tiles(world, tab, stream, reset) {
    const boxes = [...(await this.evaluate(world, 'globalThis.__charioxMirror2.opaqueBoxes()')).filter(b => !b.foreign || !stream.frames.has(b.id)), ...await this.frames.opaqueBoxes(stream, world)];
    stream.tileKeys = boxes.map(b => b.id).join(',');
    if (!boxes.length || !reset && Date.now() - stream.tilesAt < TILE_REFRESH_MS) return [];
    stream.tilesAt = Date.now();
    const scale = this.host.scales.get(tab.tab_id) ?? 1;
    const x0 = Math.max(0, Math.floor(Math.min(...boxes.map(b => b.box[0])))), y0 = Math.max(0, Math.floor(Math.min(...boxes.map(b => b.box[1]))));
    const x1 = Math.min(1280, Math.ceil(Math.max(...boxes.map(b => b.box[0] + b.box[2])))), y1 = Math.min(800, Math.ceil(Math.max(...boxes.map(b => b.box[1] + b.box[3]))));
    if (x1 <= x0 || y1 <= y0) return [];
    const clip = { x: x0, y: y0, width: x1 - x0, height: y1 - y0, scale: 1 };
    const masks = await captureRegionMasks(world.connection, world.sessionId);
    const captured = await this.host.screenshot(tab, clip);
    const protectedRegions = await masks.afterCapture({ width: 1280 * scale, height: 800 * scale });
    const frame = maskPixels(decodePng(captured.data_base64, scale), protectedRegions.map(r => [r.x - clip.x * scale, r.y - clip.y * scale, r.width, r.height]));
    const out = [];
    for (const { id, box } of boxes) {
      const left = Math.max(box[0], x0), top = Math.max(box[1], y0), right = Math.min(box[0] + box[2], x1), bottom = Math.min(box[1] + box[3], y1);
      const x = Math.floor((left - x0) * scale), y = Math.floor((top - y0) * scale);
      const w = Math.min(frame.width, Math.ceil((right - x0) * scale)) - x, h = Math.min(frame.height, Math.ceil((bottom - y0) * scale)) - y;
      if (w <= 0 || h <= 0) continue;
      out.push({ node_id: id, x: x / scale + x0 - box[0], y: y / scale + y0 - box[1], width: w / scale, height: h / scale, data_base64: losslessRegion(frame, x, y, w, h).data_base64 });
    }
    return out;
  }
  async resolveInput(stream, tab, input, scope, signal) {
    this.service.assertWebTab(tab); this.service.require(input.subscription_id, scope, this.host.generation);
    if (stream.tab_id !== tab.tab_id || stream.document_id !== tab.document_id) throw new Error('MP-11: stale mirror input document');
    if (stream.policy !== this.host.protection) throw new Error('MP-11: stale mirror protection policy');
    // Epochs are sequences of this document's current snapshot, never a wall clock.
    if (stream.fallback || !Number.isSafeInteger(input.sequence) || input.sequence < stream.resetAt || input.sequence > stream.issued) throw new MirrorInputEpochRefusal();
    const action = input.action, generation = this.host.generation;
    const validId = id => typeof id === 'string' && /^n[1-9][0-9]{0,15}$/.test(id);
    if (!action || typeof action.kind !== 'string') throw new Error('MP-11: invalid mirror input');
    if (['click', 'focus', 'scroll'].includes(action.kind) && !validId(action.node_id) || action.kind === 'selection' && (!validId(action.anchor_id) || !validId(action.focus_id))) throw new Error('MP-11: invalid mirror input');
    if (['text', 'composition'].includes(action.kind) && (typeof action.text !== 'string' || action.text.length > 16384 || action.node_id !== undefined && !validId(action.node_id))) throw new Error('MP-11: invalid mirror input');
    if (action.kind === 'composition' && (!Number.isInteger(action.selection_start) || !Number.isInteger(action.selection_end) || action.selection_start < 0 || action.selection_end < action.selection_start || action.selection_end > action.text.length)) throw new Error('MP-11: invalid mirror input');
    // A target whose attributes changed after the viewer's view is refused (re-sync).
    for (const id of [action.node_id, action.anchor_id, action.focus_id]) if (id && (stream.attrSequence.get(id) ?? 0) > input.sequence) throw new Error('MP-11: changed mirror input target');
    const world = await this.world(tab);
    const assertEpoch = () => { if (this.service.require(input.subscription_id, scope, generation) !== stream || stream.policy !== this.host.protection || stream.document_id !== tab.document_id || stream.fallback) throw new Error('MP-11: stale mirror protection policy or admitted input'); };
    const call = async expression => { assertNotCancelled(signal); assertEpoch(); await assertCurrentDocument(world.connection, world.sessionId, tab.target_id, tab.document_id); assertEpoch(); const value = await this.evaluate(world, expression); assertEpoch(); return value; };
    const m = 'globalThis.__charioxMirror2';
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
      return { perform: async () => { if (entry) await inFrame(entry, `${m}.scrollTo(${JSON.stringify({ node_id: localId(action.node_id), x: action.x, y: action.y })})`); else await call(`${m}.scrollTo(${JSON.stringify({ node_id: action.node_id ?? null, x: action.x, y: action.y })})`); } };
    }
    if (action.kind === 'focus') { const entry = frameOf(action.node_id); return { perform: async () => { if (entry) await inFrame(entry, `${m}.focus(${JSON.stringify({ node_id: localId(action.node_id) })})`); else await call(`${m}.focus(${JSON.stringify({ node_id: action.node_id })})`); } }; }
    if (action.kind === 'selection') { const entry = frameOf(action.anchor_id); return { perform: async () => { if (entry) await inFrame(entry, `${m}.select(${JSON.stringify({ ...action, anchor_id: localId(action.anchor_id), focus_id: localId(action.focus_id) })})`); else await call(`${m}.select(${JSON.stringify(action)})`); } }; }
    // Live focus inside a mirrored cross-origin frame is validated in that frame.
    const activeTarget = async editable => { const owner = await call(`${m}.activeForeign()`); const entry = owner && stream.frames.get(owner); if (owner && !entry) throw new Error('MP-11: unavailable native text focus'); return entry ? inFrame(entry, `${m}.activeTarget(${editable})`) : call(`${m}.activeTarget(${editable})`); };
    if (action.kind === 'key') {
      let checked = false;
      const guard = async () => { assertEpoch(); assertNotCancelled(signal); if (checked) return; await activeTarget(false); checked = true; };
      return { input: { kind: 'key', key: action.key }, guard, observedFrameInput: true };
    }
    if (action.kind === 'text' || action.kind === 'composition') {
      const guard = async () => { assertEpoch(); assertNotCancelled(signal); await activeTarget(true); };
      const entry = frameOf(action.node_id);
      return { guard, observedFrameInput: true, perform: async send => {
        if (action.node_id) { if (entry) await inFrame(entry, `${m}.focus(${JSON.stringify({ node_id: localId(action.node_id) })})`); else await call(`${m}.focus(${JSON.stringify({ node_id: action.node_id })})`); }
        assertNotCancelled(signal); assertEpoch();
        if (action.kind === 'text') return send('Input.insertText', { text: action.text });
        return send('Input.imeSetComposition', { text: action.text, selectionStart: action.selection_start, selectionEnd: action.selection_end });
      } };
    }
    throw new Error('MP-08: unsupported mirror action');
  }
}
export { sanitizeMirrorCss };
