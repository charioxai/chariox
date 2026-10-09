const DEFAULT_MAX_NODES = 5_000;
const DEFAULT_MAX_STRING_LENGTH = 2_048;
const DEFAULT_MAX_ATTRIBUTES = 32;
const DEFAULT_MAX_FRAMES = 64;

export class BrowserSnapshotError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserSnapshotError";
    this.code = code;
  }
}

export async function captureBrowserSnapshot({
  connection,
  sessionId,
  targetId,
  documentId,
  browserGeneration,
  snapshotRevision,
  limits = {},
  protectedValues = [],
}) {
  // MP-08/MP-10/MP-11: scrub the raw CDP strings before compaction can
  // truncate an echoed value and make exact-value redaction impossible.
  const rawConnection = connection;
  connection = {
    send: async (...args) => redactObservation(await rawConnection.send(...args), protectedValues),
  };
  const options = snapshotLimits(limits);
  const frames = snapshotFrames(
    await assertCurrentDocument(connection, sessionId, targetId, documentId),
    options.maxFrames,
  );
  const [accessibility, dom] = await Promise.all([
    captureFrameAccessibility(connection, sessionId, frames, options),
    connection.send(
      "DOMSnapshot.captureSnapshot",
      {
        computedStyles: [],
        includeDOMRects: true,
        includePaintOrder: false,
      },
      sessionId,
    ),
  ]);
  const currentFrames = snapshotFrames(
    await assertCurrentDocument(connection, sessionId, targetId, documentId),
    options.maxFrames,
  );
  if (JSON.stringify(frames) !== JSON.stringify(currentFrames)) {
    throw new BrowserSnapshotError(
      "stale_document_reference",
      "browser frame tree changed during snapshot capture",
    );
  }
  const compactedDom = compactDomSnapshot(dom, options);
  return {
    browser_generation: browserGeneration,
    target_id: targetId,
    document_id: documentId,
    snapshot_revision: snapshotRevision,
    accessibility_nodes: accessibility.nodes,
    accessibility_truncated: accessibility.truncated,
    ...compactedDom,
  };
}

// MD-N2 / MP-08/MP-10/MP-11: one variant policy for raw span protection
// and ordinary observation redaction, including kernel-replayed retired values.
export function observationProtectedVariants(protectedValues) {
  const variants = new Set();
  for (const secret of protectedValues) {
    if (typeof secret !== "string" || !secret) continue;
    for (const variant of [secret, secret.toLowerCase(), secret.toUpperCase(),
      encodeURIComponent(secret), encodeURIComponent(secret).toLowerCase(),
      JSON.stringify(secret).slice(1, -1),
      secret.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;").replaceAll("'", "&#39;"),
      Buffer.from(secret).toString("base64"), Buffer.from(secret).toString("base64url"),
      Buffer.from(secret).toString("hex"), Buffer.from(secret).toString("hex").toUpperCase()]) {
      variants.add(variant);
    }
  }
  return [...variants].sort((a, b) => b.length - a.length);
}

export function redactObservation(value, protectedValues) {
  const ordered = observationProtectedVariants(protectedValues);
  const scrub = (input) => {
    if (typeof input === "string") {
      for (const variant of ordered) input = input.replaceAll(variant, "[redacted]");
      return input;
    }
    if (Array.isArray(input)) return input.map(scrub);
    if (input && typeof input === "object") return Object.fromEntries(
      Object.entries(input).map(([key, child]) => [scrub(key), scrub(child)]),
    );
    return input;
  };
  return scrub(value);
}

// MP-08/MP-11: layout entries of one DOMSnapshot document whose rendered text
// echoes a protected value. Rendered text, incl. CSS-generated content
// (::before, ::after, ::marker, ::first-letter, counters) that has no DOM
// value, is flattened over the whole document in visual order (snapshots list
// pseudo-elements before children), so a value split across nested or sibling
// elements still matches. A second flattening reads displaced and clipped
// subtrees as separate runs. Visibility and adjacency use Chromium rendered
// text boxes and final text opacity, never computed font-size/opacity or
// reconstructed CSS clips. Tiny/unknown boxes are conservatively covered.
// A match protects every contributing entry and its nearest common laid-out
// ancestor;
// unreadable text or a match without such a container fails closed.
//
// Visual order can differ from DOM order (coordinator rule 2026-10-09). While
// values are registered, a container whose order can differ is masked, per
// container, never the page, unless its trusted layout boxes prove DOM reading
// order: flex/grid/-webkit-box containers with reversed, column-wrapped,
// column-flow or dense placement, CSS order or explicit grid placement; tables
// whose captions or header/footer groups move; block containers whose negative
// margins add up to half a rendered line height. Glyph order inside one run
// cannot be observed, so the block container of a bidi override, a direction change,
// bidi controls or right-to-left letters is masked. Displaced glyphs (absolute,
// fixed, sticky, offset relative, float, transform) near other glyphs mask both
// block containers. Gaps of a rendered line height or more are separate
// words or columns, as in any layout. The snapshot must carry
// RENDER_ORDER_STYLES (and textBoxes for per-line boxes).
export const RENDER_ORDER_STYLES = ["display", "position", "float", "order", "flex-direction", "flex-wrap", "grid-auto-flow",
  "grid-row-start", "grid-row-end", "grid-column-start", "grid-column-end", "-webkit-box-direction", "-webkit-box-ordinal-group",
  "caption-side", "direction", "unicode-bidi", "top", "right", "bottom", "left",
  "margin-top", "margin-right", "margin-bottom", "margin-left", "transform", "translate", "rotate", "scale", "offset-path",
  "overflow-x", "overflow-y", "clip-path"];
