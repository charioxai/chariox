// MP-08/MP-10/MP-11: trusted CDP protection regions for every frame of every
// visible page, in device pixels relative to that page's content viewport.
// Consumers place them on their own pixels: CDP captures at the viewport
// origin, desktop capture at the AT-SPI document origin proven by
// browser-desktop-protection.py. Page JavaScript never participates.
import { createHash } from 'node:crypto';
import { RENDER_ORDER_STYLES, redactObservation, renderedTextEchoes } from './browser-controller-snapshot.mjs';

const MAX_FRAMES = 64, MAX_REGIONS = 4096, MAX_PAGES = 32, FRAME_TIMEOUT_MS = 500;
const MARKERS = ['data-chariox-secret', 'data-chariox-observation-protected', 'data-observation-protected'];
const OPAQUE_MEDIA = new Set(['canvas', 'svg', 'img', 'video']);
const FRAME_OWNERS = new Set(['iframe', 'frame']);
const PLUGINS = new Set(['object', 'embed']);

function quadRect(quad) {
  if (!Array.isArray(quad) || quad.length !== 8 || !quad.every(Number.isFinite)) throw new Error('MP-11: unknown frame geometry');
  const xs = quad.filter((_, i) => i % 2 === 0), ys = quad.filter((_, i) => i % 2 === 1);
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys)];
}
const scaled = ([x, y, w, h], s) => [x * s, y * s, w * s, h * s];
function intersect(a, b) {
  const x = Math.max(a[0], b[0]), y = Math.max(a[1], b[1]);
  const r = Math.min(a[0] + a[2], b[0] + b[2]), bottom = Math.min(a[1] + a[3], b[1] + b[3]);
  return r > x && bottom > y ? [x, y, r - x, bottom - y] : null;
}
// Outward integer rounding: a protected subpixel edge is never left uncovered.
const outward = ([x, y, w, h]) => {
  const left = Math.floor(x), top = Math.floor(y);
  return [left, top, Math.ceil(x + w) - left, Math.ceil(y + h) - top];
};

// A frame owner maps child coordinates by translation only when its border
// quad (TL, TR, BR, BL) is its layout box moved: directed edges of the layout
// size (no mirror or rotation), no skew. Layout sizes are whole CSS pixels.
export function translatedQuad(q, width, height) {
  if (!Array.isArray(q) || q.length !== 8 || !q.every(Number.isFinite)) return false;
  const near = (a, b, tolerance = 0.01) => Math.abs(a - b) < tolerance;
  return near(q[2] - q[0], width, 0.5) && near(q[7] - q[1], height, 0.5) &&
    near(q[3], q[1]) && near(q[4], q[2]) && near(q[5], q[7]) && near(q[6], q[0]);
}

// Device-pixel boxes. A child that cannot be placed by translation (plain
// false) is withheld: its whole owner box is protected.
async function nodeBox(connection, sessionId, backendNodeId, dpr) {
  const { model } = await connection.send('DOM.getBoxModel', { backendNodeId }, sessionId);
  const border = quadRect(model?.border), content = quadRect(model?.content);
  return { border: scaled(border, dpr), content: scaled(content, dpr), plain: translatedQuad(model.border, model.width, model.height) };
}

