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

// Schema-driven rebasing (audit 2026-10-10 step 5): every field of a child record, op and delta is
// named here with how it carries ids, resource keys or CSS; an unnamed field is refused, so a new
// field cannot travel from a child frame un-rebased.
const same = value => value;
const id = slot => value => rebaseId(value, slot);
const RECORD = slot => ({ id: id(slot), parent: value => value === null ? null : rebaseId(value, slot), kind: same, tag: same, ns: same, text: same, form: same, scroll: same, fragment: same, size: same, display: same, reason: same, state: same, foreign: same,
  attrs: attrs => attrs.style ? { ...attrs, style: rebaseCss(attrs.style, slot) } : attrs, res: key => rebaseKey(key, slot), css: text => rebaseCss(text, slot), adopted: list => list.map(text => rebaseCss(text, slot)) });
const OPS = {
  children: slot => ({ children: list => list.map(id(slot)), nodes: list => list.map(record => rebaseRecord(record, slot)) }),
  attr: slot => ({ name: same, value: same }), text: () => ({ text: same }), form: () => ({ form: same }), scroll: () => ({ scroll: same }), size: () => ({ size: same }), state: () => ({ state: same }),
  css: slot => ({ css: text => rebaseCss(text, slot) }), adopted: slot => ({ sheets: list => list.map(text => rebaseCss(text, slot)) }), res: slot => ({ res: key => key && rebaseKey(key, slot) }), fragment: () => ({ fragment: same }),
  fonts: () => ({ required: same, absent: same }),
};
const DELTA = slot => ({ ops: list => list.map(op => rebaseOp(op, slot)), changed: list => list.map(id(slot)), resources: list => list.map(d => ({ ...d, key: rebaseKey(d.key, slot), slot })),
  focused: value => value ? rebaseId(value, slot) : null, selection: value => value ? { ...value, anchor_id: rebaseId(value.anchor_id, slot), focus_id: rebaseId(value.focus_id, slot) } : null,
  snapshot_complete: same, font_failures: same, sheets: same, scroll: same, fragment: same, revision: same, pressed: same, opaque: same, loaded: same }); // sheets: frame-local ids, read through the child's own CDP session first
const apply = (value, schema, what) => Object.fromEntries(Object.entries(value).map(([name, field]) => { if (!Object.hasOwn(schema, name)) throw new Error(`MP-11: unrebased mirror ${what} field`); return [name, field === undefined ? field : schema[name](field)]; }));
export function rebaseRecord(record, slot) {
  if (!slot) return record;
  const out = apply(record, RECORD(slot), 'record');
  // Depth 1 only: a frame nested inside a child frame stays an opaque region.
  if (out.foreign) { delete out.foreign; out.kind = 'tile'; out.reason = 'cross_origin_frame'; }
  return out;
}
export function rebaseOp(op, slot) {
  if (!slot) return op;
  const schema = OPS[op.op]; if (!schema) throw new Error('MP-11: unrebased mirror op');
  const out = apply(op, { op: same, id: id(slot), ...schema(slot) }, 'op');
  if (op.op === 'attr' && op.name === 'style' && op.value) out.value = rebaseCss(op.value, slot);
  return out;
}
export const rebaseDelta = (delta, slot) => apply(delta, DELTA(slot), 'delta');

