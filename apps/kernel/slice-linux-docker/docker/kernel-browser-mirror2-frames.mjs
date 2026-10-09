// MP-08/MP-10/MP-11: cross-origin child frames (plan 2.4). The same v2 observer
// runs in each child frame's own isolated world, reached through the page's
// CDP session (in-process frames) or the frame's flat OOPIF session. Child ids
// and resource keys are rebased per frame slot; a frame that cannot be
// attached stays an opaque region, painted only from masked captures.
import { mirror2ObserverExpression } from './kernel-browser-mirror2-observer.mjs';

export const FRAME_SLOT = 1e9, KEY_SLOT = 1e6;
const MAX_FRAMES = 16, WORLD = 'chariox-mirror-frame';
export const rebaseId = (id, slot) => slot ? `n${slot * FRAME_SLOT + Number(id.slice(1))}` : id;
export const rebaseKey = (key, slot) => slot ? `r${slot * KEY_SLOT + Number(key.slice(1))}` : key;
export const slotOf = id => Math.floor(Number(id.slice(1)) / FRAME_SLOT);
export const localId = id => `n${Number(id.slice(1)) % FRAME_SLOT}`;
const rebaseCss = (text, slot) => text.replace(/mr:r([0-9]{1,6})/g, (_, n) => `mr:${rebaseKey(`r${n}`, slot)}`);

export function rebaseRecord(record, slot) {
  if (!slot) return record;
  const out = { ...record, id: rebaseId(record.id, slot), parent: record.parent === null ? null : rebaseId(record.parent, slot) };
  // Depth 1 only: a frame nested inside a child frame stays an opaque region.
  if (out.foreign) { delete out.foreign; out.kind = 'tile'; out.reason = 'cross_origin_frame'; }
  if (out.res) out.res = rebaseKey(out.res, slot);
  if (out.css !== undefined) out.css = rebaseCss(out.css, slot);
  if (out.attrs?.style) out.attrs = { ...out.attrs, style: rebaseCss(out.attrs.style, slot) };
  if (out.adopted) out.adopted = out.adopted.map(text => rebaseCss(text, slot));
  return out;
}
export function rebaseOp(op, slot) {
  if (!slot) return op;
  const out = { ...op, id: rebaseId(op.id, slot) };
  if (op.op === 'children') { out.children = op.children.map(id => rebaseId(id, slot)); out.nodes = op.nodes.map(r => rebaseRecord(r, slot)); }
  if (op.op === 'attr' && op.name === 'style' && op.value) out.value = rebaseCss(op.value, slot);
  if (op.op === 'css') out.css = rebaseCss(op.css, slot);
  if (op.op === 'adopted') out.sheets = op.sheets.map(text => rebaseCss(text, slot));
  if (op.op === 'res' && op.res) out.res = rebaseKey(op.res, slot);
  return out;
}

