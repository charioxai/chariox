// MP-08/MP-10/MP-11: cross-origin child frames (plan 2.4). The same v2 observer
// runs in each child frame's own isolated world, reached through the page's
// CDP session (in-process frames) or the frame's flat OOPIF session. Child ids
// and resource keys are rebased per frame slot; a frame that cannot be
// attached stays an opaque region, painted only from masked captures. A child
// can host further cross-origin frames (an ad slot's safeframe and its creative):
// each is attached from its host's world, down to MAX_DEPTH levels.
import { mirror2ObserverExpression } from './kernel-browser-mirror2-observer.mjs';

export const FRAME_SLOT = 1e9, KEY_SLOT = 1e6;
const MAX_FRAMES = 16, MAX_DEPTH = 4, WORLD = 'chariox-mirror-frame';
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
  return apply(record, RECORD(slot), 'record');
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
  // Child frames below this world's document by owner element mirror id (the world's own ids).
  async owners(world) {
    const { frameTree } = await world.connection.send('Page.getFrameTree', {}, world.sessionId);
    // In-process descendants are in the session's frame tree (an in-process child's session is its page's: start at the
    // child's own subtree); out-of-process ones are iframe targets whose frame id is the target id (owned by this world or not).
    const find = node => node.frame.id === world.frame?.id ? node : (node.childFrames ?? []).map(find).find(Boolean);
    const below = node => (node.childFrames ?? []).flatMap(child => [child.frame, ...below(child)]);
    const { targetInfos = [] } = await world.connection.send('Target.getTargets');
    const candidates = [...below(find(frameTree) ?? frameTree), ...targetInfos.filter(t => t.type === 'iframe').map(t => ({ id: t.targetId, url: t.url, oopif: true }))];
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
  // A child document mounted under its owner (an element of `host`, at slot `hostSlot`): its records, ops and resources in
  // viewer ids, with the cross-origin frames it hosts attached below it.
  async mount(host, hostSlot, stream, owner, frame, policy, tab, depth) {
    const slot = ++stream.frameSlot;
    const child = await this.world(host, frame, stream.id);
    const loaderId = await this.loaderId(child);
    const snap = await this.snapshot(child, loaderId, policy, tab, slot);
    const entry = { slot, owner, child, loaderId, hostSlot, depth, documentId: rebaseId(snap.nodes[0].id, slot), scroll: JSON.stringify(snap.scroll), fragment: snap.fragment ?? null, focus: rebaseFocus(snap, slot) };
    stream.frames.set(owner, entry); stream.frameSlots.set(slot, entry);
    const records = snap.nodes.map(r => rebaseRecord(r, slot));
    // The child's own fragment (its header) is its document record's (protocol 503).
    records[0] = { ...records[0], parent: owner, ...(entry.fragment ? { fragment: entry.fragment } : {}) };
    const out = { records, ops: [], resources: snap.resources.map(d => ({ ...d, key: rebaseKey(d.key, slot), slot })) };
    if (snap.scroll[0] || snap.scroll[1]) out.ops.push({ op: 'scroll', id: records[0].id, scroll: snap.scroll });
    out.ops.push(...(snap.ops ?? []).map(op => rebaseOp(op, slot)), ...snap.sheetOps);
    await this.nest(stream, entry, out, records, policy, tab);
    return out;
  }
  // The frames a child's records announce: attached from the child's world, or opaque regions. A nested frame with no
  // area (a tracking pixel, an unfilled slot) stays opaque so it cannot use the stream's frame budget before a visible one.
  async nest(stream, entry, out, records, policy, tab) {
    const foreign = records.filter(r => r.foreign), visible = foreign.filter(r => r.size?.[0] > 0 && r.size?.[1] > 0).map(r => r.id);
    const hidden = foreign.map(r => r.id).filter(id => !visible.includes(id));
    const nested = await this.attach(entry.child, stream, visible, policy, tab, entry.slot, entry.depth + 1).catch(error => { this.failed('nest', error); return { records: [], ops: [], opaque: visible }; });
    this.mirror.opaque(records, [...nested.opaque, ...hidden]);
    out.records.push(...nested.records); out.ops.push(...nested.ops); out.resources.push(...(nested.resources ?? []));
  }
  // Attach every foreign frame named in this packet's records (ids of `hostSlot`'s document); others become
  // opaque regions. Returns rebased child snapshot records (document under owner).
  async attach(host, stream, foreignIds, policy, tab, hostSlot = 0, depth = 1) {
    const out = { records: [], ops: [], opaque: [] };
    if (!foreignIds.length) return out;
    const owners = depth > MAX_DEPTH ? new Map() : await this.owners(host);
    for (const id of foreignIds) {
      const frame = owners.get(hostSlot ? localId(id) : id);
      if (!frame || stream.frames.size >= MAX_FRAMES) { this.failed('unowned', `${owners.size} owners`); out.opaque.push(id); continue; }
      try {
        const mounted = await this.mount(host, hostSlot, stream, id, frame, policy, tab, depth);
        out.records.push(...mounted.records); out.ops.push(...mounted.ops); out.resources = [...(out.resources ?? []), ...mounted.resources];
      } catch (error) { this.failed('attach', error); out.opaque.push(id); }
    }
    return out;
  }
  // The entry and everything mounted below it are gone (navigated child, lost world).
  forget(stream, entry) {
    stream.frames.delete(entry.owner); stream.frameSlots.delete(entry.slot);
    for (const nested of [...stream.frames.values()]) if (nested.hostSlot === entry.slot) this.forget(stream, nested);
  }
  // The world an entry's owner element lives in.
  hostWorld(stream, entry) { return entry.hostSlot ? stream.frameSlots.get(entry.hostSlot)?.child : this.mirror.parentWorld(stream); }
  // Deltas of attached children; a navigated child is re-snapshotted under its owner.
  async drain(stream, policy, tab) {
    const out = { ops: [], resources: [], changed: [] };
    // Each child attached when the credit began, once: a re-mounted child re-enters the map and is drained next credit.
    for (const [owner, entry] of [...stream.frames]) {
      if (stream.frames.get(owner) !== entry) continue; // forgotten with the child that hosted it
      try {
        const loaderId = await this.loaderId(entry.child);
        if (loaderId !== entry.loaderId) throw new Error('navigated');
        let raw = await this.mirror.evaluate(entry.child, `${entry.child.ref}.drain()`, false, undefined, true);
        if (raw.closed_candidates && await this.mirror.markClosedHosts(entry.child, raw)) { raw = this.mirror.merge(raw, await this.mirror.evaluate(entry.child, `${entry.child.ref}.drain()`, false, undefined, true)); }
        if (raw.resync) throw new Error('resync');
        delete raw.closed_candidates; delete raw.full_pending; // Controller-private discovery signal never enters the strict rebased delta schema.
        const delta = rebaseDelta(raw, entry.slot), ops = [], resources = [...delta.resources];
        // A frame the child added is attached from the child's world, like the page's own.
        for (const op of delta.ops) {
          if (op.op !== 'children') { ops.push(op); continue; }
          const added = { records: op.nodes, ops: [], resources: [] };
          await this.nest(stream, entry, added, op.nodes, policy, tab);
          ops.push({ ...op, nodes: added.records }, ...added.ops); resources.push(...added.resources);
        }
        // Morphed child nodes (a new page node under a viewer id) are changed input targets too.
        out.ops.push(...ops, ...await this.sheetOps(entry.child, entry.slot, raw.sheets)); out.changed.push(...delta.changed); out.resources.push(...resources);
        entry.focus = { focused: delta.focused, selection: delta.selection };
        // The child's window position is its header, not an op: forward its changes.
        if (JSON.stringify(delta.scroll) !== entry.scroll) { entry.scroll = JSON.stringify(delta.scroll); out.ops.push({ op: 'scroll', id: entry.documentId, scroll: delta.scroll }); }
        if ((delta.fragment ?? null) !== entry.fragment) { entry.fragment = delta.fragment ?? null; out.ops.push({ op: 'fragment', id: entry.documentId, fragment: entry.fragment }); }
      } catch {
        // New child document (or lost world): fresh snapshot replaces the owner's child.
        this.forget(stream, entry);
        try {
          const host = this.hostWorld(stream, entry); if (!host) continue;
          const frame = (await this.owners(host)).get(entry.hostSlot ? localId(owner) : owner); if (!frame) continue;
          const mounted = await this.mount(host, entry.hostSlot, stream, owner, frame, policy, tab, entry.depth);
          out.ops.push({ op: 'children', id: owner, children: [mounted.records[0].id], nodes: mounted.records }, ...mounted.ops);
          out.resources.push(...mounted.resources);
        } catch {}
      }
    }
    return out;
  }
  // Focus inside an attached child (the page's focus is its owner): the child's leaf and selection, through nested children.
  project(stream, source) {
    if (!source || source.resync) return source;
    let view = source, at = source.focused;
    for (let depth = 0; at && depth <= MAX_DEPTH; depth++) {
      const entry = stream.frames.get(at); if (!entry) break;
      view = { ...view, focused: entry.focus.focused ?? view.focused, selection: entry.focus.selection ?? view.selection }; at = entry.focus.focused;
    }
    return view;
  }
  // §2a: child crop bounds are clipped to the current frame content viewport (a nested child's, to every host's too).
  async regionBoxes(stream, parent, tab, policy) {
    const out=[], placed=new Map(); // slot => the child's content box and visible clip in top-level coordinates
    const intersect=(a,b)=>{const x=Math.max(a[0],b[0]),y=Math.max(a[1],b[1]);return [x,y,Math.max(0,Math.min(a[0]+a[2],b[0]+b[2])-x),Math.max(0,Math.min(a[1]+a[3],b[1]+b[3])-y)];};
    for(const [owner,entry] of stream.frames){
      const host=this.hostWorld(stream,entry),at=entry.hostSlot?placed.get(entry.hostSlot):{offset:[0,0],distorted:false};
      if(!host||!at)continue;
      if(await this.loaderId(entry.child) !== entry.loaderId) continue;
      await this.mirror.protectTargets(entry.child,tab,policy,`frame:${entry.child.frame.id}:${entry.loaderId}:`);
      // A frame owner with no visible box (a 0x0 iframe in a closed dialog) holds no region; it must not fail the page's others.
      const geometry=await this.mirror.evaluate(host,`${host.ref}.regionShape(${JSON.stringify(entry.hostSlot?localId(owner):owner)})`).catch(error=>{if(error?.message==='MP-11: mirror observation unavailable')return null;throw error;});
      if(!geometry)continue;
      // The crop maps a child's rectangle into the page by translation only: no region inside a rotated/scaled owner (opacity, filter and blending do not move pixels).
      if(geometry.moved)continue;
      const [hx,hy]=at.offset,distorted=geometry.distorted||at.distorted,content=[geometry.contentBox[0]+hx,geometry.contentBox[1]+hy,geometry.contentBox[2],geometry.contentBox[3]];
      let clip=[geometry.clip[0]+hx,geometry.clip[1]+hy,geometry.clip[2],geometry.clip[3]];
      if(at.clip)clip=intersect(clip,at.clip);
      placed.set(entry.slot,{offset:[content[0],content[1]],clip:intersect(content,clip),distorted});
      const boxes=(await this.mirror.evaluate(entry.child,`${entry.child.ref}.regionBoxes()`)).filter(b=>!stream.frames.has(rebaseId(b.id,entry.slot)));
      if(await this.loaderId(entry.child) !== entry.loaderId) continue;
      const [ox,oy,w,h]=content;
      for(const b of boxes){
        const [x,y,bw,bh]=b.clip,left=Math.max(x+ox,ox,clip[0]),top=Math.max(y+oy,oy,clip[1]);
        const right=Math.min(x+ox+bw,ox+w,clip[0]+clip[2]),bottom=Math.min(y+oy+bh,oy+h,clip[1]+clip[3]);
        if(right>left&&bottom>top)out.push({id:rebaseId(b.id,entry.slot),box:[b.box[0]+ox,b.box[1]+oy,b.box[2],b.box[3]],clip:[left,top,right-left,bottom-top],distorted:b.distorted||distorted,media:b.media===true});
      }
    }
    return out;
  }
  // A child's content origin and scale in top-level coordinates: its owner in the host's world, then the host's own.
  async origin(stream, entry, top) {
    const host = this.hostWorld(stream, entry) ?? top, own = await this.mirror.evaluate(host, `${host.ref}.frameOrigin(${JSON.stringify(entry.hostSlot ? localId(entry.owner) : entry.owner)})`);
    if (!entry.hostSlot) return own;
    const up = await this.origin(stream, stream.frameSlots.get(entry.hostSlot), top), [sx, sy] = [up[2] ?? 1, up[3] ?? 1];
    return [up[0] + sx * own[0], up[1] + sy * own[1], sx * (own[2] ?? 1), sy * (own[3] ?? 1)];
  }
  // Viewport boxes of child opaque regions, in top-level coordinates.
  async opaqueBoxes(stream, parent) {
    const out = [];
    for (const [owner, entry] of stream.frames) {
      try {
        const origin = await this.origin(stream, entry, parent);
        const boxes = await this.mirror.evaluate(entry.child, `${entry.child.ref}.opaqueBoxes()`);
        for (const { id, box, distorted, tag, foreign } of boxes) if (!foreign || !stream.frames.has(rebaseId(id, entry.slot))) out.push({ id: rebaseId(id, entry.slot), box: [origin[0] + (origin[2] ?? 1) * box[0], origin[1] + (origin[3] ?? 1) * box[1], (origin[2] ?? 1) * box[2], (origin[3] ?? 1) * box[3]], ...(distorted ? { distorted, tag } : {}) });
      } catch {}
    }
    return out;
  }
}
