// Card frame identity is host state, alongside restored Tabs; renderer node IDs are not durable.
export const maxCardAnchors = 1_024;
export const cardAnchorLifetimeMs = 15_000;

export function snapshotCardFrames(snapshot, tabUrl, baseURI) {
  const nodes = new Map((snapshot.dom_nodes ?? []).map(node => [node.node_ref, node]));
  const documents = new Map((snapshot.dom_documents ?? []).map(document => [document.document_index, document]));
  const frames = new Map();
  for (const node of snapshot.dom_nodes ?? []) {
    if (node.document_index === 0) continue;
    const owner = nodes.get(documents.get(node.document_index)?.owner_node_ref);
    if (owner?.document_index !== 0 || owner.node_name !== 'IFRAME') continue;
    const attrs = owner.attributes ?? {};
    try {
      frames.set(node.node_ref, { page_origin: new URL(tabUrl).origin, frame_origin: new URL(attrs.src || 'about:blank', baseURI).origin,
        id: attrs.id ?? '', name: attrs.name ?? '', title: attrs.title ?? '' });
    } catch { /* An unresolvable frame address cannot identify a restored frame. */ }
  }
  return frames;
}

export function restoreCardFrames(saved) {
  if (saved === undefined) return new Map();
  if (!Array.isArray(saved) || saved.length > maxCardAnchors || saved.some(entry => !Array.isArray(entry) || entry.length !== 2
    || typeof entry[0] !== 'string' || !entry[1] || typeof entry[1].tab_id !== 'string'
    || !['page_origin','frame_origin','id','name','title'].every(field => typeof entry[1].frame?.[field] === 'string'))) {
    throw new Error('Browser card frame registry is unreadable; preserve the Tab registry for recovery');
  }
  return new Map(saved);
}

export function rememberCardFrame(host, key, tabId, frame) {
  if (!key || !frame) return;
  const next = { tab_id: tabId, frame };
  if (JSON.stringify(host.cardFrames.get(key)) === JSON.stringify(next)) return;
  if (!host.cardFrames.has(key) && host.cardFrames.size >= maxCardAnchors) throw new Error('Browser card limit reached; close an existing browser card and try again');
  host.cardFrames.set(key, next); host.cardFrameRevision++;
}
export function forgetCardFrame(host, key) {
  if (host.cardFrames.delete(key)) host.cardFrameRevision++;
}
export async function saveCardFrames(host) {
  const revision = host.cardFrameRevision;
  if (revision === host.savedCardFrameRevision) return;
  await host.save();
  host.savedCardFrameRevision = revision;
}

// Only a unique iframe with the observed identity in the same page origin is followed.
function findCardFrame(frame) {
  if (location.origin !== frame.page_origin) return null;
  const matches = [...document.querySelectorAll('iframe')].filter(node => {
    try { return node.id === frame.id && node.name === frame.name && node.title === frame.title
      && new URL(node.getAttribute('src') || 'about:blank', document.baseURI).origin === frame.frame_origin; }
    catch { return false; }
  });
  return matches.length === 1 ? matches[0] : null;
}
export async function resolveCardFrame(connection, sessionId, contextId, frame) {
  const { result, exceptionDetails } = await connection.send('Runtime.callFunctionOn', {
    executionContextId: contextId, functionDeclaration: String(findCardFrame), arguments: [{ value: frame }], returnByValue: false,
  }, sessionId);
  if (exceptionDetails) throw new Error('Browser card frame could not be restored');
  if (!result?.objectId) return null;
  try {
    const { node } = await connection.send('DOM.describeNode', { objectId: result.objectId }, sessionId);
    return `backend:${node.backendNodeId}`;
  } finally { await connection.send('Runtime.releaseObject', { objectId: result.objectId }, sessionId).catch(() => {}); }
}
