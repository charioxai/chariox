// MP-08/MP-09/MP-10/MP-11: save the bytes of an image or other resource the page shows into the Room workspace.
// The page's own session fetches it (cookies included), so a signed-in image saves like it shows. Nothing here
// logs or echoes the URL: a URL can carry a token.
import { createHash } from 'node:crypto';
import { assertCurrentDocument, assertNotCancelled, resolveBackendNode } from './browser-controller-actions.mjs';
import { AgentToolRefusal, UserDomainRefusal } from './kernel-browser-refusal.mjs';
import { topLevelNodeId, withLiveNode } from './kernel-browser-node-ref.mjs';
import { planWorkspaceFile } from './kernel-browser-workspace-file.mjs';

export const SAVE_LIMIT = 25 * 1024 * 1024;
const READ_CHUNK = 1024 * 1024;

// The URL a snapshot node shows: an image, media or link target, or a canvas rendered to PNG.
function describeResource() {
  const element = this.nodeType === 1 ? this : this.parentElement;
  if (!element) return null;
  if (element.localName === 'canvas') {
    try { return { canvas: true, url: element.toDataURL('image/png') }; } catch { return { canvas: true, url: null }; }
  }
  for (const value of [element.currentSrc, element.src, element.href]) if (typeof value === 'string' && value) return { url: value };
  return null;
}

async function nodeUrl(host, connection, sessionId, nodeRef) {
  const backendNodeId = topLevelNodeId(nodeRef);
  const answer = await withLiveNode(async () => {
    const objectId = await resolveBackendNode(connection, sessionId, backendNodeId);
    try {
      return (await connection.send('Runtime.callFunctionOn', { objectId, functionDeclaration: describeResource.toString(), returnByValue: true }, sessionId))?.result?.value;
    } finally { await connection.send('Runtime.releaseObject', { objectId }, sessionId).catch(() => {}); }
  });
  // A canvas shows page pixels, which cannot be masked: a tainted one, or any while a protected field is on the page, stays unread.
  if (answer?.canvas && (typeof answer.url !== 'string' || host.protection.targets.length || host.browser.fillTargets?.size)) throw new AgentToolRefusal('save_canvas_unavailable');
  if (typeof answer?.url !== 'string') throw new AgentToolRefusal('save_url_unsupported');
  return answer.url;
}

const cleanMime = value => {
  const media = String(value ?? '').split(';', 1)[0].trim().toLowerCase();
  return /^[a-z0-9][a-z0-9.+-]*\/[a-z0-9][a-z0-9.+-]*$/.test(media) ? media : 'application/octet-stream';
};

function decodeDataUrl(url) {
  const match = /^data:([^,]*),([\s\S]*)$/.exec(url);
  if (!match) throw new AgentToolRefusal('save_url_unsupported');
  const base64 = /;base64$/i.test(match[1]);
  const bytes = base64 ? Buffer.from(match[2], 'base64') : Buffer.from(decodeURIComponent(match[2]), 'utf8');
  if (bytes.length > SAVE_LIMIT) throw new AgentToolRefusal('save_too_large');
  return { bytes, mime: cleanMime(match[1].replace(/;base64$/i, '')) };
}

// A CDP answer that the save's authority may stop waiting for: the request is left to finish, and `late` releases
// what it still returns.
function untilCancelled(pending, signal, late = () => {}) {
  assertNotCancelled(signal);
  if (!signal) return pending;
  return new Promise((resolve, reject) => {
    const stop = () => { pending.then(late, () => {}); try { assertNotCancelled(signal); } catch (error) { reject(error); } };
    signal.addEventListener('abort', stop, { once: true });
    pending.then(resolve, reject).finally(() => signal.removeEventListener('abort', stop));
  });
}

async function fetchWithPageSession(connection, sessionId, url, signal) {
  const { frameTree } = await untilCancelled(connection.send('Page.getFrameTree', {}, sessionId), signal);
  const close = handle => connection.send('IO.close', { handle }, sessionId).catch(() => {});
  const { resource } = await untilCancelled(connection.send('Network.loadNetworkResource', {
    frameId: frameTree.frame.id, url, options: { disableCache: false, includeCredentials: true },
  }, sessionId), signal, answer => { if (typeof answer?.resource?.stream === 'string') close(answer.resource.stream); });
  if (!resource?.success || typeof resource.stream !== 'string') throw new AgentToolRefusal('save_fetch_failed');
  if (resource.httpStatusCode !== undefined && (resource.httpStatusCode < 200 || resource.httpStatusCode > 299)) {
    await close(resource.stream);
    throw new AgentToolRefusal('save_fetch_failed');
  }
  const headers = Object.fromEntries(Object.entries(resource.headers ?? {}).map(([name, value]) => [name.toLowerCase(), value]));
  const chunks = [];
  let total = 0;
  try {
    for (;;) {
      const piece = await untilCancelled(connection.send('IO.read', { handle: resource.stream, size: READ_CHUNK }, sessionId), signal);
      const bytes = piece.base64Encoded ? Buffer.from(piece.data ?? '', 'base64') : Buffer.from(piece.data ?? '', 'utf8');
      total += bytes.length;
      if (total > SAVE_LIMIT) throw new AgentToolRefusal('save_too_large');
      chunks.push(bytes);
      if (piece.eof) break;
    }
  } finally { await close(resource.stream); }
  return { bytes: Buffer.concat(chunks), mime: cleanMime(headers['content-type']) };
}

export async function saveResource(host, tab, command, signal) {
  if ((command.node_ref === undefined) === (command.url === undefined)) throw new AgentToolRefusal('save_source_conflict');
  const plan = await planWorkspaceFile(command._save_roots, command.path);
  const { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
  await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
  const url = command.node_ref !== undefined ? await nodeUrl(host, connection, sessionId, command.node_ref) : command.url;
  if (typeof url !== 'string' || url.length > 8 * 1024 * 1024) throw new AgentToolRefusal('save_url_unsupported');
  let parsed;
  try { parsed = new URL(url); } catch { throw new AgentToolRefusal('save_url_unsupported'); }
  // The same address rule a navigation has: http(s), no credentials in the URL.
  if (!['http:', 'https:', 'data:'].includes(parsed.protocol) || parsed.username || parsed.password) throw new AgentToolRefusal('save_url_unsupported');
  const scheme = parsed.protocol;
  const { bytes, mime } = scheme === 'data:' ? decodeDataUrl(url) : await fetchWithPageSession(connection, sessionId, url, signal);
  const generation = host.generation;
  // The last check before the rename: an ended authority, a replaced document or a restarted browser writes nothing.
  const saved = await plan.commit(bytes, async () => {
    assertNotCancelled(signal);
    await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
    assertNotCancelled(signal);
    if (host.generation !== generation) throw new UserDomainRefusal('stale_epoch');
  });
  return { generation: host.generation, tab_id: tab.tab_id, document_id: tab.document_id, path: saved, bytes: bytes.length,
    mime_type: mime, sha256: createHash('sha256').update(bytes).digest('hex') };
}