export class MirrorFrames {
  constructor(mirror) { this.mirror = mirror; this.failures = []; }
  // Private diagnostics (fixed CDP step labels only), bounded.
  failed(step, error) { this.failures.push({ step, error: String(error?.message ?? error).slice(0, 120) }); if (this.failures.length > 64) this.failures.shift(); }
  // Child frame ids of this page by owner element mirror id (top-level owners only).
  async owners(world) {
    const { frameTree } = await world.connection.send('Page.getFrameTree', {}, world.sessionId);
    // In-process children are in the page's frame tree; out-of-process ones are
    // iframe targets whose frame id is the target id (owned by this page or not).
    const { targetInfos = [] } = await world.connection.send('Target.getTargets');
    const candidates = [...(frameTree.childFrames ?? []).map(child => child.frame), ...targetInfos.filter(t => t.type === 'iframe').map(t => ({ id: t.targetId, url: t.url, oopif: true }))];
    const owners = new Map();
    for (const frame of candidates.slice(0, 64)) {
      try {
        const { backendNodeId } = await world.connection.send('DOM.getFrameOwner', { frameId: frame.id }, world.sessionId);
        const { object } = await world.connection.send('DOM.resolveNode', { backendNodeId, executionContextId: world.contextId }, world.sessionId);
        const reply = await world.connection.send('Runtime.callFunctionOn', { objectId: object.objectId, functionDeclaration: 'function(){return globalThis.__charioxMirror2.idOfNode.call(this)}', returnByValue: true }, world.sessionId);
        await world.connection.send('Runtime.releaseObject', { objectId: object.objectId }, world.sessionId).catch(() => {});
        const id = reply.result?.value;
        if (typeof id === 'string' && /^n[1-9][0-9]*$/.test(id)) owners.set(id, frame); else this.failed('owner_id', id);
      } catch (error) { if (!frame.oopif) this.failed('owner', error); }
    }
    return owners;
  }
  // A child world: the OOPIF's own flat session if it has one, else the page session.
  async world(world, frame) {
    const sessionId = this.mirror.host.browser.frameSession?.(frame.id, world.connection) ?? world.sessionId;
    const { executionContextId } = await world.connection.send('Page.createIsolatedWorld', { frameId: frame.id, worldName: WORLD, grantUniveralAccess: false }, sessionId);
    const child = { connection: world.connection, sessionId, contextId: executionContextId, frame };
    const installed = await world.connection.send('Runtime.evaluate', { expression: mirror2ObserverExpression(), contextId: executionContextId, returnByValue: true }, sessionId);
    if (installed.exceptionDetails || installed.result?.value !== true) throw new Error('MP-11: frame observer unavailable');
    return child;
  }
  async loaderId(child) {
    const { frameTree } = await child.connection.send('Page.getFrameTree', {}, child.sessionId);
    const find = node => node.frame.id === child.frame.id ? node.frame : (node.childFrames ?? []).map(find).find(Boolean);
    return find(frameTree)?.loaderId ?? null;
  }
  // Attach every foreign frame named in this packet's records; others become
  // opaque regions. Returns rebased child snapshot records (document under owner).
  async attach(world, stream, foreignIds, variants, policy, tab) {
    const out = { records: [], ops: [], opaque: [] };
    if (!foreignIds.length) return out;
    // Vault targets inside child frames are only protected by region pixels.
    const frameTargets = policy.targets.some(t => t.kind === 'browser' && t.target_id === tab.target_id && /^frame:/.test(t.node_ref ?? ''));
    const owners = frameTargets ? new Map() : await this.owners(world);
    for (const id of foreignIds) {
      const frame = owners.get(id);
      if (!frame || stream.frames.size >= MAX_FRAMES) { this.failed('unowned', `${owners.size} owners`); out.opaque.push(id); continue; }
      try {
        const slot = ++stream.frameSlot;
        const child = await this.world(world, frame);
        const loaderId = await this.loaderId(child);
        const snap = await this.mirror.evaluate(child, `globalThis.__charioxMirror2.snapshot(${JSON.stringify({ variants })})`);
        const entry = { slot, owner: id, child, loaderId };
        stream.frames.set(id, entry); stream.frameSlots.set(slot, entry);
        const records = snap.nodes.map(r => rebaseRecord(r, slot));
        records[0] = { ...records[0], parent: id };
        out.records.push(...records);
        out.resources = [...(out.resources ?? []), ...snap.resources.map(d => ({ ...d, key: rebaseKey(d.key, slot), slot }))];
        if (snap.scroll[0] || snap.scroll[1]) out.ops.push({ op: 'scroll', id: records[0].id, scroll: snap.scroll });
      } catch (error) { this.failed('attach', error); out.opaque.push(id); }
    }
    return out;
  }
  // Deltas of attached children; a navigated child is re-snapshotted under its owner.
  async drain(stream, variants) {
    const out = { ops: [], resources: [] };
    for (const [owner, entry] of stream.frames) {
      try {
        const loaderId = await this.loaderId(entry.child);
        if (loaderId !== entry.loaderId) throw new Error('navigated');
        const delta = await this.mirror.evaluate(entry.child, `globalThis.__charioxMirror2.drain(${JSON.stringify({ variants })})`);
        if (delta.resync) throw new Error('resync');
        out.ops.push(...delta.ops.map(op => rebaseOp(op, entry.slot)));
        out.resources.push(...delta.resources.map(d => ({ ...d, key: rebaseKey(d.key, entry.slot), slot: entry.slot })));
      } catch {
        // New child document (or lost world): fresh snapshot replaces the owner's child.
        stream.frames.delete(owner); stream.frameSlots.delete(entry.slot);
        try {
          const owners = await this.owners(this.mirror.parentWorld(stream));
          const frame = owners.get(owner); if (!frame) continue;
          const slot = ++stream.frameSlot, child = await this.world(this.mirror.parentWorld(stream), frame);
          const snap = await this.mirror.evaluate(child, `globalThis.__charioxMirror2.snapshot(${JSON.stringify({ variants })})`);
          const next = { slot, owner, child, loaderId: await this.loaderId(child) };
          stream.frames.set(owner, next); stream.frameSlots.set(slot, next);
          const records = snap.nodes.map(r => rebaseRecord(r, slot)); records[0] = { ...records[0], parent: owner };
          out.ops.push({ op: 'children', id: owner, children: [records[0].id], nodes: records });
          out.resources.push(...snap.resources.map(d => ({ ...d, key: rebaseKey(d.key, slot), slot })));
        } catch {}
      }
    }
    return out;
  }
  // Viewport boxes of child opaque regions, in top-level coordinates.
  async opaqueBoxes(stream, parent) {
    const out = [];
    for (const [owner, entry] of stream.frames) {
      try {
        const origin = await this.mirror.evaluate(parent, `globalThis.__charioxMirror2.frameOrigin(${JSON.stringify(owner)})`);
        const boxes = await this.mirror.evaluate(entry.child, 'globalThis.__charioxMirror2.opaqueBoxes()');
        for (const { id, box } of boxes) out.push({ id: rebaseId(id, entry.slot), box: [box[0] + origin[0], box[1] + origin[1], box[2], box[3]] });
      } catch {}
    }
    return out;
  }
}
