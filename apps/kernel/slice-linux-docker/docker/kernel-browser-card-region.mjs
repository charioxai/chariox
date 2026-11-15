// MP-08 / MP-09 / MP-10 / MP-11: person cards reference an observed element,
// while their pixels stay on the existing protected Tab display.
import { UserDomainRefusal } from './kernel-browser-refusal.mjs';
import { withBrowserFrames, assertBrowserFramesUnchanged } from './browser-controller-frames.mjs';
import { setTimeout as delay } from 'node:timers/promises';
import { BrowserSnapshotError } from './browser-controller-snapshot.mjs';
import { BrowserCardVerification, validateCardVerification } from './kernel-browser-card-verification.mjs';

const validReference = ref => typeof ref === 'string' && /^(?:frame:.+:)?backend:[1-9][0-9]{0,15}$/.test(ref);
const anchorLifetimeMs = 15_000;
const maxAnchors = 1_024;

export function forgetCardAnchor(host, id) {
  host.cardAnchors.get(id)?.verification?.close();
  host.cardAnchors.get(id)?.releaseElement?.();
  host.cardAnchors.delete(id);
}

// A framework may replace a complete challenge form in the same DOM slot.
// Follow only its unique stable id in the original parent and document.
function trackCardElement() {
  const original = this, parent = this.parentElement, id = this.id, tag = this.localName;
  return () => {
    if (original.isConnected && original.ownerDocument === document) return original;
    if (!id || !parent?.isConnected || parent.ownerDocument !== document) return null;
    const matches = document.querySelectorAll('#' + CSS.escape(id));
    if (matches.length !== 1) return null;
    const node = matches[0];
    return node.parentElement === parent && node.localName === tag ? node : null;
  };
}

export function pruneCardAnchors(host, now = Date.now()) {
  for (const [id, anchor] of host.cardAnchors) if (anchor.expires <= now) forgetCardAnchor(host, id);
}

export function rememberCardReferences(host, scope, tab, snapshot) {
  pruneCardAnchors(host);
  let tabs = host.cardReferences.get(scope);
  if (!tabs) {
    if (host.cardReferences.size >= 64) host.cardReferences.delete(host.cardReferences.keys().next().value);
    host.cardReferences.set(scope, tabs = new Map());
  }
  const previous = tabs.get(tab.tab_id);
  const refs = previous?.generation === host.generation && previous.document_id === tab.document_id ? previous.refs : new Set();
  const children = previous?.generation === host.generation && previous.document_id === tab.document_id ? previous.children : new Set();
  const topReferences = new Set((snapshot.dom_nodes ?? []).filter(node => node.document_index === 0).map(node => node.node_ref));
  for (const node of [...(snapshot.dom_nodes ?? []), ...(snapshot.accessibility_nodes ?? [])]) {
    if (validReference(node.node_ref)) {
      refs.add(node.node_ref);
      // AX can retain a node cut by the DOM snapshot bound. Box quads also
      // cover that case without guessing which same-renderer document owns it.
      if (!topReferences.has(node.node_ref)) children.add(node.node_ref);
      else children.delete(node.node_ref);
    }
  }
  while (refs.size > 10_000) { const ref = refs.values().next().value; refs.delete(ref); children.delete(ref); }
  tabs.set(tab.tab_id, { generation: host.generation, document_id: tab.document_id, refs, children });
}