// Pure: protected layout rectangles of one DOMSnapshot document, in device
// pixels of that document's viewport. Protection is inherited by descendants
// (display:contents, overflow, shadow content); markers, password/OTP/payment
// fields, policy target nodes and Vault value echoes (DOM strings and rendered
// layout text, which alone holds CSS-generated content) are protected. While
// Vault values are registered, opaque media are protected too: a page can draw
// a value into them and delete every DOM echo; so are containers whose visual
// order may differ from DOM order (renderedTextEchoes). Frame and plugin owners are
// returned for mapping or withholding (owner decision 2026-10-08: inspectable
// frames, and media without Vault values, are not masked whole).
export function documentProtection(snapshot, index, { values = [], targetNodes = new Set() } = {}) {
  const document = snapshot.documents[index], strings = snapshot.strings ?? [];
  const nodes = document.nodes ?? {}, layout = document.layout ?? {};
  const count = nodes.nodeName?.length ?? 0;
  const text = i => (typeof strings[i] === 'string' ? strings[i] : '');
  const echoed = i => values.length > 0 && typeof strings[i] === 'string' && redactObservation(strings[i], values) !== strings[i];
  const sparse = field => new Map((field?.index ?? []).map((node, i) => [node, field.value[i]]));
  const inputValue = sparse(nodes.inputValue), textValue = sparse(nodes.textValue), contentDocument = sparse(nodes.contentDocumentIndex);
  const parent = nodes.parentIndex ?? [];
  const marked = new Uint8Array(count);
  for (let i = 0; i < count; i++) {
    const name = text(nodes.nodeName[i]).toLowerCase();
    const attributes = nodes.attributes?.[i] ?? [];
    let own = targetNodes.has(nodes.backendNodeId?.[i]) || echoed(nodes.nodeValue?.[i]) || echoed(inputValue.get(i)) || echoed(textValue.get(i));
    for (let a = 0; !own && a + 1 < attributes.length; a += 2) {
      const key = text(attributes[a]).toLowerCase(), value = text(attributes[a + 1]);
      own = MARKERS.includes(key) || (key === 'autocomplete' && /password|one-time-code|cc-/i.test(value)) ||
        (name === 'input' && key === 'type' && value.toLowerCase() === 'password') || echoed(attributes[a + 1]);
    }
    if (values.length && OPAQUE_MEDIA.has(name)) own = true;
    const up = parent[i] ?? -1;
    if (!Number.isInteger(up) || up >= i) throw new Error('MP-11: unordered snapshot'); // Pre-order: parents first.
    marked[i] = own || (up >= 0 && marked[up]) ? 1 : 0;
  }
  const regions = [], owners = [], rendered = renderedTextEchoes(strings, document, values);
  const scroll = [document.scrollOffsetX ?? 0, document.scrollOffsetY ?? 0];
  if (!scroll.every(Number.isFinite)) throw new Error('MP-11: unknown document scroll');
  for (let k = 0; k < (layout.nodeIndex?.length ?? 0); k++) {
    const i = layout.nodeIndex[k], bounds = layout.bounds?.[k];
    if (!Array.isArray(bounds) || bounds.length !== 4 || !bounds.every(Number.isFinite)) throw new Error('MP-11: unknown layout region');
    const rect = [bounds[0] - scroll[0], bounds[1] - scroll[1], bounds[2], bounds[3]];
    if (marked[i] || rendered.has(k)) { if (rect[2] > 0 && rect[3] > 0) regions.push(rect); continue; }
    const name = text(nodes.nodeName[i]).toLowerCase();
    if (FRAME_OWNERS.has(name) || PLUGINS.has(name)) {
      owners.push({ backendNodeId: nodes.backendNodeId[i], rect, contentDocument: contentDocument.get(i), ...(PLUGINS.has(name) ? { plugin: true } : {}) });
    }
  }
  return { regions, owners };
}

async function frameTree(connection, sessionId) {
  return (await connection.send('Page.getFrameTree', {}, sessionId)).frameTree;
}
const treeIdentity = tree => JSON.stringify([tree?.frame?.id, tree?.frame?.loaderId, (tree?.childFrames ?? []).map(treeIdentity)]);

// Isolated (out-of-process) frames of one page, attached for this measurement.
// A frame that cannot be attached stays uninspected: its owner is protected.
async function isolatedFrames(connection, top, frameIds) {
  const { targetInfos = [] } = await connection.send('Target.getTargets');
  const candidates = targetInfos.filter(target => target.type === 'iframe');
  if (candidates.length > MAX_FRAMES) return { owned: [], detach: async () => {} };
  const frames = [];
  for (const target of candidates) {
    try {
      const { sessionId } = await connection.send('Target.attachToTarget', { targetId: target.targetId, flatten: true });
      frames.push({ sessionId, attached: true });
      const tree = await frameTree(connection, sessionId);
      Object.assign(frames.at(-1), { tree, frame: tree.frame });
    } catch {}
  }
  const owned = [];
  for (let changed = true; changed;) {
    changed = false;
    for (const entry of frames) {
      if (!entry.frame || owned.includes(entry) || !frameIds.has(entry.frame.parentId)) continue;
      owned.push(entry); changed = true;
      const visit = tree => { frameIds.add(tree.frame.id); (tree.childFrames ?? []).forEach(visit); };
      visit(entry.tree);
    }
  }
  return { owned, detach: () => Promise.all(frames.map(({ sessionId }) => connection.send('Target.detachFromTarget', { sessionId }).catch(() => {}))) };
}