const PSEUDO_ORDER = { "::marker": 0, "::first-letter": 1, "::before": 2, "::after": 5 };
const BIDI_TEXT = /[\u0590-\u08ff\u200e\u200f\u202a-\u202e\u2066-\u2069\ufb1d-\ufdff\ufe70-\ufefc\u{10800}-\u{10fff}\u{1e800}-\u{1efff}]/u;
const GRID_PLACEMENT = ["grid-row-start", "grid-row-end", "grid-column-start", "grid-column-end"];
const TABLE_RANK = { "table-header-group": 1, "table-row-group": 2, "table-row": 2, "table-footer-group": 3 };
const MAX_GRID_ENTRIES = 16_384, MAX_GEOMETRY_COMPARISONS = 65_536, GRID_MARGIN = 128;
export function renderedTextEchoes(strings, document, protectedValues, overflow = { regions: [], unmeasured: new Set() }, viewport) {
  const echoes = new Set();
  if (!protectedValues.length) return echoes;
  if (!Array.isArray(viewport) || viewport.length !== 4 || !viewport.every(Number.isFinite) || viewport[2] <= 0 || viewport[3] <= 0) throw new Error("MP-11: unknown protection viewport");
  const nodes = document.nodes ?? {}, layout = document.layout ?? {};
  const count = nodes.nodeName?.length ?? 0, parent = [], depth = [], children = [], roots = [], entries = new Map(), css = new Map();
  for (let i = 0; i < count; i++) {
    const up = nodes.parentIndex?.[i] ?? -1;
    if (!Number.isInteger(up) || up >= i) throw new Error("MP-11: unordered snapshot");
    parent.push(up < 0 ? -1 : up); depth.push(up < 0 ? 0 : depth[up] + 1); children.push([]);
    (up < 0 ? roots : children[up]).push(i);
  }
  for (let k = 0; k < (layout.nodeIndex?.length ?? 0); k++) {
    const node = layout.nodeIndex[k];
    if (!Number.isInteger(node) || node < 0 || node >= count) throw new Error("MP-11: unknown layout node");
    (entries.get(node) ?? entries.set(node, []).get(node)).push(k);
  }
  const validBox = b => Array.isArray(b) && b.length === 4 && b.every(Number.isFinite) && b[2] >= 0 && b[3] >= 0;
  const lines = new Map(); // Chromium post-transform line boxes, in document coordinates.
  (document.textBoxes?.layoutIndex ?? []).forEach((k, j) => (lines.get(k) ?? lines.set(k, []).get(k)).push(document.textBoxes.bounds?.[j]));
  const textBoxes = k => lines.get(k) ?? [layout.bounds?.[k]];
  const uncertainText = k => textBoxes(k).some(b => !validBox(b) || b[2] < 2 || b[3] < 2);
  // The snapshot's final opacity accounts for overlapping/ancestor opacity.
  // It cannot prove arbitrary clipping; absent evidence always stays covered.
  const visibleText = k => uncertainText(k) || layout.textColorOpacities?.[k] !== 0;
  const name = (i) => strings[nodes.nodeName[i]] ?? "";
  const px = (value) => parseFloat(value) || 0;
  const displaced = (s) => /absolute|fixed|sticky/.test(s.position) || s.float !== "none" ||
    (s.position === "relative" && ["top", "right", "bottom", "left"].some((p) => px(s[p]) !== 0)) ||
    ["transform", "translate", "rotate", "scale", "offset-path"].some((p) => s[p] !== "none");
  const rows = new Map(); // Most entries share a style row: hash -> [[row, style]].
  for (const [i, ks] of entries) {
    if (name(i) === "#document") continue;
    const row = layout.styles?.[ks[0]];
    if (!Array.isArray(row) || row.length !== RENDER_ORDER_STYLES.length) throw new Error("MP-11: unknown render-order styles");
    let hash = 0, style;
    for (let j = 0; j < row.length; j++) hash = (hash * 31 + row[j]) | 0;
    const bucket = rows.get(hash) ?? rows.set(hash, []).get(hash);
    for (let b = 0; !style && b < bucket.length; b++) {
      let j = 0;
      while (j < row.length && bucket[b][0][j] === row[j]) j++;
      if (j === row.length) style = bucket[b][1];
    }
    if (!style) {
      if (!row.every((s) => typeof strings[s] === "string")) throw new Error("MP-11: unknown render-order styles");
      style = Object.fromEntries(RENDER_ORDER_STYLES.map((property, j) => [property, strings[row[j]]]));
      const out = /absolute|fixed/.test(style.position), moved = displaced(style);
      // separate: read as a run of its own; styles never exclude text.
      const d = style.display;
      Object.assign(style, { out, moved, separate: moved || style["overflow-x"] !== "visible" || style["overflow-y"] !== "visible" || style["clip-path"] !== "none",
        flex: /flex/.test(d), grid: /grid/.test(d), box: /box/.test(d), table: /^(inline-)?table$/.test(d), inline: /^(inline|contents|ruby|ruby-text)$/.test(d) });
      bucket.push([row, style]);
    }
    css.set(i, style);
  }
  const order = (i) => PSEUDO_ORDER[name(i)] ?? (name(i).startsWith("::") ? 3 : 4);
  children.forEach((list) => list.sort((a, b) => order(a) - order(b)));
  // Pass 1: one run in DOM visual order. Pass 2: separated subtrees are runs of their own, invisible text is skipped.
  const flatten = (split) => {
    let flat = "";
    const spans = [], runs = [roots]; // spans: [start, end, layout entry, node], in flat order.
    while (runs.length) {
      const run = runs.shift();
      for (const stack = [...run].reverse(); stack.length;) {
        const i = stack.pop(), s = css.get(i);
        if (split && s && i !== run[0] && s.separate) { runs.push([i]); continue; }
        for (const k of entries.get(i) ?? []) {
          const piece = layout.text?.[k];
          if (piece === -1) continue;
          if (typeof strings[piece] !== "string") throw new Error("MP-11: unreadable layout text");
          if (split && !visibleText(k)) continue;
          if (strings[piece]) spans.push([flat.length, flat.length + strings[piece].length, k, i]);
          flat += strings[piece];
        }
        for (let c = children[i].length - 1; c >= 0; c--) stack.push(children[i][c]);
      }
      flat += "\0";
    }
    return [flat, spans];
  };
  const common = (a, b) => { while (a !== b && a >= 0 && b >= 0) depth[a] >= depth[b] ? (a = parent[a]) : (b = parent[b]); return a === b ? a : -1; };
  for (const [flat, spans] of [flatten(false), flatten(true)]) {
    for (const variant of observationProtectedVariants(protectedValues)) {
      for (let at = flat.indexOf(variant); at >= 0; at = flat.indexOf(variant, at + 1)) {
        let low = 0, high = spans.length;
        while (low < high) { const mid = (low + high) >> 1; spans[mid][1] <= at ? (low = mid + 1) : (high = mid); }
        let container;
        for (let s = low; s < spans.length && spans[s][0] < at + variant.length; s++) {
          echoes.add(spans[s][2]);
          container = container === undefined ? spans[s][3] : common(container, spans[s][3]);
        }
        while (container >= 0 && !entries.has(container)) container = parent[container];
        if (!(container >= 0)) throw new Error("MP-11: rendered text without a laid-out container");
        entries.get(container).forEach((k) => echoes.add(k));
      }
    }
  }
  const blockOf = (i) => {
    while (i >= 0 && (!css.has(i) || name(i) === "#text" || css.get(i).inline)) i = parent[i];
    if (i < 0) throw new Error("MP-11: uncertain order without a container");
    return i;
  };
  // MP-08/MP-11 review af34d30f0: a container's own box does not contain
  // overflow. Cover its descendants' layout AND per-line text boxes too.
  // Missing geometry falls back to the nearest local clipping ancestor.
  const masked = new Set(), extra = new Set(), geometryUnknown = new Set();
  const mask = (i) => {
    if (masked.has(i)) return;
    masked.add(i);
    const own = (entries.get(i) ?? []).map(k => layout.bounds?.[k]).filter(validBox), boxes = [], missing = [];
    for (const stack = [i]; stack.length;) {
      const node = stack.pop();
      for (const k of entries.get(node) ?? []) {
        for (const box of [layout.bounds?.[k], ...(lines.get(k) ?? [])]) validBox(box) ? boxes.push(box) : missing.push(k);
      }
      stack.push(...children[node]);
    }
    if (missing.length) {
      let clipped = i, fallback;
      while (clipped >= 0 && !fallback) {
        const s = css.get(clipped), name_ = name(clipped).toLowerCase();
        if (s && !['#document', 'html', 'body'].includes(name_) && s['overflow-x'] !== 'visible' && s['overflow-y'] !== 'visible') {
          const bounds = (entries.get(clipped) ?? []).map(k => layout.bounds?.[k]);
          if (bounds.length && bounds.every(box => validBox(box) && box[2] > 0 && box[3] > 0)) fallback = bounds;
        }
        clipped = parent[clipped];
      }
      if (!fallback) throw new Error('MP-11: unmeasurable container overflow without a local clip');
      boxes.push(...fallback);missing.forEach(k => overflow.unmeasured.add(k));
    }
    (entries.get(i) ?? []).forEach(k => echoes.add(k));
    for (const b of boxes) {
      if (b[2] <= 0 || b[3] <= 0 || own.some(a => a[2] > 0 && a[3] > 0 && a[0] <= b[0] && a[1] <= b[1] && a[0] + a[2] >= b[0] + b[2] && a[1] + a[3] >= b[1] + b[3])) continue;
      const key = b.join(',');if (!extra.has(key)) { extra.add(key);overflow.regions.push(b); }
    }
  };
  for (const [i, ks] of entries) if (ks.some(k => !validBox(layout.bounds?.[k]))) geometryUnknown.add(i);
  const corners = ([x, y, w, h] = []) => {
    if (![x, y, w, h].every(Number.isFinite)) throw new Error("MP-11: unknown text bounds");
    return [x, y, x + w, y + h];
  };
  const mover = new Array(count).fill(-1), textHeight = new Array(count).fill(0), uncertain = new Set();
  const glyphs = new Map(), extent = new Array(count).fill(null);
  const grow = (i, b) => { const e = extent[i]; extent[i] = e ? [Math.min(e[0], b[0]), Math.min(e[1], b[1]), Math.max(e[2], b[2]), Math.max(e[3], b[3])] : b; };
  for (let i = 0; i < count; i++) {
    const s = css.get(i), up = parent[i];
    mover[i] = up >= 0 ? mover[up] : -1;
    if (s?.moved && name(i) !== "#text") mover[i] = i;
    for (const k of entries.get(i) ?? []) {
      if (layout.text?.[k] === -1 || !/\S/.test(strings[layout.text?.[k]] ?? "") || !visibleText(k)) continue;
      if (uncertainText(k)) { uncertain.add(i); geometryUnknown.add(i); }
      const seen = [];
      for (const line of textBoxes(k)) {
        if (!validBox(line)) { geometryUnknown.add(i); continue; }
        seen.push(corners(line));
        textHeight[i] = Math.max(textHeight[i], line[3]);
      }
      if (seen.length) { glyphs.set(k, seen); seen.forEach(b => grow(i, b)); }
    }
  }
  // In-flow glyph extents; out-of-flow glyphs are left to the proximity check.
  const outOfFlow = (i) => name(i) !== "#text" && css.has(i) && (css.get(i).out || css.get(i).float !== "none");
  for (let i = count - 1; i > 0; i--) if (extent[i] && parent[i] >= 0 && !outOfFlow(i)) {
    grow(parent[i], extent[i]); textHeight[parent[i]] = Math.max(textHeight[parent[i]], textHeight[i]);
  }
  // Rendered flow of a container in DOM order: line boxes of inline content,
  // glyph extents of block-level children and items; out-of-flow boxes are
  // left to the proximity check below.
  const flow = (i, out = []) => {
    for (const c of children[i]) {
      const s = css.get(c);
      if (outOfFlow(c)) continue;
      if (!s || name(c) === "#text" || s.inline) {
        for (const k of entries.get(c) ?? []) out.push(...(glyphs.get(k) ?? []));
        flow(c, out);
      } else if (extent[c]) out.push(extent[c]);
    }
    return out;
  };
  // DOM reading order: the nearest box within a rendered line height to the
  // right is DOM-next; the nearest box within a line height below comes later; boxes overlapping on a line are uncertain. Farther
  // boxes are separate words or columns, as in any layout.
  let comparisons = 0;
  const ordered = (boxes, reach) => boxes.length <= 2000 && boxes.every((a, j) => {
    let right = -1, below = -1;
    for (let m = 0; m < boxes.length; m++) {
      if (comparisons >= MAX_GEOMETRY_COMPARISONS) return false;
      comparisons++;
      const b = boxes[m];
      if (m === j) continue;
      if (Math.min(a[3], b[3]) - Math.max(a[1], b[1]) > Math.min(a[3] - a[1], b[3] - b[1]) / 2) {
        if (b[0] < a[2] - 1 && b[2] > a[0] + 1) return false;
        if (b[0] >= a[2] - 1 && b[0] - a[2] < reach && (right < 0 || b[0] < boxes[right][0])) right = m;
      } else if (b[1] + b[3] > a[1] + a[3] && Math.min(a[2], b[2]) > Math.max(a[0], b[0]) && b[1] - a[3] < reach && (below < 0 || b[1] < boxes[below][1])) below = m;
    }
    return (right < 0 || right === j + 1) && (below < 0 || below > j);
  });
  const items = (i) => children[i].flatMap((c) => (!css.has(c) ? items(c) : name(c) === "#text" || css.get(c).out ? [] : [css.get(c)]));
  const rank = (s) => (s.display === "table-caption" ? (s["caption-side"] === "bottom" ? 4 : 0) : TABLE_RANK[s.display]);
  const suspects = new Set(), pulled = new Map();
  for (const [i, s] of css) {
    if (name(i) === "#text") {
      if (entries.get(i).some((k) => BIDI_TEXT.test(strings[layout.text?.[k]] ?? ""))) mask(blockOf(i));
      continue;
    }
    let up = parent[i];
    while (up >= 0 && !css.has(up)) up = parent[up];
    const outer = css.get(up), { flex, grid, box, table } = s;
    const kids = flex || grid || box || table ? items(i) : [];
    const ranks = table ? kids.map(rank).filter((r) => r !== undefined) : [];
    if ((flex && (/reverse/.test(s["flex-direction"] + s["flex-wrap"]) || (s["flex-direction"].startsWith("column") && s["flex-wrap"] !== "nowrap"))) ||
      (grid && /column|dense/.test(s["grid-auto-flow"])) || (box && s["-webkit-box-direction"] === "reverse") ||
      ((flex || grid || box) && kids.some((k) => k.order !== "0" || k["-webkit-box-ordinal-group"] !== "1" || (grid && GRID_PLACEMENT.some((p) => k[p] !== "auto")))) ||
      ranks.some((r, j) => j > 0 && r < ranks[j - 1])) suspects.add(i);
    // Glyph order inside one text run is not observable: bidi reordering masks its block container.
    if (/override|plaintext/.test(s["unicode-bidi"]) || s.direction !== (outer?.direction ?? "ltr")) mask(blockOf(i));
    const item = /^inline/.test(s.display) || Boolean(outer && (outer.flex || outer.grid || outer.box));
    const pull = s.out ? 0 : Math.max(0, -px(s["margin-top"])) + Math.max(0, -px(s["margin-bottom"])) +
      (item ? Math.max(0, -px(s["margin-left"])) + Math.max(0, -px(s["margin-right"])) : 0);
    if (pull) { const at = blockOf(up >= 0 ? up : i); pulled.set(at, (pulled.get(at) ?? 0) + pull); }
  }
  // Negative margins trigger a geometry proof at half a rendered line height.
  for (const [at, pull] of pulled) if (pull >= Math.max(2, textHeight[at]) / 2) suspects.add(at);
  const hasUnknownGeometry = i => [...geometryUnknown].some(node => { while (node >= 0 && node !== i) node = parent[node];return node === i; });
  for (const i of suspects) if (hasUnknownGeometry(i) || !ordered(flow(i), Math.max(2, textHeight[i]))) mask(i);
  // Displaced glyphs within a rendered line height of glyphs outside their displaced subtree.
  const pre = [], size = new Array(count).fill(1), cells = new Map(), CELL = 128;
  for (const stack = [...roots].reverse(); stack.length;) { const i = stack.pop(); pre[i] = pre.length; for (let c = children[i].length - 1; c >= 0; c--) stack.push(children[i][c]); }
  for (let i = count - 1; i >= 0; i--) if (parent[i] >= 0) size[parent[i]] += size[i];
  const near = []; // Grow boxes by half their rendered line height.
  // MP-08/MP-11: transformed glyphs may cover millions of CSS pixels. Only
  // index the capture viewport plus one cell of margin, in document pixels.
  // Count memberships (including overlapping boxes), not just unique cells.
  const view = [viewport[0] - GRID_MARGIN, viewport[1] - GRID_MARGIN,
    viewport[0] + viewport[2] + GRID_MARGIN, viewport[1] + viewport[3] + GRID_MARGIN];
  // When the shared proof budget is exhausted, cover each participating local
  // container. Root text uses its own laid-out box, never html/body/the page.
  const cover = i => {
    let block = i;
    while (block >= 0 && (!css.has(block) || name(block) === '#text' || css.get(block).inline)) block = parent[block];
    mask(block < 0 || ['#document', 'html', 'body'].includes(name(block).toLowerCase()) ? i : block);
  };
  // Tiny and unknown text remains visible to protection, never an order exemption.
  for (const i of uncertain) cover(i);
  const coverGlyphs = () => {
    let moved;
    for (const [k, boxes] of glyphs) {
      const i = layout.nodeIndex[k];
      if (mover[i] < 0) continue;
      cover(i);
      const reach = Math.max(2, textHeight[i]) / 2;
      for (const b of boxes) {
        const r = [Math.max(view[0], b[0] - reach), Math.max(view[1], b[1] - reach), Math.min(view[2], b[2] + reach), Math.min(view[3], b[3] + reach)];
        if (r[0] >= r[2] || r[1] >= r[3]) continue;
        moved = moved ? [Math.min(moved[0], r[0]), Math.min(moved[1], r[1]), Math.max(moved[2], r[2]), Math.max(moved[3], r[3])] : r;
      }
    }
    if (!moved) return;
    for (const [k, boxes] of glyphs) {
      const i = layout.nodeIndex[k], reach = Math.max(2, textHeight[i]) / 2;
      if (mover[i] < 0 && boxes.some(b => b[0] - reach <= moved[2] && b[2] + reach >= moved[0] && b[1] - reach <= moved[3] && b[3] + reach >= moved[1])) cover(i);
    }
  };
  let gridEntries = 0;
  for (const k of glyphs.keys()) {
    const i = layout.nodeIndex[k], reach = Math.max(2, textHeight[i]) / 2;
    for (const b of glyphs.get(k)) {
      const rect = [Math.max(b[0] - reach, view[0]), Math.max(b[1] - reach, view[1]),
        Math.min(b[2] + reach, view[2]), Math.min(b[3] + reach, view[3]), i];
      if (rect[0] >= rect[2] || rect[1] >= rect[3]) continue;
      const needed = (Math.floor(rect[2] / CELL) - Math.floor(rect[0] / CELL) + 1) *
        (Math.floor(rect[3] / CELL) - Math.floor(rect[1] / CELL) + 1);
      if (needed > MAX_GRID_ENTRIES - gridEntries) { coverGlyphs(); return echoes; }
      gridEntries += needed; // Checked before allocating rect/bucket entries.
      near.push(rect);
      for (let x = Math.floor(rect[0] / CELL); x <= Math.floor(rect[2] / CELL); x++) {
        for (let y = Math.floor(rect[1] / CELL); y <= Math.floor(rect[3] / CELL); y++) (cells.get(`${x},${y}`) ?? cells.set(`${x},${y}`, []).get(`${x},${y}`)).push(rect);
      }
    }
  }
  const within = (i, d) => pre[d] <= pre[i] && pre[i] < pre[d] + size[d];
  for (const a of near) {
    const d = mover[a[4]];
    if (d < 0) continue;
    for (let x = Math.floor(a[0] / CELL); x <= Math.floor(a[2] / CELL); x++) for (let y = Math.floor(a[1] / CELL); y <= Math.floor(a[3] / CELL); y++) {
      for (const b of cells.get(`${x},${y}`) ?? []) {
        if (comparisons >= MAX_GEOMETRY_COMPARISONS) { coverGlyphs(); return echoes; }
        comparisons++;
        if (!within(b[4], d) && a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]) { mask(blockOf(a[4])); mask(blockOf(b[4])); }
      }
    }
  }
  return echoes;
}

