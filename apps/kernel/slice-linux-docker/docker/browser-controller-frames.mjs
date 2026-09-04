import { BrowserSnapshotError, captureBrowserSnapshot } from "./browser-controller-snapshot.mjs";

const MAX_FRAMES = 64;
const MAX_NODES = 5_000;

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
