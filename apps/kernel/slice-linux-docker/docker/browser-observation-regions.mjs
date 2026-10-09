// MP-08/MP-10/MP-11: trusted CDP layout, re-located before/after every frame.
import { BrowserCdpClient } from './browser-controller-cdp.mjs';
import { withBrowserFrames } from './browser-controller-frames.mjs';
import { RENDER_ORDER_STYLES, redactObservation, renderedTextEchoes } from './browser-controller-snapshot.mjs';
import { fileURLToPath } from 'node:url';

function quadRegion(quad) {
  if (!Array.isArray(quad) || quad.length !== 8 || !quad.every(Number.isFinite)) throw new Error('unknown region');
  const xs = quad.filter((_, i) => i % 2 === 0), ys = quad.filter((_, i) => i % 2 === 1);
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys)];
}

const desktop = (origin, [x, y, width, height]) => [origin[0] + x, origin[1] + y, width, height];
// Where a frame owner's document renders: its content origin, or its whole
// border box when a transform cannot map local layout. Hidden owners: null.
async function ownerPlacement(connection, sessionId, backendNodeId, [x, y]) {
  if (!Number.isSafeInteger(backendNodeId)) throw new Error('unknown frame owner');
  let model;
  try { ({ model } = await connection.send('DOM.getBoxModel', { backendNodeId }, sessionId)); }
  catch (error) { if (/Could not compute box model/.test(error?.message ?? '')) return null; throw error; }
  const [bx, by, bw, bh] = quadRegion(model.border), [cx, cy] = quadRegion(model.content);
  const [x1, y1, x2, y2, , , x4, y4] = model.border, near = (a, b) => Math.abs(a - b) < 0.5;
  const plain = near(bw, model.width) && near(bh, model.height) && near(y1, y2) && near(x1, x4) && x1 < x2 && y1 < y4;
  return plain ? { origin: [x + cx, y + cy] } : { whole: [x + bx, y + by, bw, bh] };
}

