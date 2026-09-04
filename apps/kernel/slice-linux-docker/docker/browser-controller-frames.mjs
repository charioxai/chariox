import { BrowserSnapshotError, captureBrowserSnapshot } from "./browser-controller-snapshot.mjs";

const MAX_FRAMES = 64;
const MAX_NODES = 5_000;

// Browser download events carry a frame ID, not a page session. Retain the
// full page tree so child-frame downloads have the same tab identity.
export class BrowserFrameTargets {
  constructor() { this.frames = new Map(); }
  clear() { this.frames.clear(); }
  get(frameId) { return this.frames.get(frameId)?.targetId; }
  removeTarget(targetId) {
    for (const [id, frame] of this.frames) if (frame.targetId === targetId) this.frames.delete(id);
  }
  replace(targetId, trees) {
    this.removeTarget(targetId);
    this.merge(targetId, trees);
  }
  merge(targetId, trees) {
    const visit = (node) => {
      if (!this.add(targetId, node?.frame?.id, node?.frame?.parentId)) return;
      for (const child of node.childFrames ?? []) visit(child);
    };
    for (const tree of trees) visit(tree);
  }
  record(message, targetId, rootSession = true) {
    if (!targetId) return;
    const params = message.params ?? {};
    if (message.method === "Page.frameAttached") this.add(targetId, params.frameId, params.parentFrameId);
    if (message.method === "Page.frameNavigated") {
      const frame = params.frame;
      if (!frame?.parentId && rootSession) this.removeTarget(targetId);
      else this.remove(frame.id);
      this.add(targetId, frame?.id, frame?.parentId);
    }
    if (message.method === "Page.frameDetached" && params.reason !== "swap") this.remove(params.frameId);
  }
  add(targetId, frameId, parentId) {
    if (typeof frameId !== "string" || !frameId) return false;
    if (!this.frames.has(frameId) && [...this.frames.values()].filter((frame) => frame.targetId === targetId).length >= MAX_FRAMES) return false;
    this.frames.set(frameId, { targetId, parentId });
    return true;
  }
  remove(frameId) {
    const removed = new Set([frameId]);
    for (let changed = true; changed;) {
      changed = false;
      for (const [id, frame] of this.frames) {
        if (removed.has(id) || removed.has(frame.parentId)) {
          removed.add(id);
          this.frames.delete(id);
          changed = true;
        }
      }
    }
  }
}