function snapshotLimits(rawLimits) {
  return {
    maxNodes: positiveBound(rawLimits.maxNodes, DEFAULT_MAX_NODES),
    maxStringLength: positiveBound(
      rawLimits.maxStringLength,
      DEFAULT_MAX_STRING_LENGTH,
    ),
    maxAttributes: positiveBound(
      rawLimits.maxAttributes,
      DEFAULT_MAX_ATTRIBUTES,
    ),
    maxFrames: positiveBound(rawLimits.maxFrames, DEFAULT_MAX_FRAMES),
  };
}

function positiveBound(value, fallback) {
  return Number.isSafeInteger(value) && value > 0
    ? Math.min(value, fallback)
    : fallback;
}

async function assertCurrentDocument(connection, sessionId, targetId, documentId) {
  const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
  const currentDocumentId = frameTree?.frameTree?.frame?.loaderId;
  if (currentDocumentId !== documentId) {
    throw new BrowserSnapshotError(
      "stale_document_reference",
      `browser target ${JSON.stringify(targetId)} moved from document ${JSON.stringify(documentId)} to ${JSON.stringify(currentDocumentId ?? null)}`,
    );
  }
  return frameTree?.frameTree;
}

function snapshotFrames(frameTree, maxFrames) {
  const pending = [frameTree];
  const frames = [];
  while (pending.length) {
    const tree = pending.shift();
    if (frames.length + pending.length >= maxFrames) {
      throw new BrowserSnapshotError("snapshot_frame_limit", "browser frame tree exceeded its frame bound");
    }
    frames.push({ id: tree?.frame?.id, loaderId: tree?.frame?.loaderId });
    pending.push(...(tree?.childFrames ?? []));
  }
  return frames;
}