export async function locateBrowserRegions(targets, browser, values = [], { contentTarget = null, contentScale = 1 } = {}) {
  const regions = [];
  if (typeof browser.ensureConnection === 'function') {
    const connection = await browser.ensureConnection();
    const { targetInfos = [] } = await connection.send('Target.getTargets');
    const pages = targetInfos.filter(target => target.type === 'page' && (!contentTarget || target.targetId === contentTarget));
    // Closed tabs have no pixels. Scan all current pages for delayed copies,
    // including tabs opened by page handlers after the original insertion.
    targets = targets.filter(target => pages.some(page => page.targetId === target.target_id));
    for (const page of pages) {
      if (!targets.some(target => target.target_id === page.targetId)) targets.push({ target_id: page.targetId, echo_only: true });
    }
  }
  for (let target of targets) {
    const { connection, sessionId } = await browser.resolvePageTarget(target.target_id);
    const top = await connection.send('Page.getFrameTree', {}, sessionId);
    // Backend nodes belong to one document. A departed document has no pixels;
    // retain value/opaque-media scanning in its replacement.
    if (target.document_id && target.document_id !== top.frameTree.frame.loaderId) {
      target = { target_id: target.target_id, echo_only: true };
    }
    const { executionContextId } = await connection.send('Page.createIsolatedWorld', { frameId: top.frameTree.frame.id, worldName: 'chariox-observation-mask' }, sessionId);
    const { result: visibility } = await connection.send('Runtime.evaluate', { contextId: executionContextId, expression: '[document.visibilityState, window.devicePixelRatio, window.innerHeight]', returnByValue: true }, sessionId);
    if (!contentTarget && visibility.value?.[0] === 'hidden') continue; // Confirmed absent from desktop pixels.
    if ((!contentTarget && visibility.value?.[0] !== 'visible') || (visibility.value?.[1] !== (contentTarget ? contentScale : 1))) throw new Error('unbound desktop frame');
    await withBrowserFrames(connection, sessionId, target.target_id, target.document_id ?? top.frameTree.frame.loaderId, async frames => {
      let entry = target.node_ref?.startsWith('frame:')
        ? frames.find(frame => frame.prefix && target.node_ref.startsWith(frame.prefix)) : frames[0];
      // An isolated child document can navigate without changing the top loader.
      if (!entry) { target = { target_id: target.target_id, echo_only: true }; entry = frames[0]; }
      const nodeRef = target.echo_only ? 'backend:1' : target.node_ref.slice(entry.prefix.length);
      if (!/^backend:[1-9][0-9]*$/.test(nodeRef)) throw new Error('unknown region');
      // CDP screenshots bind to the emulated content viewport, which can be
      // larger than the native/headless window. Desktop masks still need it.
      const bounds = contentTarget ? null : (await connection.send('Browser.getWindowForTarget', { targetId: target.target_id })).bounds;
      const { cssLayoutViewport: viewport, cssVisualViewport: visual } = await connection.send('Page.getLayoutMetrics', {}, sessionId);
      if (visual?.scale !== 1 || !viewport?.clientHeight || (!contentTarget && (bounds.windowState === 'minimized' || viewport.clientHeight > bounds.height))) throw new Error('unbound frame');
      const contentHeight = visibility.value?.[2];
      if (!Number.isFinite(contentHeight) || contentHeight <= 0 || (!contentTarget && contentHeight > bounds.height) || contentHeight < viewport.clientHeight) throw new Error('unbound content origin');
      // innerHeight includes the horizontal scrollbar, unlike CDP clientHeight.
      // Browser chrome and the native border sit above the content viewport.
      // Image masking adds padding for the border and outward rounding.
      const origin = contentTarget ? [0, 0] : [bounds.left, bounds.top + bounds.height - contentHeight];
      let offset = [0, 0];
      for (let frame = entry; frame.parent; frame = frame.parent) {
        const owner = await connection.send('DOM.getFrameOwner', { frameId: frame.frame.id }, frame.parent.sessionId);
        const { model } = await connection.send('DOM.getBoxModel', { backendNodeId: owner.backendNodeId }, frame.parent.sessionId);
        const [x, y] = quadRegion(model.content);
        offset = [offset[0] + x, offset[1] + y];
      }
      let snapshot;
      if (!target.echo_only) {
        try {
          const { model } = await connection.send('DOM.getBoxModel', { backendNodeId: Number(nodeRef.slice(8)) }, entry.sessionId);
          const [x, y, width, height] = quadRegion(model.border);
          regions.push([origin[0] + offset[0] + x, origin[1] + offset[1] + y, width, height]);
        } catch (error) {
          if (entry !== frames[0] || !values.length) throw error;
          // A renderer can replace a field without changing its document. Re-locate
          // value-bearing inputs using raw, trusted layout; never retry insertion.
          snapshot = await connection.send('DOMSnapshot.captureSnapshot', { computedStyles: RENDER_ORDER_STYLES, includeDOMRects: true }, sessionId);
          const document = snapshot.documents?.[0], strings = snapshot.strings ?? [];
          const nodes = document?.nodes ?? {}, layout = document?.layout ?? {};
          const input = nodes.inputValue ?? {};
          for (let i = 0; i < (input.index?.length ?? 0); i++) {
            const value = strings[input.value[i]];
            if (typeof value !== 'string' || redactObservation(value, values) === value) continue;
            const region = layout.bounds?.[layout.nodeIndex?.indexOf(input.index[i])];
            if (!Array.isArray(region) || region.length !== 4 || !region.every(Number.isFinite) || region[2] <= 0 || region[3] <= 0) throw new Error('unknown field region');
            regions.push([origin[0] + region[0] - viewport.pageX, origin[1] + region[1] - viewport.pageY, region[2], region[3]]);
          }
        }
      }
      // Raw page strings and rendered layout text are checked before any
      // truncation. MP-11 (owner 2026-10-08): every frame is scanned in its own
      // session and only the echoing text, attributes and input values are masked;
      // media is not masked whole. A frame that cannot be mapped or inspected is (fail closed).
      if (values.length) {
        const owned = new Set(), placed = new Map();
        const place = frame => {
          if (!frame.parent) return { origin: [0, 0] };
          if (!placed.has(frame)) placed.set(frame, (async () => {
            const parent = await place(frame.parent);
            const owner = await connection.send('DOM.getFrameOwner', { frameId: frame.frame.id }, frame.parent.sessionId);
            owned.add(frame.parent.sessionId + ' ' + owner.backendNodeId);
            return parent?.origin ? ownerPlacement(connection, frame.parent.sessionId, owner.backendNodeId, parent.origin) : parent;
          })());
          return placed.get(frame);
        };
        const placements = [];
        for (const frame of frames) placements.push(await place(frame));
        for (const [n, frame] of frames.entries()) {
          const placement = placements[n];
          if (placement?.whole) regions.push(desktop(origin, placement.whole));
          if (!placement?.origin) continue;
          const frameSnapshot = frame.sessionId === sessionId && snapshot ? snapshot
            : await connection.send('DOMSnapshot.captureSnapshot', { computedStyles: RENDER_ORDER_STYLES, includeDOMRects: true }, frame.sessionId);
          if (frame.sessionId === sessionId) snapshot = frameSnapshot;
          const strings = frameSnapshot.strings ?? [], documents = frameSnapshot.documents ?? [];
          const echoed = index => typeof strings[index] === 'string' && redactObservation(strings[index], values) !== strings[index];
          // In-process child documents report layout in their own coordinates.
          const owners = new Map();
          for (const document of documents) {
            const content = document.nodes?.contentDocumentIndex ?? {};
            for (let i = 0; i < (content.index?.length ?? 0); i++) owners.set(content.value[i], document.nodes.backendNodeId?.[content.index[i]]);
          }
          for (const [d, document] of documents.entries()) {
            const at = d === 0 ? placement : await ownerPlacement(connection, frame.sessionId, owners.get(d), placement.origin);
            if (at?.whole) regions.push(desktop(origin, at.whole));
            if (!at?.origin) continue;
            const nodes = document.nodes ?? {}, layout = document.layout ?? {};
            const inputValues = new Map((nodes.inputValue?.index ?? []).map((index, i) => [index, nodes.inputValue.value[i]]));
            const frameOwners = new Set((nodes.contentDocumentIndex?.index ?? []));
            const overflow = { regions: [], unmeasured: new Set() };
            const rendered = renderedTextEchoes(strings, document, values, overflow);
            regions.push(...overflow.regions.map(([x,y,w,h]) => desktop(origin,
              [at.origin[0]+x-(document.scrollOffsetX??0),at.origin[1]+y-(document.scrollOffsetY??0),w,h])));
            for (let i = 0; i < (layout.nodeIndex?.length ?? 0); i++) {
              const index = layout.nodeIndex[i];
              if (overflow.unmeasured.has(i)) continue; // Covered by its clipping ancestor.
              const name = strings[nodes.nodeName?.[index]]?.toLowerCase();
              const attributes = nodes.attributes?.[index] ?? [];
              const uninspected = ['iframe', 'frame'].includes(name) && !frameOwners.has(index) && !owned.has(frame.sessionId + ' ' + nodes.backendNodeId?.[index]);
              if (uninspected || rendered.has(i) || echoed(nodes.nodeValue?.[index]) || echoed(inputValues.get(index)) || attributes.some(echoed)) {
                const region = layout.bounds?.[i];
                if (!Array.isArray(region) || region.length !== 4 || !region.every(Number.isFinite)) throw new Error('unknown echo region');
                if (region[2] > 0 && region[3] > 0) regions.push(desktop(origin, [at.origin[0] + region[0] - (document.scrollOffsetX ?? 0), at.origin[1] + region[1] - (document.scrollOffsetY ?? 0), region[2], region[3]]));
              }
            }
          }
        }
      }
      // Browser titles, URL bars and link-status overlays can echo a Vault value.
      // They are outside DOM layout, so mask their value-free native regions too.
      if (!contentTarget) {
        regions.push([bounds.left, bounds.top, bounds.width, Math.max(1, bounds.height - contentHeight)]);
        regions.push([bounds.left, bounds.top + bounds.height - 24, bounds.width, 24]);
      }
      for (const frame of frames) {
        const current = await connection.send('Page.getFrameTree', {}, frame.sessionId);
        if (current.frameTree?.frame?.loaderId !== frame.frame.loaderId) throw new Error('stale frame');
      }
    });
  }
  return regions;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const browser = new BrowserCdpClient({ requestTimeoutMs: 1500 });
  try {
    let input = '';
    for await (const chunk of process.stdin) input += chunk;
    const policy = JSON.parse(input);
    process.stdout.write(JSON.stringify(await locateBrowserRegions(policy.targets, browser, policy.values)));
  } catch {
    process.stderr.write('observation redacted, retrying\n');
    process.exitCode = 75;
  } finally { await browser.close(); }
}