// Keep nested isolated-frame lifecycle events observable between snapshots.
// Only iframe targets are attached. A paused child is always resumed, including
// setup failure, overflow, root removal and controller shutdown.
export class BrowserFrameSessions {
  constructor(registry, rootTargetForSession) {
    this.registry = registry;
    this.rootTargetForSession = rootTargetForSession;
    this.sessions = new Map();
    this.pending = new Set();
  }
  clear() { this.sessions.clear(); }
  removeTarget(targetId) {
    return this.removeWhere((entry) => entry.targetId === targetId);
  }
  close() { return this.removeWhere(() => true); }
  async removeWhere(matches) {
    const removed = [];
    for (const [id, entry] of this.sessions) {
      if (!matches(entry, id)) continue;
      this.sessions.delete(id);
      removed.push(this.release(entry.connection, id, true));
    }
    await Promise.all(removed);
  }
  removeSession(sessionId) {
    const removed = new Set([sessionId]);
    for (let changed = true; changed;) {
      changed = false;
      for (const [id, entry] of this.sessions) {
        if (!removed.has(id) && removed.has(entry.parentSessionId)) { removed.add(id); changed = true; }
      }
    }
    return this.removeWhere((_entry, id) => removed.has(id));
  }
  start(connection, sessionId) {
    return connection.send("Target.setAutoAttach", {
      autoAttach: true, waitForDebuggerOnStart: true, flatten: true,
      filter: [{ type: "iframe" }, { exclude: true }],
    }, sessionId);
  }
  observe(message, connection) {
    const params = message?.params ?? {};
    if (message?.method === "Target.attachedToTarget" && params.targetInfo?.type === "iframe") {
      const targetId = this.sessions.get(message.sessionId)?.targetId ?? this.rootTargetForSession(message.sessionId);
      const childSession = params.sessionId;
      if (typeof childSession !== "string" || !childSession) return false;
      if (!targetId) {
        if (params.waitingForDebugger !== true) return false;
        void this.release(connection, childSession, true);
        return true;
      }
      const entry = { targetId, parentSessionId: message.sessionId, connection };
      if (this.pending.size >= MAX_FRAMES || [...this.sessions.values()].filter((current) => current.targetId === targetId).length >= MAX_FRAMES) {
        void this.release(connection, childSession, true);
        return true;
      }
      this.sessions.set(childSession, entry);
      const pending = this.initialize(connection, childSession, entry);
      this.pending.add(pending);
      void pending.finally(() => this.pending.delete(pending));
      return true;
    }
    if (message?.method === "Target.detachedFromTarget" && this.sessions.has(params.sessionId)) {
      void this.removeSession(params.sessionId);
      return true;
    }
    const entry = this.sessions.get(message?.sessionId);
    if (!entry) return false;
    this.registry.record(message, entry.targetId, false);
    return true;
  }
  async initialize(connection, sessionId, entry) {
    let failed = false;
    try {
      await connection.send("Page.enable", {}, sessionId);
      const { frameTree } = await connection.send("Page.getFrameTree", {}, sessionId);
      if (this.sessions.get(sessionId) !== entry) return;
      this.registry.merge(entry.targetId, [frameTree]);
      await this.start(connection, sessionId);
    } catch {
      failed = true;
    } finally {
      const removed = this.sessions.get(sessionId) !== entry;
      if (failed && !removed) this.sessions.delete(sessionId);
      await this.release(connection, sessionId, failed || removed);
    }
  }
  async release(connection, sessionId, detach) {
    await connection.send("Runtime.runIfWaitingForDebugger", {}, sessionId).catch(() => {});
    if (detach) await connection.send("Target.detachFromTarget", { sessionId }).catch(() => {});
  }
}

export async function registerBrowserFrameTargets(connection, sessionId, targetId, documentId, registry) {
  return withBrowserFrames(connection, sessionId, targetId, documentId, (frames) => {
    registry.replace(targetId, frames.map((entry) => entry.tree));
  });
}

// A renderer's backend node IDs are not browser-wide IDs. Keep the owning
// frame and document in references returned for isolated renderers.
const prefix = (frame) => `frame:${frame.id}:${frame.loaderId}:`;

export async function withBrowserFrames(connection, sessionId, targetId, documentId, run) {
  const attached = [];
  try {
    const top = await connection.send("Page.getFrameTree", {}, sessionId);
    if (top.frameTree?.frame?.loaderId !== documentId) {
      throw new BrowserSnapshotError("stale_document_reference", "browser top document changed");
    }
    const root = { sessionId, tree: top.frameTree, frame: top.frameTree.frame, prefix: "" };
    const { targetInfos = [] } = await connection.send("Target.getTargets");
    const candidates = targetInfos.filter((target) => target.type === "iframe");
    if (candidates.length >= MAX_FRAMES) {
      throw new BrowserSnapshotError("snapshot_frame_limit", "browser exceeded isolated frame bound");
    }
    const frames = [root];
    for (const target of candidates) {
      const { sessionId: childSession } = await connection.send("Target.attachToTarget", { targetId: target.targetId, flatten: true });
      attached.push(childSession);
      const { frameTree } = await connection.send("Page.getFrameTree", {}, childSession);
      frames.push({ sessionId: childSession, tree: frameTree, frame: frameTree.frame, prefix: prefix(frameTree.frame) });
    }
    const owned = [root];
    const owners = new Map();
    const remember = (entry, tree) => {
      owners.set(tree.frame.id, entry);
      if (owners.size > MAX_FRAMES) throw new BrowserSnapshotError("snapshot_frame_limit", "browser exceeded frame bound");
      for (const child of tree.childFrames ?? []) remember(entry, child);
    };
    remember(root, root.tree);
    for (let changed = true; changed;) {
      changed = false;
      for (const entry of frames.slice(1)) {
        if (!owned.includes(entry) && owners.has(entry.frame.parentId)) {
          entry.parent = owners.get(entry.frame.parentId);
          owned.push(entry);
          remember(entry, entry.tree);
          changed = true;
        }
      }
    }
    const result = await run(owned);
    return result;
  } finally {
    await Promise.all(attached.map((childSession) => connection.send("Target.detachFromTarget", { sessionId: childSession }).catch(() => {})));
  }
}