async function sessionRegions(connection, entry, dpr, origin, clip, policy, targetNodes, regions, children, withheld) {
  const snapshot = await connection.send('DOMSnapshot.captureSnapshot', { computedStyles: policy.values.length ? RENDER_ORDER_STYLES : [] }, entry.sessionId);
  const pending = [{ index: 0, origin, clip }];
  const ownersBySession = new Map(children.map(child => [child.ownerBackendNodeId, child]));
  while (pending.length) {
    const doc = pending.shift();
    if ((snapshot.documents?.length ?? 0) <= doc.index) throw new Error('MP-11: missing frame document');
    const { regions: own, owners } = documentProtection(snapshot, doc.index, { values: policy.values, targetNodes });
    const place = rect => { const placed = intersect([rect[0] + doc.origin[0], rect[1] + doc.origin[1], rect[2], rect[3]], doc.clip); if (placed) regions.push(placed); return placed; };
    own.forEach(place);
    for (const owner of owners) {
      const isolated = ownersBySession.get(owner.backendNodeId);
      let box = null;
      try { box = await nodeBox(connection, entry.sessionId, owner.backendNodeId, dpr); } catch {}
      // Plugin/PDF content has no inspectable document.
      const reason = owner.plugin ? 'plugin' : !box ? 'frame_failed' : !box.plain ? 'transformed_frame' : owner.contentDocument === undefined && !isolated ? 'uninspected_frame' : null;
      if (reason) { if (place(owner.rect)) withheld.push(reason); continue; }
      const childOrigin = [origin[0] + box.content[0], origin[1] + box.content[1]];
      const childClip = intersect([childOrigin[0], childOrigin[1], box.content[2], box.content[3]], doc.clip);
      if (!childClip) continue; // Scrolled out of its parent: no pixels.
      if (owner.contentDocument !== undefined) pending.push({ index: owner.contentDocument, origin: childOrigin, clip: childClip });
      else Object.assign(isolated, { origin: childOrigin, clip: childClip, ownerRect: intersect([owner.rect[0] + doc.origin[0], owner.rect[1] + doc.origin[1], owner.rect[2], owner.rect[3]], doc.clip) });
    }
  }
}

// CDP's errors for a node without a layout box (hidden, detached).
const NOT_RENDERED = /Could not compute box model|No node found for given backend id/;
const SECRET_FIELDS = 'input[type=password i],[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected],[autocomplete*=password i],[autocomplete*=one-time-code i],[autocomplete*=cc- i]';
async function search(connection, sessionId, query) {
  const { searchId, resultCount } = await connection.send('DOM.performSearch', { query }, sessionId);
  try {
    if (!Number.isInteger(resultCount) || resultCount > MAX_REGIONS) throw new Error('MP-11: protection search bound');
    return resultCount ? (await connection.send('DOM.getSearchResults', { searchId, fromIndex: 0, toIndex: resultCount }, sessionId)).nodeIds : [];
  } finally { await connection.send('DOM.discardSearchResults', { searchId }, sessionId).catch(() => {}); }
}