// DOM box quads already place same-renderer children in their renderer's
// viewport. Isolated renderers use the same affine frame-owner mapping as the
// browser's protected-field geometry, through the shared frame ownership walk.
async function childFrameBox(browser, connection, sessionId, tab, command) {
  if (command.after) await delay(100);
  return withBrowserFrames(connection, sessionId, tab.target_id, tab.document_id, async frames => {
    const entry = frames.find(f => f.prefix && command.ref.startsWith(f.prefix)) ?? (command.ref.startsWith('backend:') ? frames[0] : null);
    if (!entry) return null;
    const backendNodeId = Number(command.ref.slice(entry.prefix.length + 8));
    const root = frames[0], world = await browser.ensureFocusWorld(connection, sessionId, tab.target_id, root.frame);
    const { result: viewport } = await connection.send('Runtime.evaluate', { contextId: world.contextId, returnByValue: true, expression: '[innerWidth,innerHeight]' }, sessionId);
    const [width, height] = viewport?.value ?? [];
    if (!(width > 0 && height > 0)) throw new Error('Browser card viewport unavailable');
    const childWorld = entry === root ? world.contextId : (await connection.send('Page.createIsolatedWorld', { frameId: entry.frame.id, worldName: 'chariox-card-viewport', grantUniveralAccess: false }, entry.sessionId)).executionContextId;
    const { object } = await connection.send('DOM.resolveNode', { backendNodeId, executionContextId: childWorld }, entry.sessionId);
    if (!object?.objectId) return null;
    try {
      const { result, exceptionDetails } = await connection.send('Runtime.callFunctionOn', { objectId: object.objectId, functionDeclaration: 'function() { return this.isConnected; }', arguments: [{ value: null }], returnByValue: true }, entry.sessionId);
      if (exceptionDetails || typeof result?.value !== 'boolean') throw new Error('Browser card element connection could not be read');
      if (!result.value) return null;
    } finally { await connection.send('Runtime.releaseObject', { objectId: object.objectId }, entry.sessionId).catch(() => {}); }
    let model;
    try { ({ model } = await connection.send('DOM.getBoxModel', { backendNodeId }, entry.sessionId)); }
    catch (error) {
      if (!/Could not compute box model/.test(error?.message ?? '')) throw error;
      await assertBrowserFramesUnchanged(connection, frames);
      return { rect: { x: 0, y: 0, width: 0, height: 0 }, viewport_width: width, viewport_height: height };
    }
    const transforms = new Map([[frames[0], point => point]]);
    const toViewport = async child => {
      if (transforms.has(child)) return transforms.get(child);
      const up = await toViewport(child.parent);
      const owner = await connection.send('DOM.getFrameOwner', { frameId: child.frame.id }, child.parent.sessionId);
      const { model: parentModel } = await connection.send('DOM.getBoxModel', { backendNodeId: owner.backendNodeId }, child.parent.sessionId);
      const quad = parentModel.content;
      const { executionContextId } = await connection.send('Page.createIsolatedWorld', { frameId: child.frame.id, worldName: 'chariox-card-viewport', grantUniveralAccess: false }, child.sessionId);
      const { result } = await connection.send('Runtime.evaluate', { contextId: executionContextId, returnByValue: true, expression: '[innerWidth,innerHeight]' }, child.sessionId);
      const [width, height] = result?.value ?? [];
      if (!(width > 0 && height > 0) || !Array.isArray(quad) || quad.length !== 8 || !quad.every(Number.isFinite) || Math.abs(quad[0] + quad[4] - quad[2] - quad[6]) > .1 || Math.abs(quad[1] + quad[5] - quad[3] - quad[7]) > .1) throw new Error('Browser card frame transform unavailable');
      const map = ([x, y]) => up([quad[0] + x * (quad[2] - quad[0]) / width + y * (quad[6] - quad[0]) / height, quad[1] + x * (quad[3] - quad[1]) / width + y * (quad[7] - quad[1]) / height]);
      transforms.set(child, map);
      return map;
    };
    if (!Array.isArray(model?.border) || model.border.length !== 8 || !model.border.every(Number.isFinite)) throw new Error('Invalid browser card frame geometry');
    const map = await toViewport(entry), points = [];
    for (let i = 0; i < 8; i += 2) points.push(map(model.border.slice(i, i + 2)));
    const x = Math.max(0, Math.min(width, Math.floor(Math.min(...points.map(p => p[0])) - 16)));
    const y = Math.max(0, Math.min(height, Math.floor(Math.min(...points.map(p => p[1])) - 16)));
    const right = Math.max(x, Math.min(width, Math.ceil(Math.max(...points.map(p => p[0])) + 16)));
    const bottom = Math.max(y, Math.min(height, Math.ceil(Math.max(...points.map(p => p[1])) + 16)));
    await assertBrowserFramesUnchanged(connection, frames);
    return { rect: { x, y, width: right - x, height: bottom - y }, viewport_width: width, viewport_height: height };
  });
}

// Runs in a controller-owned isolated world. A long read returns on the next
// geometry change, including scroll, resize, animation and element removal.
// The timer also settles hidden documents where animation frames are paused.
function watchBox(previous, resolveAnchor) {
  const original = this;
  const read = () => {
    const node = resolveAnchor ? resolveAnchor() : original;
    if (!node?.isConnected || node.ownerDocument !== document) return null;
    const r = node.getBoundingClientRect(), w = innerWidth, h = innerHeight;
    if (r.right <= r.left || r.bottom <= r.top) return { rect: { x: 0, y: 0, width: 0, height: 0 }, viewport_width: w, viewport_height: h };
    const x = Math.max(0, Math.min(w, Math.floor(r.left - 16)));
    const y = Math.max(0, Math.min(h, Math.floor(r.top - 16)));
    const right = Math.max(x, Math.min(w, Math.ceil(r.right + 16)));
    const bottom = Math.max(y, Math.min(h, Math.ceil(r.bottom + 16)));
    return { rect: { x, y, width: right - x, height: bottom - y }, viewport_width: w, viewport_height: h };
  };
  const first = read();
  if (!previous || JSON.stringify(first) !== JSON.stringify(previous)) return first;
  return new Promise(resolve => {
    let frame;
    const done = value => { cancelAnimationFrame(frame); clearTimeout(timer); resolve(value); };
    const tick = () => {
      const value = read();
      if (JSON.stringify(value) !== JSON.stringify(previous)) done(value);
      else frame = requestAnimationFrame(tick);
    };
    const timer = setTimeout(() => done(read()), 1_000);
    frame = requestAnimationFrame(tick);
  });
}

