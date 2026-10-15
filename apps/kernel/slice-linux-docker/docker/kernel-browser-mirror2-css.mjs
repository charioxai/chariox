// MP-08/MP-10: CDP CSS events replace stylesheet polling. CSSOM edits (insertRule,
// rule.style writes, replace/replaceSync) make no mutation record; Chrome's CSS
// agent reports them as styleSheetChanged. A changed sheet marks its owner node
// (constructed sheets: the adopted lists; imported sheets: sheets with imports)
// in every observer instance of its frame's isolated world, which wakes the
// waiting credit. Disabled toggles and media lists stay with the per-drain
// signature (Chrome reports `disabled` as removal, media edits not at all).
const MARK = 'function(){return globalThis.__charioxMirror2?.sheetChanged(this) ?? false}';

export class MirrorCssEvents {
  constructor() { this.sessions = new Map(); }
  pending(sessionId) { const entry = this.sessions.get(sessionId); return !!entry && !entry.enabled; }
  // A world (page or frame session) whose frame's sheets this subscription mirrors.
  async watch(world, frameId, streamId) {
    let entry = this.sessions.get(world.sessionId);
    if (entry?.connection !== world.connection) {
      entry?.stop();
      entry = { connection: world.connection, sessionId: world.sessionId, worlds: new Map(), headers: new Map(), changed: new Set(), streams: new Set(), timer: null };
      const off = world.connection.subscribe(message => this.event(entry, message));
      entry.stop = () => { entry.stopped = true; off(); clearTimeout(entry.timer); if (this.sessions.get(entry.sessionId) === entry) this.sessions.delete(entry.sessionId); };
      this.sessions.set(world.sessionId, entry);
    }
    entry.worlds.set(frameId, world.contextId); entry.streams.add(streamId);
    entry.ready ??= (async () => {
      try {
        await world.connection.send('DOM.enable', {}, world.sessionId);
        // CSS.enable waits for resource bodies, independently of DOM readiness.
        // Large pages start this bounded watcher without delaying first paint.
        await world.connection.send('CSS.enable', {}, world.sessionId, { timeoutMs: 30000 });
        if (entry.stopped) return;
        entry.enabled = true;
        // Re-read initial owners: a CSSOM edit between snapshot and activation
        // must not be lost merely because it preceded styleSheetChanged events.
        for (const id of entry.headers.keys()) entry.changed.add(id);
        await this.flush(entry);
      } catch (error) { entry.stop(); throw error; }
    })();
    await entry.ready;
  }
  // The last subscription of a session stops its CSS agent (best effort: a gone session has none).
  release(streamId) {
    for (const entry of [...this.sessions.values()]) {
      if (!entry.streams.delete(streamId) || entry.streams.size) continue;
      entry.stop();
      for (const method of ['CSS.disable', 'DOM.disable']) entry.connection.send(method, {}, entry.sessionId).catch(() => {});
    }
  }
  event(entry, message) {
    if (message.sessionId !== entry.sessionId) return;
    const { method, params } = message;
    if (method === 'CSS.styleSheetAdded') {
      const header = params?.header; if (!header?.styleSheetId) return;
      entry.headers.set(header.styleSheetId, { frameId: header.frameId, owner: header.ownerNode, constructed: header.isConstructed === true });
      if (entry.headers.size > 20000) entry.headers.delete(entry.headers.keys().next().value);
    } else if (method === 'CSS.styleSheetRemoved') entry.headers.delete(params?.styleSheetId);
    else if (method === 'CSS.styleSheetChanged' && entry.headers.has(params?.styleSheetId)) {
      entry.changed.add(params.styleSheetId);
      entry.timer ??= setTimeout(() => { entry.timer = null; void this.flush(entry); }, 0);
    }
  }
  // Coalesced per task: each changed owner is marked once per world. A frame without its own world (a
  // same-origin child document, observed by an ancestor's world) is marked in every world of the session:
  // only the world mirroring that document knows the owner.
  async flush(entry) {
    const marks = new Map();
    for (const id of entry.changed) {
      const header = entry.headers.get(id); if (!header) continue;
      const contexts = entry.worlds.has(header.frameId) ? [entry.worlds.get(header.frameId)] : new Set(entry.worlds.values());
      for (const contextId of contexts) {
        if (!marks.has(contextId)) marks.set(contextId, new Set());
        marks.get(contextId).add(header.constructed ? 'adopted' : Number.isSafeInteger(header.owner) ? header.owner : 'imports');
      }
    }
    entry.changed.clear();
    const send = (method, params) => entry.connection.send(method, params, entry.sessionId);
    for (const [contextId, owners] of marks) for (const owner of owners) {
      try {
        if (typeof owner === 'string') { await send('Runtime.evaluate', { expression: `globalThis.__charioxMirror2?.sheetChanged(${JSON.stringify(owner)})`, contextId, returnByValue: true }); continue; }
        const { object } = await send('DOM.resolveNode', { backendNodeId: owner, executionContextId: contextId });
        await send('Runtime.callFunctionOn', { objectId: object.objectId, functionDeclaration: MARK, returnByValue: true });
        await send('Runtime.releaseObject', { objectId: object.objectId }).catch(() => {});
      } catch {} // a departed node or world: its next snapshot reads the sheet anyway
    }
  }
}