// Without Vault values or targets, one selector search (it pierces shadow
// roots, closed ones included, and in-process frame documents) finds the
// secret fields and frame owners, instead of a whole-page DOMSnapshot. A
// marker protects its descendants, and a failed box lookup does not prove
// absent layout, which this path cannot bound: false lets the caller measure
// the session from a DOMSnapshot instead.
async function searchedRegions(connection, entry, dpr, regions, children, withheld) {
  const { sessionId, origin, clip } = entry;
  await connection.send('DOM.getDocument', { depth: 0 }, sessionId);
  const describe = async nodeId => (await connection.send('DOM.describeNode', { nodeId }, sessionId)).node;
  const secrets = await Promise.all((await search(connection, sessionId, SECRET_FIELDS)).map(describe));
  if (secrets.some(node => node.localName !== 'input')) return false;
  // In-process frame documents were searched; other owners need their box.
  const owners = (await Promise.all((await search(connection, sessionId, 'iframe,frame,object,embed')).map(describe)))
    .filter(node => !(FRAME_OWNERS.has(node.localName) && node.contentDocument));
  let secretBoxes, ownerBoxes;
  try {
    [secretBoxes, ownerBoxes] = await Promise.all([secrets, owners].map(nodes => Promise.all(nodes.map(node => nodeBox(connection, sessionId, node.backendNodeId, dpr)))));
  } catch { return false; }
  const place = rect => { const placed = intersect([rect[0] + origin[0], rect[1] + origin[1], rect[2], rect[3]], clip); if (placed) regions.push(placed); return placed; };
  const isolated = new Map(children.map(child => [child.ownerBackendNodeId, child]));
  secretBoxes.forEach(found => place(found.border));
  for (const [i, node] of owners.entries()) {
    const found = ownerBoxes[i], child = isolated.get(node.backendNodeId);
    if (!child || !found.plain) { // Plugin, uninspected or transformed frame.
      if (place(found.border)) withheld.push(PLUGINS.has(node.localName) ? 'plugin' : child ? 'transformed_frame' : 'uninspected_frame');
      continue;
    }
    const childOrigin = [origin[0] + found.content[0], origin[1] + found.content[1]];
    const childClip = intersect([childOrigin[0], childOrigin[1], found.content[2], found.content[3]], clip);
    if (childClip) Object.assign(child, { origin: childOrigin, clip: childClip, ownerRect: intersect([origin[0] + found.border[0], origin[1] + found.border[1], found.border[2], found.border[3]], clip) });
  }
  return true;
}

function policyTargets(policy, targetId, prefix, documentId) {
  const ids = new Set();
  for (const target of policy.targets ?? []) {
    if (target.target_id !== targetId || typeof target.node_ref !== 'string') continue;
    const ref = prefix ? (target.node_ref.startsWith(prefix) ? target.node_ref.slice(prefix.length) : null)
      : (target.node_ref.startsWith('frame:') || target.document_id !== documentId ? null : target.node_ref);
    if (/^backend:[1-9][0-9]*$/.test(ref ?? '')) ids.add(Number(ref.slice(8)));
  }
  return ids;
}