// Reports whether it cut the tree at the node bound, since dropped and
// duplicate nodes can leave fewer nodes than the bound after a cut.
async function captureFrameAccessibility(connection, sessionId, frames, options) {
  const nodes = [];
  const seen = new Set();
  let truncated = false;
  for (const frame of frames) {
    if (nodes.length >= options.maxNodes) {
      truncated = true;
      break;
    }
    const tree = await connection.send(
      "Accessibility.getFullAXTree",
      frame.id ? { frameId: frame.id } : {},
      sessionId,
    );
    const remaining = options.maxNodes - nodes.length;
    if (Array.isArray(tree?.nodes) && tree.nodes.length > remaining) truncated = true;
    // AX node IDs belong to each frame's tree. Resolve their relationships
    // before joining frames through the shared backend DOM references.
    for (const node of compactAccessibilityNodes(tree?.nodes, { ...options, maxNodes: remaining })) {
      if (!seen.has(node.node_ref)) {
        seen.add(node.node_ref);
        nodes.push(node);
      }
    }
  }
  return { nodes, truncated };
}

function compactAccessibilityNodes(rawNodes, options) {
  const nodes = Array.isArray(rawNodes) ? rawNodes.slice(0, options.maxNodes) : [];
  const referenceByNodeId = new Map();
  for (const node of nodes) {
    const reference = backendNodeReference(node?.backendDOMNodeId);
    if (reference && typeof node?.nodeId === "string") {
      referenceByNodeId.set(node.nodeId, reference);
    }
  }
  return nodes.flatMap((node) => {
    const nodeRef = backendNodeReference(node?.backendDOMNodeId);
    if (!nodeRef) {
      return [];
    }
    const properties = new Map(
      (Array.isArray(node?.properties) ? node.properties : [])
        .filter((property) => typeof property?.name === "string")
        .map((property) => [property.name, property?.value?.value]),
    );
    const role = compactString(node?.role?.value, options.maxStringLength);
    const protectedValue =
      properties.get("protected") === true ||
      role.toLowerCase().includes("password");
    const rawValue = compactString(node?.value?.value, options.maxStringLength);
    return [{
      node_ref: nodeRef,
      parent_ref: referenceByNodeId.get(node?.parentId) ?? null,
      child_refs: (Array.isArray(node?.childIds) ? node.childIds : [])
        .map((childId) => referenceByNodeId.get(childId))
        .filter(Boolean),
      role,
      name: compactString(node?.name?.value, options.maxStringLength),
      description: compactString(
        node?.description?.value,
        options.maxStringLength,
      ),
      value: protectedValue && rawValue ? "[redacted]" : rawValue,
      ignored: node?.ignored === true,
      disabled: properties.get("disabled") === true,
      focused: properties.get("focused") === true,
      states: announcedStates(properties),
    }];
  });
}

