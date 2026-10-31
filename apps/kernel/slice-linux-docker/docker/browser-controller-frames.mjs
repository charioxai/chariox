import {BrowserActionError} from './browser-controller-actions.mjs';
import { BrowserSnapshotError, captureBrowserSnapshot } from "./browser-controller-snapshot.mjs";

// MP-08/MP-10/MP-11: an asynchronous inspection may populate a cold cache,
// but it cannot replace a newer navigation/session event with an old result.
export function pageObservationFence(browser, connection, sessionId, targetId, frame) {
  const entry = browser.pageFramesByTarget.get(targetId), document = browser.documentIdsByTarget.get(targetId);
  const owner = browser.currentPageFrame(targetId);
  const stale = () => new BrowserSnapshotError('stale_document_reference', 'browser document observation retired');
  if (owner && (owner.frame.loaderId !== frame.loaderId || owner.frame.id !== frame.id)) throw stale();
  const assertCurrent = () => {
    if (browser.connection !== connection || browser.sessionsByTarget.get(targetId) !== sessionId ||
        browser.pageFramesByTarget.get(targetId) !== entry || browser.documentIdsByTarget.get(targetId) !== document) throw stale();
  };
  assertCurrent();return { owner, assertCurrent };
}

export function captureDocumentOwner(browser, tab, expectedConnection, expectedSession) {
  const target=tab?.target_id, document=tab?.document_id, frame=target&&browser?.currentPageFrame?.(target);
  if (!frame) return null;
  const {connection,sessionId}=frame;
  const current=()=>browser.connection===connection && frame.connection===connection && frame.sessionId===sessionId &&
    browser.currentPageFrame(target)===frame && frame.frame.loaderId===document &&
    (expectedConnection===undefined||expectedConnection===connection) && (expectedSession===undefined||expectedSession===sessionId);
  const assertCurrent=()=>{if(!current())throw new BrowserActionError('stale_document_reference','browser capture owner retired');};
  assertCurrent();return {connection,sessionId,assertCurrent};
}

// Native state is metadata only. A pending page keeps its last committed id;
// this never populates the frame-owner cache or admits pixels/input.
export function nativeStatePage(browser, connection, sessionId, target, visibleTargetId, configured) {
  if (!configured || typeof visibleTargetId !== 'string' || browser.connection !== connection ||
      browser.sessionsByTarget.get(target.targetId) !== sessionId) return null;
  const document = browser.currentPageFrame(target.targetId)?.frame.loaderId ?? browser.documentIdsByTarget.get(target.targetId);
  if (typeof document !== 'string' || !document) return null;
  return { target_id: target.targetId, ...(typeof target.openerId === 'string' ? {opener_target_id:target.openerId} : {}),
    document_id: document, url: typeof target.url === 'string' ? target.url : '',
    title: typeof target.title === 'string' ? target.title : '', focused: target.targetId === visibleTargetId };
}

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
  // MP-11: the flat session that renders an isolated frame (its target id).
  sessionFor(frameId, connection) {
    for (const [id, entry] of this.sessions) if (entry.frameId === frameId && entry.connection === connection) return id;
    return null;
  }
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
      const entry = { targetId, frameId: params.targetInfo.targetId, parentSessionId: message.sessionId, connection };
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
  // MP-08/MP-10/MP-11: ordinary state/input preflight is not a structured
  // snapshot. Global advertising iframe counts must not disable a live tab.
  // Page/isolated-session lifecycle listeners own removal and nested frames;
  // merge this page tree without dropping their live isolated descendants.
  // The registry still caps attribution at MAX_FRAMES (unknown downloads fail
  // closed), and explicit snapshots/actions retain withBrowserFrames bounds.
  const { frameTree } = await connection.send("Page.getFrameTree", {}, sessionId);
  if (frameTree?.frame?.loaderId !== documentId) {
    throw new BrowserSnapshotError("stale_document_reference", "browser top document changed");
  }
  registry.merge(targetId, [frameTree]);
}

// A renderer's backend node IDs are not browser-wide IDs. Keep the owning
// frame and document in references returned for isolated renderers.
const prefix = (frame) => `frame:${frame.id}:${frame.loaderId}:`;

// Reads the frames this page owns: its own tree and, going down from it, every isolated renderer whose parent
// frame is already owned. The browser's other iframe targets (other tabs, advertising frames) never count against
// this page. A page that owns more than MAX_FRAMES frames is cut there and `info.truncated` says so; `strict`
// callers (they must see every frame to protect it) get snapshot_frame_limit instead.
export async function withBrowserFrames(connection, sessionId, targetId, documentId, run, { strict = false } = {}) {
  const attached = [];
  try {
    const top = await connection.send("Page.getFrameTree", {}, sessionId);
    if (top.frameTree?.frame?.loaderId !== documentId) {
      throw new BrowserSnapshotError("stale_document_reference", "browser top document changed");
    }
    const root = { sessionId, tree: top.frameTree, frame: top.frameTree.frame, prefix: "" };
    const owned = [root];
    const owners = new Map();
    const info = { truncated: false };
    const remember = (entry, tree) => {
      if (owners.size >= MAX_FRAMES) {
        if (strict) throw new BrowserSnapshotError("snapshot_frame_limit", "browser exceeded frame bound");
        info.truncated = true;
        return;
      }
      owners.set(tree.frame.id, entry);
      for (const child of tree.childFrames ?? []) remember(entry, child);
    };
    remember(root, root.tree);
    const { targetInfos = [] } = await connection.send("Target.getTargets");
    const pending = targetInfos.filter((target) => target.type === "iframe");
    for (let changed = true; changed && !info.truncated;) {
      changed = false;
      for (const target of [...pending]) {
        if (info.truncated) break;
        if (!owners.has(target.parentFrameId)) continue;
        pending.splice(pending.indexOf(target), 1);
        const { sessionId: childSession } = await connection.send("Target.attachToTarget", { targetId: target.targetId, flatten: true });
        attached.push(childSession);
        const { frameTree } = await connection.send("Page.getFrameTree", {}, childSession);
        const entry = { sessionId: childSession, tree: frameTree, frame: frameTree.frame, prefix: prefix(frameTree.frame), parent: owners.get(target.parentFrameId) };
        owned.push(entry);
        remember(entry, entry.tree);
        changed = true;
      }
    }
    return await run(owned, info);
  } finally {
    await Promise.all(attached.map((childSession) => connection.send("Target.detachFromTarget", { sessionId: childSession }).catch(() => {})));
  }
}

export async function captureBrowserFrames(options) {
  const { connection, sessionId, targetId, documentId } = options;
  return withBrowserFrames(connection, sessionId, targetId, documentId, async (frames, info) => {
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
    await assertBrowserFramesUnchanged(connection, frames);
    if (info.truncated) result.accessibility_truncated = true;
    return result;
  });
}

// MP-08/MP-10/MP-11: composited images and semantic snapshots share the same
// document fence, including local descendants and isolated renderer parents.
export async function assertBrowserFramesUnchanged(connection, frames) {
  for (const entry of frames) {
    const current = await connection.send("Page.getFrameTree", {}, entry.sessionId);
    if (frameIdentity(current.frameTree) !== frameIdentity(entry.tree)) {
      throw new BrowserSnapshotError("stale_document_reference", "browser frame tree changed during capture");
    }
  }
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