// One visible page: device-pixel regions relative to its content viewport.
// Returns null for a hidden page (no pixels) unless the caller renders hidden
// pages itself (CDP screenshots: `hidden`). Throws when the top document
// cannot be bound; an uninspectable child frame protects its owner box.
export async function measurePageProtection(connection, sessionId, targetId, policy, { hidden = false } = {}) {
  const top = await frameTree(connection, sessionId);
  const { executionContextId } = await connection.send('Page.createIsolatedWorld', { frameId: top.frame.id, worldName: 'chariox-protection-regions' }, sessionId);
  const { result } = await connection.send('Runtime.evaluate', { contextId: executionContextId, returnByValue: true,
    expression: '[document.visibilityState, devicePixelRatio, innerWidth, innerHeight]' }, sessionId);
  const [visibility, dpr, innerWidth, innerHeight] = result?.value ?? [];
  if (visibility === 'hidden' && !hidden) return null;
  if (!['visible', ...(hidden ? ['hidden'] : [])].includes(visibility) || !(dpr > 0) || !(innerWidth > 0) || !(innerHeight > 0)) throw new Error('MP-11: unbound page');
  // DevTools, settings/password pages and extensions render page or browser
  // data outside page markers: a visible one is never partially revealed.
  if (top.frame.url !== 'about:blank' && !/^(https?:|file:|chrome-error:)/.test(top.frame.url)) throw new Error('MP-11: browser-internal page');
  const { cssVisualViewport: visual } = await connection.send('Page.getLayoutMetrics', {}, sessionId);
  if (visual?.scale !== 1 || !(visual?.zoom > 0)) throw new Error('MP-11: pinch-zoomed page'); // Visual offsets are not mapped.
  const viewport = [0, 0, Math.round(innerWidth * dpr), Math.round(innerHeight * dpr)];
  const regions = [], withheld = [];
  const frameIds = new Set(); const visit = tree => { frameIds.add(tree.frame.id); (tree.childFrames ?? []).forEach(visit); }; visit(top);
  const { owned = [], detach = async () => {} } = await isolatedFrames(connection, top, frameIds);
  try {
    const root = { sessionId, frame: top.frame, tree: top, origin: [0, 0], clip: viewport };
    const entries = [root, ...owned];
    const contains = (tree, id) => (tree.childFrames ?? []).some(child => child.frame.id === id || contains(child, id));
    for (const entry of owned) {
      entry.parent = entries.find(candidate => candidate.frame.id === entry.frame.parentId || contains(candidate.tree, entry.frame.parentId));
      try { entry.ownerBackendNodeId = (await connection.send('DOM.getFrameOwner', { frameId: entry.frame.id }, entry.parent.sessionId)).backendNodeId; } catch {}
    }
    for (const entry of entries) {
      if (entry !== root && !entry.origin) continue; // Owner protected, uninspected or scrolled away.
      const children = owned.filter(child => child.parent === entry && Number.isInteger(child.ownerBackendNodeId));
      const prefix = entry === root ? '' : `frame:${entry.frame.id}:${entry.frame.loaderId}:`;
      try {
        const targetNodes = policyTargets(policy, targetId, prefix, top.frame.loaderId);
        const searched = !policy.values.length && !targetNodes.size && await searchedRegions(connection, entry, dpr, regions, children, withheld);
        if (!searched) await sessionRegions(connection, entry, dpr, entry.origin, entry.clip, policy, targetNodes, regions, children, withheld);
      } catch (error) {
        if (entry === root || !entry.ownerRect) throw error;
        regions.push(entry.ownerRect); withheld.push('frame_failed'); // MP-11: fail closed for this frame only.
        for (const child of owned) if (child.parent === entry) child.origin = null;
      }
    }
    for (const entry of entries) {
      if (entry !== root && !entry.origin) continue;
      if (treeIdentity(await frameTree(connection, entry.sessionId)) !== treeIdentity(entry.tree)) throw new Error('MP-11: frame changed during protection');
    }
  } finally { await detach(); }
  // withheld: why whole frames were masked (fixed labels, never page data).
  // zoom: page zoom (CSS to DIP); devicePixelRatio is screen scale x zoom unless emulated.
  const page = { url: top.frame.url, document_id: top.frame.loaderId, dpr, zoom: visual.zoom, viewport: viewport.slice(2) };
  if (regions.length > MAX_REGIONS) return { ...page, regions: [viewport], withheld: ['region_bound'] };
  return { ...page, regions: regions.map(outward), withheld };
}

// A private session per measurement: its DOM agent state (document node ids,
// searches, isolated worlds) never invalidates the controller's page session.
async function withSession(connection, targetId, run) {
  const { sessionId } = await connection.send('Target.attachToTarget', { targetId, flatten: true });
  try { return await run(sessionId); } finally { await connection.send('Target.detachFromTarget', { sessionId }).catch(() => {}); }
}

// Every visible page target, with its window (screen DIP) for desktop placement.
export async function measureBrowserProtection(browser, policy) {
  if (policy?.unknown || !Array.isArray(policy?.values) || !Array.isArray(policy?.targets)) throw new Error('MP-11: browser protection policy unknown');
  const connection = await browser.ensureConnection();
  const { targetInfos = [] } = await connection.send('Target.getTargets');
  const pages = targetInfos.filter(target => target.type === 'page');
  if (pages.length > MAX_PAGES) throw new Error('MP-11: browser page bound exceeded');
  const result = [];
  for (const page of pages) {
    const { windowId, bounds } = await connection.send('Browser.getWindowForTarget', { targetId: page.targetId });
    if (bounds?.windowState === 'minimized') continue;
    const window = [bounds?.left, bounds?.top, bounds?.width, bounds?.height];
    if (!window.every(Number.isFinite)) throw new Error('MP-11: unknown browser window');
    // A visible page that cannot be bound fails the whole measurement closed.
    const measured = await withSession(connection, page.targetId, sessionId => measurePageProtection(connection, sessionId, page.targetId, policy));
    if (measured === null) continue;
    // Screen DIP: the X11 client window must be this exact rectangle at one scale.
    result.push({ target_id: page.targetId, window_id: windowId, window, chrome: Boolean(policy.values.length || policy.targets.length), ...measured });
  }
  return { pages: result };
}