// The control states a screen reader announces, named as the kernel expects.
function announcedStates(properties) {
  const tristate = (name, on, off) => {
    const value = properties.get(name);
    if (value === undefined) return [];
    if (value === "mixed") return ["mixed"];
    return [value === true || value === "true" ? on : off];
  };
  const invalid = properties.get("invalid");
  return [
    ...tristate("checked", "checked", "not checked"),
    ...tristate("pressed", "pressed", "not pressed"),
    ...(properties.has("expanded") ? [properties.get("expanded") === true ? "expanded" : "collapsed"] : []),
    ...(properties.get("selected") === true ? ["selected"] : []),
    ...(properties.get("required") === true ? ["required"] : []),
    ...(invalid !== undefined && invalid !== false && invalid !== "false" ? ["invalid"] : []),
  ];
}

function compactDomSnapshot(rawSnapshot, options) {
  const strings = Array.isArray(rawSnapshot?.strings) ? rawSnapshot.strings : [];
  const documents = Array.isArray(rawSnapshot?.documents)
    ? rawSnapshot.documents
    : [];
  const compacted = [];
  const ownerByDocumentIndex = new Map();
  const shadowRoots = [];
  const documentCount = Math.min(documents.length, options.maxNodes);
  for (let documentIndex = 0; documentIndex < documentCount; documentIndex += 1) {
    const document = documents[documentIndex];
    if (compacted.length >= options.maxNodes) {
      break;
    }
    const nodes = document?.nodes ?? {};
    const backendNodeIds = Array.isArray(nodes.backendNodeId)
      ? nodes.backendNodeId
      : [];
    const layoutBounds = boundsByNodeIndex(document?.layout);
    const contentDocuments = rareDataByIndex(nodes.contentDocumentIndex);
    const shadowRootTypes = rareDataByIndex(nodes.shadowRootType);
    for (
      let nodeIndex = 0;
      nodeIndex < backendNodeIds.length && compacted.length < options.maxNodes;
      nodeIndex += 1
    ) {
      const nodeRef = backendNodeReference(backendNodeIds[nodeIndex]);
      if (!nodeRef) {
        continue;
      }
      const parentIndex = arrayValue(nodes.parentIndex, nodeIndex);
      const parentRef = Number.isSafeInteger(parentIndex)
        ? backendNodeReference(backendNodeIds[parentIndex])
        : null;
      const nodeType = arrayValue(nodes.nodeType, nodeIndex);
      const nodeName = snapshotString(
        strings,
        arrayValue(nodes.nodeName, nodeIndex),
        options.maxStringLength,
      );
      const text =
        nodeType === 3 || nodeType === 4
          ? snapshotString(
              strings,
              arrayValue(nodes.nodeValue, nodeIndex),
              options.maxStringLength,
            )
          : "";
      const contentDocumentIndex = contentDocuments.get(nodeIndex);
      if (
        Number.isSafeInteger(contentDocumentIndex) &&
        contentDocumentIndex >= 0 &&
        contentDocumentIndex < documentCount
      ) {
        ownerByDocumentIndex.set(contentDocumentIndex, nodeRef);
      }
      const shadowRootTypeIndex = shadowRootTypes.get(nodeIndex);
      const shadowRootType = snapshotString(
        strings,
        shadowRootTypeIndex,
        options.maxStringLength,
      );
      if (shadowRootType) {
        shadowRoots.push({
          node_ref: nodeRef,
          shadow_root_type: shadowRootType,
        });
      }
      compacted.push({
        node_ref: nodeRef,
        parent_ref: parentRef,
        document_index: documentIndex,
        node_type: Number.isSafeInteger(nodeType) ? nodeType : 0,
        node_name: nodeName,
        text,
        attributes: compactAttributes(
          strings,
          arrayValue(nodes.attributes, nodeIndex),
          nodeName,
          options,
        ),
        bounds: layoutBounds.get(nodeIndex) ?? null,
      });
    }
  }
  return {
    dom_documents: documents.slice(0, documentCount).map((document, documentIndex) => ({
      document_index: documentIndex,
      url: snapshotString(
        strings,
        document?.documentURL,
        options.maxStringLength,
      ),
      owner_node_ref: ownerByDocumentIndex.get(documentIndex) ?? null,
    })),
    shadow_roots: shadowRoots,
    dom_nodes: compacted,
  };
}