// A child's focused leaf and selection in viewer ids (its page sees only the owner focused).
const rebaseFocus = (header, slot) => { const { focused, selection } = rebaseDelta({ focused: header.focused, selection: header.selection }, slot); return { focused, selection }; };

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
        const reply = await world.connection.send('Runtime.callFunctionOn', { objectId: object.objectId, functionDeclaration: `function(){return ${world.ref}.idOfNode.call(this)}`, returnByValue: true }, world.sessionId, { timeoutMs: world.patience?.() });
        await world.connection.send('Runtime.releaseObject', { objectId: object.objectId }, world.sessionId).catch(() => {});
        const id = reply.result?.value;
        if (typeof id === 'string' && /^n[1-9][0-9]*$/.test(id)) owners.set(id, frame); else this.failed('owner_id', id);
      } catch (error) { if (!frame.oopif) this.failed('owner', error); }
    }
    return owners;
  }
  // A child world: the OOPIF's own flat session if it has one, else the page session.
  async world(world, frame, streamId) {
    const sessionId = this.mirror.host.browser.frameSession?.(frame.id, world.connection) ?? world.sessionId;
    const { executionContextId } = await world.connection.send('Page.createIsolatedWorld', { frameId: frame.id, worldName: WORLD, grantUniveralAccess: false }, sessionId);
    const child = { connection: world.connection, sessionId, contextId: executionContextId, frame, ref: world.ref };
    const installed = await world.connection.send('Runtime.evaluate', { expression: mirror2ObserverExpression(), contextId: executionContextId, returnByValue: true }, sessionId);
    if (installed.exceptionDetails || installed.result?.value !== true) throw new Error('MP-11: frame observer unavailable');
    await this.mirror.watchCss(child, frame.id, streamId);
    return child;
  }
  async loaderId(child) {
    const { frameTree } = await child.connection.send('Page.getFrameTree', {}, child.sessionId);
    const find = node => node.frame.id === child.frame.id ? node.frame : (node.childFrames ?? []).map(find).find(Boolean);
    return find(frameTree)?.loaderId ?? null;
  }
  // Attach every foreign frame named in this packet's records; others become
  // opaque regions. Returns rebased child snapshot records (document under owner).
  // Child snapshot with the frame's Vault fill targets protected by identity.
  async snapshot(child, loaderId, policy, tab, slot) {
    child.documentId = loaderId;
    await this.mirror.protectTargets(child, tab, policy, `frame:${child.frame.id}:${loaderId}:`);
    const snap = await this.mirror.evaluate(child, `${child.ref}.snapshot()`, false, undefined, true);
    delete snap.full_pending; // a child frame sends its sheets whole
    await this.mirror.markClosedHosts(child, snap);
    return { ...snap, sheetOps: await this.sheetOps(child, slot, snap.sheets) };
  }
  // Stylesheets the child's CSSOM cannot read: CDP reads them in the child's own
  // session; the CSS op is rebased into the frame slot like its records.
  async sheetOps(child, slot, sheets = []) {
    const ops = [];
    for (const { id, url } of sheets) {
      const text = await this.mirror.crossOriginSheet(child, url).catch(error => { this.failed('sheet', error); return null; });
      if (text !== null) ops.push(rebaseOp({ op: 'css', id, css: text }, slot));
    }
    return ops;
  }
  async attach(world, stream, foreignIds, policy, tab) {
    const out = { records: [], ops: [], opaque: [] };
    if (!foreignIds.length) return out;
    const owners = await this.owners(world);
    for (const id of foreignIds) {
      const frame = owners.get(id);
      if (!frame || stream.frames.size >= MAX_FRAMES) { this.failed('unowned', `${owners.size} owners`); out.opaque.push(id); continue; }
      try {
        const slot = ++stream.frameSlot;
        const child = await this.world(world, frame, stream.id);
        const loaderId = await this.loaderId(child);
        const snap = await this.snapshot(child, loaderId, policy, tab, slot);
        const entry = { slot, owner: id, child, loaderId, documentId: rebaseId(snap.nodes[0].id, slot), scroll: JSON.stringify(snap.scroll), fragment: snap.fragment ?? null, focus: rebaseFocus(snap, slot) };
        stream.frames.set(id, entry); stream.frameSlots.set(slot, entry);
        const records = snap.nodes.map(r => rebaseRecord(r, slot));
        // The child's own fragment (its header) is its document record's (protocol 503).
        records[0] = { ...records[0], parent: id, ...(entry.fragment ? { fragment: entry.fragment } : {}) };
        out.records.push(...records);
        out.resources = [...(out.resources ?? []), ...snap.resources.map(d => ({ ...d, key: rebaseKey(d.key, slot), slot }))];
        if (snap.scroll[0] || snap.scroll[1]) out.ops.push({ op: 'scroll', id: records[0].id, scroll: snap.scroll });
        out.ops.push(...(snap.ops ?? []).map(op => rebaseOp(op, slot)), ...snap.sheetOps);
      } catch (error) { this.failed('attach', error); out.opaque.push(id); }
    }
    return out;
  }
  // Deltas of attached children; a navigated child is re-snapshotted under its owner.
  async drain(stream, policy, tab) {
    const out = { ops: [], resources: [], changed: [] };
    for (const [owner, entry] of stream.frames) {
      try {
        const loaderId = await this.loaderId(entry.child);
        if (loaderId !== entry.loaderId) throw new Error('navigated');
        let raw = await this.mirror.evaluate(entry.child, `${entry.child.ref}.drain()`, false, undefined, true);
        if (raw.closed_candidates && await this.mirror.markClosedHosts(entry.child, raw)) { raw = this.mirror.merge(raw, await this.mirror.evaluate(entry.child, `${entry.child.ref}.drain()`, false, undefined, true)); }
        if (raw.resync) throw new Error('resync');
        delete raw.closed_candidates; delete raw.full_pending; // Controller-private discovery signal never enters the strict rebased delta schema.
        const delta = rebaseDelta(raw, entry.slot);
        // Morphed child nodes (a new page node under a viewer id) are changed input targets too.
        out.ops.push(...delta.ops, ...await this.sheetOps(entry.child, entry.slot, raw.sheets)); out.changed.push(...delta.changed); out.resources.push(...delta.resources);
        entry.focus = { focused: delta.focused, selection: delta.selection };
        // The child's window position is its header, not an op: forward its changes.
        if (JSON.stringify(delta.scroll) !== entry.scroll) { entry.scroll = JSON.stringify(delta.scroll); out.ops.push({ op: 'scroll', id: entry.documentId, scroll: delta.scroll }); }
        if ((delta.fragment ?? null) !== entry.fragment) { entry.fragment = delta.fragment ?? null; out.ops.push({ op: 'fragment', id: entry.documentId, fragment: entry.fragment }); }
      } catch {
        // New child document (or lost world): fresh snapshot replaces the owner's child.
        stream.frames.delete(owner); stream.frameSlots.delete(entry.slot);
        try {
          const owners = await this.owners(this.mirror.parentWorld(stream));
          const frame = owners.get(owner); if (!frame) continue;
          const slot = ++stream.frameSlot, child = await this.world(this.mirror.parentWorld(stream), frame, stream.id), loaderId = await this.loaderId(child);
          const snap = await this.snapshot(child, loaderId, policy, tab, slot);
          const next = { slot, owner, child, loaderId, documentId: rebaseId(snap.nodes[0].id, slot), scroll: JSON.stringify(snap.scroll), fragment: snap.fragment ?? null, focus: rebaseFocus(snap, slot) };
          stream.frames.set(owner, next); stream.frameSlots.set(slot, next);
          const records = snap.nodes.map(r => rebaseRecord(r, slot)); records[0] = { ...records[0], parent: owner, ...(next.fragment ? { fragment: next.fragment } : {}) };
          out.ops.push({ op: 'children', id: owner, children: [records[0].id], nodes: records }, ...(snap.ops ?? []).map(op => rebaseOp(op, slot)), ...snap.sheetOps);
          if (snap.scroll[0] || snap.scroll[1]) out.ops.push({ op: 'scroll', id: records[0].id, scroll: snap.scroll });
          out.resources.push(...snap.resources.map(d => ({ ...d, key: rebaseKey(d.key, slot), slot })));
        } catch {}
      }
    }
    return out;
  }
  // Focus inside an attached child (the page's focus is its owner): the child's leaf and selection.
  project(stream, source) {
    const entry = source && !source.resync && source.focused ? stream.frames.get(source.focused) : null;
    return entry ? { ...source, focused: entry.focus.focused ?? source.focused, selection: entry.focus.selection ?? source.selection } : source;
  }
  // §2a: child crop bounds are clipped to the current frame content viewport.
  async regionBoxes(stream, parent, tab, policy) {
    const out=[];
    for(const [owner,entry] of stream.frames){
      if(await this.loaderId(entry.child) !== entry.loaderId) continue;
      await this.mirror.protectTargets(entry.child,tab,policy,`frame:${entry.child.frame.id}:${entry.loaderId}:`);
      // A frame owner with no visible box (a 0x0 iframe in a closed dialog) holds no region; it must not fail the page's others.
      const geometry=await this.mirror.evaluate(parent,`${parent.ref}.regionShape(${JSON.stringify(owner)})`).catch(error=>{if(error?.message==='MP-11: mirror observation unavailable')return null;throw error;});
      if(!geometry)continue;
      // The crop maps a child's rectangle into the page by translation only: no region inside a rotated/scaled owner (opacity, filter and blending do not move pixels).
      if(geometry.moved)continue;
      const boxes=await this.mirror.evaluate(entry.child,`${entry.child.ref}.regionBoxes()`);
      if(await this.loaderId(entry.child) !== entry.loaderId) continue;
      const [ox,oy,w,h]=geometry.contentBox,clip=geometry.clip;
      for(const b of boxes){
        const [x,y,bw,bh]=b.clip,left=Math.max(x+ox,ox,clip[0]),top=Math.max(y+oy,oy,clip[1]);
        const right=Math.min(x+ox+bw,ox+w,clip[0]+clip[2]),bottom=Math.min(y+oy+bh,oy+h,clip[1]+clip[3]);
        if(right>left&&bottom>top)out.push({id:rebaseId(b.id,entry.slot),box:[b.box[0]+ox,b.box[1]+oy,b.box[2],b.box[3]],clip:[left,top,right-left,bottom-top],distorted:b.distorted||geometry.distorted,media:b.media===true});
      }
    }
    return out;
  }
  // Viewport boxes of child opaque regions, in top-level coordinates.
  async opaqueBoxes(stream, parent) {
    const out = [];
    for (const [owner, entry] of stream.frames) {
      try {
        const origin = await this.mirror.evaluate(parent, `${parent.ref}.frameOrigin(${JSON.stringify(owner)})`);
        const boxes = await this.mirror.evaluate(entry.child, `${entry.child.ref}.opaqueBoxes()`);
        for (const { id, box, distorted, tag } of boxes) out.push({ id: rebaseId(id, entry.slot), box: [origin[0] + (origin[2] ?? 1) * box[0], origin[1] + (origin[3] ?? 1) * box[1], (origin[2] ?? 1) * box[2], (origin[3] ?? 1) * box[3]], ...(distorted ? { distorted, tag } : {}) });
      } catch {}
    }
    return out;
  }
}