export const protectionDigest = measurement => createHash('sha256').update(JSON.stringify(measurement)).digest('hex');

// The compositor may still show the layout preceding a measurement: after
// two animation frames, pixels derive from that layout or later. One frame
// suffices before re-measuring: it applies pending compositor scroll offsets.
export async function awaitPresented(browser, pages, frames = 2) {
  const nested = 'requestAnimationFrame(() => resolve(true))';
  const callback = frames === 2 ? `requestAnimationFrame(() => ${nested})` : nested;
  const connection = await browser.ensureConnection();
  await Promise.all(pages.map(page => withSession(connection, page.target_id, async sessionId => {
    const top = await frameTree(connection, sessionId);
    const { executionContextId } = await connection.send('Page.createIsolatedWorld', { frameId: top.frame.id, worldName: 'chariox-protection-regions' }, sessionId);
    const { result } = await connection.send('Runtime.evaluate', { contextId: executionContextId, awaitPromise: true, returnByValue: true,
      expression: `new Promise(resolve => { setTimeout(() => resolve(false), ${FRAME_TIMEOUT_MS}); ${callback}; })` }, sessionId);
    if (result?.value !== true) throw new Error('MP-11: page did not present a frame');
  })));
}

// A measurement whose layout has reached the screen, or null (fail closed).
// Measuring again after a presented frame also observes compositor scrolls.
export async function measurePresented(browser, policy) {
  try {
    const measurement = await measureBrowserProtection(browser, policy);
    await awaitPresented(browser, measurement.pages);
    return measurement;
  } catch { return null; }
}

// Measure, let the measured layout reach the screen, capture, re-measure. A
// changed page, window, frame tree or region set retries with a fresh
// measurement; after `attempts` it captures fail-closed: capture(null) must
// protect every browser pixel it cannot bind.
export async function fenceBrowserCapture(browser, policy, capture, attempts = 3) {
  for (let attempt = 0; attempt < attempts; attempt++) {
    const before = await measurePresented(browser, policy);
    if (!before) break;
    const result = await capture(before);
    // One frame first: the re-measurement then sees compositor scrolls too.
    let after = null;
    try { await awaitPresented(browser, before.pages, 1); after = await measureBrowserProtection(browser, policy); } catch {}
    if (after && protectionDigest(after) === protectionDigest(before)) return result;
  }
  return capture(null);
}

// Streams: a frame captured with protection serial S is released only when a
// measurement that began after its capture still equals S. New protection is
// adopted only after two equal measurements separated by a presented frame
// (measure() awaits one first); meanwhile serial 0 withholds every browser pixel.
export class ProtectionGate {
  constructor(measure) { this.measure = measure; this.serial = 0; this.current = null; this.candidate = null; }
  get protection() { return this.current?.measurement ?? null; }
  get protectionSerial() { return this.current?.serial ?? 0; }
  async step(now = Date.now) {
    const started = now();
    const measurement = await this.measure().catch(() => null);
    const digest = measurement ? protectionDigest(measurement) : null;
    const verified = digest && digest === this.current?.digest ? this.current.serial : null;
    let changed = false;
    if (digest !== (this.current?.digest ?? null)) {
      const stable = digest !== null && digest === this.candidate;
      changed = stable || this.current !== null;
      this.current = stable ? { serial: ++this.serial, digest, measurement } : null;
      this.candidate = digest;
    }
    return { started, verified, changed };
  }
}