function rareDataByIndex(rawData) {
  const indexes = Array.isArray(rawData?.index) ? rawData.index : [];
  const values = Array.isArray(rawData?.value) ? rawData.value : [];
  const result = new Map();
  for (let offset = 0; offset < indexes.length && offset < values.length; offset += 1) {
    const nodeIndex = indexes[offset];
    if (Number.isSafeInteger(nodeIndex)) {
      result.set(nodeIndex, values[offset]);
    }
  }
  return result;
}

function boundsByNodeIndex(layout) {
  const result = new Map();
  const nodeIndices = Array.isArray(layout?.nodeIndex) ? layout.nodeIndex : [];
  const bounds = Array.isArray(layout?.bounds) ? layout.bounds : [];
  for (let index = 0; index < nodeIndices.length; index += 1) {
    const nodeIndex = nodeIndices[index];
    const rawBounds = bounds[index];
    if (
      !Number.isSafeInteger(nodeIndex) ||
      !Array.isArray(rawBounds) ||
      rawBounds.length < 4 ||
      !rawBounds.slice(0, 4).every(Number.isFinite)
    ) {
      continue;
    }
    result.set(nodeIndex, {
      x: rawBounds[0],
      y: rawBounds[1],
      width: rawBounds[2],
      height: rawBounds[3],
    });
  }
  return result;
}