export async function captureBrowserFrames(options) {
  const { connection, sessionId, targetId, documentId } = options;
  return withBrowserFrames(connection, sessionId, targetId, documentId, async (frames) => {
    let result;
    for (const entry of frames) {
      const remaining = MAX_NODES - Math.max(result?.accessibility_nodes.length ?? 0, result?.dom_nodes.length ?? 0);
      if (remaining <= 0) break;
      const snapshot = await captureBrowserSnapshot({ ...options, sessionId: entry.sessionId, documentId: entry.frame.loaderId, limits: { maxNodes: remaining } });
      if (!result) { result = snapshot; continue; }
      const ref = (value) => value ? entry.prefix + value : null;
      const offset = result.dom_documents.length;
      const owner = await connection.send("DOM.getFrameOwner", { frameId: entry.frame.id }, entry.parent.sessionId);
      result.accessibility_nodes.push(...snapshot.accessibility_nodes.map((node) => ({ ...node, node_ref: ref(node.node_ref), parent_ref: ref(node.parent_ref), child_refs: node.child_refs.map(ref) })));
      result.dom_nodes.push(...snapshot.dom_nodes.map((node) => ({ ...node, node_ref: ref(node.node_ref), parent_ref: ref(node.parent_ref), document_index: node.document_index + offset })));
      result.dom_documents.push(...snapshot.dom_documents.map((document) => ({ ...document, document_index: document.document_index + offset, owner_node_ref: document.owner_node_ref ? ref(document.owner_node_ref) : `${entry.parent.prefix}backend:${owner.backendNodeId}` })));
      result.shadow_roots.push(...snapshot.shadow_roots.map((node) => ({ ...node, node_ref: ref(node.node_ref) })));
    }
    for (const entry of frames) {
      const current = await connection.send("Page.getFrameTree", {}, entry.sessionId);
      if (frameIdentity(current.frameTree) !== frameIdentity(entry.tree)) {
        throw new BrowserSnapshotError("stale_document_reference", "browser frame tree changed during snapshot capture");
      }
    }
    return result;
  });
}

export async function withBrowserActionFrame(options, run) {
  const { connection, sessionId, targetId, documentId, nodeRef } = options;
  if (typeof nodeRef !== "string" || !nodeRef.startsWith("frame:")) return run(options);
  return withBrowserFrames(connection, sessionId, targetId, documentId, async (frames) => {
    const entry = frames.find((frame) => frame.prefix && nodeRef.startsWith(frame.prefix));
    if (!entry) throw new BrowserSnapshotError("stale_element_reference", "isolated frame changed; capture a fresh snapshot before retrying");
    const assertContext = async () => {
      for (let parent = entry.parent; parent; parent = parent.parent) {
        const current = await connection.send("Page.getFrameTree", {}, parent.sessionId);
        if (frameIdentity(current.frameTree) !== frameIdentity(parent.tree)) {
          throw new BrowserSnapshotError("stale_document_reference", "browser parent frame changed during action");
        }
      }
    };
    const result = await run({ ...options, sessionId: entry.sessionId, documentId: entry.frame.loaderId, nodeRef: nodeRef.slice(entry.prefix.length), assertContext });
    return { ...result, target_id: targetId, document_id: documentId };
  });
}

function frameIdentity(tree) {
  return JSON.stringify([tree?.frame?.id, tree?.frame?.loaderId, tree?.frame?.parentId, (tree?.childFrames ?? []).map(frameIdentity)]);
}