// An App's confirmation preview is found by the host itself in the Tab's top document.
// The App never names the element: a page that holds more than one candidate shows nothing.
const confirmationSelector = '[data-chariox-confirmation]';
async function confirmationReference(host, scope, command) {
  const tab = await host.displayTarget({ tab_id: command.tab_id, generation: command.generation });
  const connection = await host.browser.ensureConnection();
  const sessionId = await host.browser.ensureTargetSession(connection, tab.target_id);
  const { root } = await connection.send('DOM.getDocument', { depth: 0 }, sessionId);
  const { nodeIds } = await connection.send('DOM.querySelectorAll', { nodeId: root.nodeId, selector: confirmationSelector }, sessionId);
  if (nodeIds?.length !== 1) throw new UserDomainRefusal('stale_reference');
  const { node } = await connection.send('DOM.describeNode', { nodeId: nodeIds[0] }, sessionId);
  const ref = `backend:${node.backendNodeId}`;
  rememberCardReferences(host, scope, tab, { dom_nodes: [{ document_index: 0, node_ref: ref }] });
  return ref;
}

export async function resolveCardRegion(host, command, scope, signal) {
  pruneCardAnchors(host);
  const key = typeof command.card_id === 'string' ? `${scope}\0${command.card_id}` : null;
  let anchor = key ? host.cardAnchors.get(key) : null;
  if (command.app_confirmation === true) {
    if (!key || command.release === true) throw new UserDomainRefusal('stale_reference');
    command = { ...command, ref: anchor?.ref ?? await confirmationReference(host, scope, command) };
  }
  if (command.release === true) {
    if (anchor?.tab_id === command.tab_id) forgetCardAnchor(host, key);
    return { released: true };
  }
  if (anchor && (anchor.tab_id !== command.tab_id || anchor.ref !== command.ref)) throw new UserDomainRefusal('stale_reference');
  const remembered = anchor?.binding ?? host.cardReferences.get(scope)?.get(command.tab_id);
  const completion = () => ({state:'condition_met',reason:JSON.stringify({reason:'Verification accepted',card_id:command.card_id,tab_id:command.tab_id,generation:anchor.binding.generation,document_id:anchor.binding.document_id,verification_result:anchor.verification.receipt})});
  const gone = async reason => {
    if (!anchor?.verification) return {state:'condition_met',reason};
    if (anchor.verification.receipt?.result === 'accepted') return completion();
    return {state:'unavailable',reason};
  };
  if ((!remembered || !remembered.refs.has(command.ref)) && command.document_id !== undefined) {
    const tab = await host.displayTarget({ tab_id: command.tab_id, generation: command.generation });
    if (tab.document_id !== command.document_id) return gone('Browser document changed');
    return { state: 'unavailable', reason: 'Browser region reference expired; take a fresh snapshot and ask again' };
  }
  if (!remembered || !remembered.refs.has(command.ref) || !validReference(command.ref)) throw new UserDomainRefusal('stale_reference');
  if (remembered.generation !== host.generation || command.generation !== undefined && command.generation !== host.generation) return gone('Browser restarted');
  if (command.document_id !== undefined && command.document_id !== remembered.document_id) return gone('Browser document changed');
  const tab = await host.displayTarget({ tab_id: command.tab_id, generation: remembered.generation });
  if (tab.document_id !== remembered.document_id) return gone('Browser document changed');
  const config = command.verification ? validateCardVerification(command.verification, new URL(tab.url).origin) : null;
  if (config && (!key || !command.ref.startsWith('backend:'))) throw new Error('Browser verification requires a card identity and a Tab-document region');
  if (anchor && JSON.stringify(anchor.verification?.config ?? null) !== JSON.stringify(config)) throw new UserDomainRefusal('stale_reference');
  const browser = host.browser, connection = await browser.ensureConnection();
  const sessionId = await browser.ensureTargetSession(connection, tab.target_id);
  const current = async () => (await connection.send('Page.getFrameTree', {}, sessionId)).frameTree?.frame;
  const frame = await current();
  if (frame?.loaderId !== remembered.document_id) return gone('Browser document changed');
  if (key) {
    if (!anchor) {
      if (host.cardAnchors.size >= maxAnchors) throw new Error('Browser card limit reached; close an existing browser card and try again');
      // Pin only this card's binding. Snapshot cache eviction must not retire
      // a live element. Failed registration expires without a watcher renewal.
      anchor = { scope, tab_id: command.tab_id, ref: command.ref, binding: { generation: remembered.generation, document_id: remembered.document_id, refs: new Set([command.ref]), children: new Set(remembered.children.has(command.ref) ? [command.ref] : []) } };
      if (config) anchor.verification = await BrowserCardVerification.create(connection, sessionId, frame, config);
      host.cardAnchors.set(key, anchor);
    }
    anchor.expires = Date.now() + anchorLifetimeMs;
  }
  if (anchor?.verification?.receipt?.result === 'accepted') return completion();
  const world = await browser.ensureFocusWorld(connection, sessionId, tab.target_id, frame);
  let objectId;
  try {
    let box;
    if (anchor?.verification && !anchor.elementResolver) {
      const resolved=await connection.send('DOM.resolveNode',{backendNodeId:Number(command.ref.slice(8)),executionContextId:world.contextId},sessionId);
      objectId=resolved.object?.objectId;
      if(!objectId)return gone('Browser element disappeared');
      const tracked = await connection.send('Runtime.callFunctionOn', {objectId,functionDeclaration:String(trackCardElement),returnByValue:false},sessionId);
      if (tracked.exceptionDetails || !tracked.result?.objectId) throw new Error('Browser card element tracking could not start');
      anchor.elementResolver = tracked.result.objectId;
      const resolver = anchor.elementResolver;
      anchor.releaseElement = () => { void connection.send('Runtime.releaseObject',{objectId:resolver},sessionId).catch(()=>{}); };
      await anchor.verification.bindInput(objectId,world.contextId,resolver);
    }
    if (anchor?.elementResolver) {
      if (objectId) await connection.send('Runtime.releaseObject',{objectId},sessionId);
      objectId = undefined;
      const resolved = await connection.send('Runtime.callFunctionOn', {objectId:anchor.elementResolver,functionDeclaration:'function(){return this()}',returnByValue:false},sessionId);
      if (resolved.exceptionDetails) return gone('Browser element disappeared');
      objectId = resolved.result?.objectId;
      if (!objectId) return gone('Browser element disappeared');
    }
    if (remembered.children.has(command.ref)) {
      box = await childFrameBox(browser, connection, sessionId, tab, command);
    } else {
      const resolved = objectId ? {object:{objectId}} : await connection.send('DOM.resolveNode', { backendNodeId: Number(command.ref.slice(8)), executionContextId: world.contextId }, sessionId);
      objectId = resolved.object?.objectId;
      if (!objectId) return gone('Browser element disappeared');
      const reply = await connection.send('Runtime.callFunctionOn', { objectId, functionDeclaration: String(watchBox), arguments: [{ value: command.after ?? null }, ...(anchor?.elementResolver ? [{ objectId: anchor.elementResolver }] : [])], returnByValue: true, awaitPromise: true }, sessionId);
      if (signal?.aborted) throw new Error('Browser card observation cancelled');
      if (reply.exceptionDetails) throw new Error('Browser card geometry could not be read');
      box = reply.result?.value;
    }
    if ((await current())?.loaderId !== remembered.document_id) return gone('Browser document changed');
    if (box === null) return gone('Browser element disappeared');
    if (!box || ![box.rect?.x, box.rect?.y, box.rect?.width, box.rect?.height, box.viewport_width, box.viewport_height].every(v => Number.isSafeInteger(v) && v >= 0)) throw new Error('Invalid browser card geometry');
    // Only the site origin is projected: URLs can contain userinfo and secret query parameters.
    let origin;
    try { origin = new URL(tab.url).origin; } catch { throw new Error('Invalid browser card site address'); }
    const region = { tab_id: tab.tab_id, ref: command.ref, generation: host.generation, document_id: tab.document_id, origin, ...box,
      ...(config ? {verification:config} : {}), ...(anchor?.verification?.receipt ? {verification_result:anchor.verification.receipt} : {}) };
    if (anchor) anchor.region = region;
    return {state:'present',region};
  } catch (error) {
    if (/No node with given id|Could not find node with given id|Could not find object with given id/.test(error?.message ?? '')) return gone('Browser element disappeared');
    if (/Execution context was destroyed|Cannot find context with specified id/.test(error?.message ?? '')) return gone('Browser document changed');
    if (error instanceof BrowserSnapshotError && ['stale_document_reference', 'stale_element_reference'].includes(error.code)) return gone('Browser document changed');
    throw error;
  } finally {
    if (objectId) await connection.send('Runtime.releaseObject', { objectId }, sessionId).catch(() => {});
  }
}