function compactAttributes(strings, rawAttributes, nodeName, options) {
  const attributes = {};
  const indexes = Array.isArray(rawAttributes) ? rawAttributes : [];
  for (
    let index = 0;
    index + 1 < indexes.length &&
    Object.keys(attributes).length < options.maxAttributes;
    index += 2
  ) {
    const name = snapshotString(
      strings,
      indexes[index],
      options.maxStringLength,
    );
    if (!name) {
      continue;
    }
    const value = snapshotString(
      strings,
      indexes[index + 1],
      options.maxStringLength,
    );
    attributes[name] = shouldRedactAttribute(nodeName, name)
      ? "[redacted]"
      : value;
  }
  return attributes;
}

function shouldRedactAttribute(nodeName, attributeName) {
  const normalizedNodeName = nodeName.toLowerCase();
  const normalizedAttributeName = attributeName.toLowerCase();
  if (
    normalizedAttributeName.includes("password") ||
    normalizedAttributeName.includes("secret") ||
    normalizedAttributeName.includes("token") ||
    normalizedAttributeName === "authorization" ||
    normalizedAttributeName === "cookie"
  ) {
    return true;
  }
  return (
    normalizedAttributeName === "value" &&
    ["input", "textarea", "option"].includes(normalizedNodeName)
  );
}

function backendNodeReference(backendNodeId) {
  return Number.isSafeInteger(backendNodeId) && backendNodeId > 0
    ? `backend:${backendNodeId}`
    : null;
}

function compactString(value, maxLength) {
  if (typeof value !== "string") {
    return "";
  }
  let result = "";
  let byteLength = 0;
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    const characterBytes = codePoint <= 0x7f
      ? 1
      : codePoint <= 0x7ff
        ? 2
        : codePoint <= 0xffff
          ? 3
          : 4;
    if (byteLength + characterBytes > maxLength) {
      break;
    }
    result += character;
    byteLength += characterBytes;
  }
  return result;
}

function snapshotString(strings, index, maxLength) {
  return Number.isSafeInteger(index)
    ? compactString(strings[index], maxLength)
    : "";
}

function arrayValue(value, index) {
  return Array.isArray(value) ? value[index] : undefined;
}
