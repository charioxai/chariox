// MP-08/MP-10/MP-11: DOM mirror v2 observer (local protocol 482). Installed only
// in the controller's isolated world. One full snapshot per document, then
// MutationObserver deltas (rrweb's model): the page's own stylesheets and
// attributes travel instead of a computed-style dump. Page code never sees
// this object; raw mutation records, URLs and unsanitized values stay here.
export const mirror2ObserverExpression = () => `(${installMirror2.toString()})(${sanitizeMirrorCss.toString()})`;

// Pure CSS sanitizer, shared verbatim with the kernel (cross-origin sheets read
// through CDP). Every url() becomes a kernel resource key or `none`; nothing
// executable or network-addressable survives. Input must be CSSOM-serialized.
export function sanitizeMirrorCss(text, base, resource, variants = []) {
  // Known namespace URIs are identifiers, never fetched; any other @namespace drops.
  const namespaces = new Set(['http://www.w3.org/1999/xhtml', 'http://www.w3.org/2000/svg', 'http://www.w3.org/1998/Math/MathML', 'http://www.w3.org/1999/xlink']);
  let out = text.replace(/@namespace\s+([a-zA-Z_][\w-]*\s+)?(?:url\(\s*)?["']?([^"')\s;]*)["']?\s*\)?\s*;/gi, (_, prefix = '', uri) => namespaces.has(uri) ? `@namespace ${prefix}"${uri}";` : '');
  // Escaped function names (custom properties keep raw tokens: `u\\72l(`) are
  // decoded first, so the url() rewrite below sees every fetching function.
  out = out.replace(/((?:[a-zA-Z_-]|\\[0-9a-fA-F]{1,6}\s?|\\[^\n0-9a-fA-F])+)\(/g, (match, name) => {
    if (!name.includes('\\')) return match;
    const plain = name.replace(/\\(?:([0-9a-fA-F]{1,6})\s?|([\s\S]))/g, (_, hex, char) => hex ? String.fromCodePoint(Math.min(parseInt(hex, 16), 0x10ffff) || 0xfffd) : char);
    return /^[a-zA-Z_-]+$/.test(plain) ? `${plain}(` : 'x-invalid(';
  });
  // Custom properties and var()-bearing declarations keep their source tokens,
  // so every url() spelling occurs; image-set() strings are URLs too.
  out = out.replace(/((?:-webkit-)?image-set\()([^()]*(?:\([^()]*\)[^()]*)*)\)/gi, (_, open, body) => `${open}${body.replace(/(^|[,\s(])("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')/g, (m, lead, string) => `${lead}url(${string})`)})`);
  const fontFaces = [...out.matchAll(/@font-face\s*\{[^}]*\}/g)].map(match => [match.index, match.index + match[0].length]);
  const unescape = value => value.replace(/\\(?:([0-9a-fA-F]{1,6})\s?|([\s\S]))/g, (_, hex, char) => hex ? String.fromCodePoint(Math.min(parseInt(hex, 16), 0x10ffff) || 0xfffd) : char);
  out = out.replace(/url\(\s*(?:"((?:[^"\\]|\\[\s\S])*)"|'((?:[^'\\]|\\[\s\S])*)'|((?:[^)\s"'\\]|\\[0-9a-fA-F]{1,6}\s?|\\[^\n])*))\s*\)/gi, (_, double, single, bare, at) => {
    const value = unescape(double ?? single ?? bare ?? '');
    if (!value) return 'none';
    if (value.startsWith('#')) return `url("#${value.slice(1).replace(/[^\w-]/g, '')}")`;
    const key = resource(value, base, fontFaces.some(([start, end]) => at >= start && at < end) ? 'font' : 'image');
    return key ? `url("mr:${key}")` : 'none';
  });
  // Invalidates (never rewrites) legacy executable/binding constructs.
  out = out.replace(/expression\s*\(/gi, 'x-expr(').replace(/(^|[;{\s])behavior\s*:/gi, '$1x-behavior:').replace(/-moz-binding/gi, 'x-binding').replace(/javascript\s*:/gi, 'x-js:');
  out = out.replace(/@import[^;]*;/gi, '');
  // Residual spellings the rewrites cannot parse (an unterminated url( inside a
  // string, a bare @import) are neutralized; the client refuses any that remain.
  out = out.replace(/url(\s*)\((?!"(?:mr:r[0-9]{1,9}|#[\w-]*)"\))/gi, 'urlx$1(').replace(/@import/gi, '@x-import');
  for (const variant of variants) if (variant && out.includes(variant)) out = out.replaceAll(variant, '*'.repeat(Math.min(variant.length, 64)));
  return out;
}

// The sanitizer is passed in: the isolated world has no module scope.
export function installMirror2(sanitizeMirrorCss) {
  if (globalThis.__charioxMirror2) return true;
  const HTML = 'http://www.w3.org/1999/xhtml', SVG = 'http://www.w3.org/2000/svg', MATH = 'http://www.w3.org/1998/Math/MathML';
  const NODE_BUDGET = 200000, BYTE_BUDGET = 64 * 1024 * 1024, RESOURCE_BUDGET = 4096, DEPTH = 512;
  const DROP = new Set('script noscript template meta base title param track'.split(' '));
  const OPAQUE = new Set('canvas video audio object embed applet frame frameset portal fencedframe'.split(' '));
  const SVG_TAGS = new Set('svg g path circle ellipse rect line polyline polygon text tspan textPath use defs symbol clipPath mask marker pattern linearGradient radialGradient stop title desc a image switch view filter feBlend feColorMatrix feComponentTransfer feComposite feConvolveMatrix feDiffuseLighting feDisplacementMap feDistantLight feDropShadow feFlood feFuncA feFuncB feFuncG feFuncR feGaussianBlur feMerge feMergeNode feMorphology feOffset fePointLight feSpecularLighting feSpotLight feTile feTurbulence style'.split(' '));
  const MATH_TAGS = new Set('math mi mo mn ms mtext mrow mfrac msqrt mroot msub msup msubsup munder mover munderover mtable mtr mtd mspace mstyle mpadded mphantom menclose semantics annotation merror'.split(' '));
  const DENY_ATTR = new Set('src srcset href xlink:href action formaction srcdoc ping data codebase nonce integrity poster background lowsrc dynsrc manifest autofocus target download http-equiv is sizes imagesrcset imagesizes'.split(' '));
  // HTML attributes that the UA renders or exposes (presentational, form,
  // accessibility, tooltips); any other attribute ships only if a selector or
  // attr() of the page's own CSS references it (CSS is the only reader).
  const RENDERED = new Set('class id style title lang dir hidden tabindex role alt for placeholder width height colspan rowspan span type value checked selected disabled readonly multiple open start reversed size rows cols wrap label popover inert contenteditable slot part exportparts href align valign bgcolor border cellpadding cellspacing color face nowrap hspace vspace clear noshade frame rules text link vlink alink compact abbr scope summary datetime cite min max low high optimum media headers'.split(' '));
  const INPUT_TYPES = new Set('text search email url number tel checkbox radio range button submit reset date time color hidden'.split(' '));
  const MARKERS = '[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected],input[type=password]';
  let serial = 0, variants = [], targets = new WeakSet(), records = [], overflow = false, revision = 0;
  const ids = new WeakMap(), nodes = new Map(), kids = new Map(), parentOf = new Map(), kindOf = new Map();
  const styleNodes = new Map(), roots = new Map(), pendingHosts = new Map(), foreign = new Set();
  // Hosts of closed shadow roots (found by the kernel's trusted DOMSnapshot pass):
  // this world cannot read their content, so they are opaque regions.
  const closedHosts = new WeakSet();
  const urlKeys = new Map();
  let newResources = [], pendingSheets = [], marked = new WeakSet(), lastSheetCheck = 0, cssAttrs = null;
  const referenced = text => { const out = []; for (const m of String(text).matchAll(/\[\s*(?:[\w-]*\|)?([a-zA-Z_:][-a-zA-Z0-9_:.]*)|attr\(\s*([a-zA-Z_:][-a-zA-Z0-9_:.]*)/g)) out.push((m[1] ?? m[2]).toLowerCase()); return out; };
  const keepAttr = (html, lower) => !html || !cssAttrs || RENDERED.has(lower) || lower.startsWith('aria-') || cssAttrs.has(lower);
  // New attribute names in CSS mean earlier records lack attributes: resnapshot.
  const noteCss = text => { if (!cssAttrs) return false; let grew = false; for (const name of referenced(text)) if (!cssAttrs.has(name) && !RENDERED.has(name) && !name.startsWith('aria-')) { cssAttrs.add(name); grew = true; } return grew; };
  const dirty = { children: new Set(), attrs: new Map(), text: new Set(), form: new Set(), scroll: new Set(), replace: new Set(), sheets: new Set(), frames: new Set(), masks: new Set() };
  let waiters = [];
  const wake = () => { const list = waiters; waiters = []; for (const resolve of list) resolve(true); };
  const observer = new MutationObserver(list => { revision++; if (records.length + list.length > 100000) overflow = true; else for (const record of list) records.push(record); wake(); });
  const tainted = value => typeof value === 'string' && variants.some(secret => value.includes(secret));
  const idOf = node => { let id = ids.get(node); if (!id) { id = `n${++serial}`; ids.set(node, id); } return id; };
  const mirrored = node => { const id = ids.get(node); return id && nodes.get(id) === node ? id : null; };
  const resource = (raw, base, kind) => {
    let url; try { url = new URL(raw, base); } catch { return null; }
    if (!['http:', 'https:', 'data:'].includes(url.protocol) || tainted(url.href) || url.href.length > 4 * 1024 * 1024) return null;
    if (url.protocol === 'data:' && !/^data:(image\/(png|jpeg|gif|webp|svg\+xml)|font\/|application\/(font|x-font)|application\/octet-stream)/i.test(url.href)) return null;
    const known = urlKeys.get(url.href); if (known) return known.key;
    if (urlKeys.size >= RESOURCE_BUDGET) return null;
    const key = `r${urlKeys.size + 1}`; urlKeys.set(url.href, { key, kind }); newResources.push({ key, url: url.href, kind }); return key;
  };
  const sheetText = (sheet, base, depth = 0) => {
    let out = '';
    for (const rule of sheet.cssRules) {
      if (rule.type === 3) { // @import: inline the imported sheet when readable.
        if (depth < 8 && rule.styleSheet) { let inner = ''; try { inner = sheetText(rule.styleSheet, rule.styleSheet.href ?? base, depth + 1); } catch { inner = ''; } const media = rule.media?.mediaText; out += media ? `@media ${media}{${inner}}\n` : `${inner}\n`; }
        continue;
      }
      out += rule.cssText + '\n';
    }
    return out;
  };
  const css = (sheet, base) => sanitizeMirrorCss(sheetText(sheet, base), base, resource, variants);
  const inlineStyle = (element, base) => sanitizeMirrorCss(element.style.cssText, base, resource, variants);
  const box = node => { const r = node.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; };
  const textBox = node => { const range = node.ownerDocument.createRange(); range.selectNodeContents(node); const r = range.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; };
  const secretElement = node => node.matches(MARKERS) || /password|one-time-code|cc-/i.test(node.autocomplete ?? '') || targets.has(node)
    || tainted(node.value) || [...node.attributes].some(a => tainted(a.value));
  const listened = new WeakSet();
  const watch = (root, rootId) => {
    roots.set(rootId, root);
    observer.observe(root, { subtree: true, childList: true, attributes: true, characterData: true });
    if (root.nodeType === 9 && !listened.has(root)) {
      listened.add(root);
      const listen = (type, fn) => root.addEventListener(type, fn, { capture: true, passive: true });
      listen('scroll', event => { const target = event.target; dirty.scroll.add(target.nodeType === 9 ? target.documentElement : target); wake(); });
      listen('input', event => { dirty.form.add(event.target); wake(); }); listen('change', event => { dirty.form.add(event.target); wake(); });
      listen('focusin', wake); listen('selectionchange', wake);
      listen('load', event => { const target = event.target; if (target?.localName === 'iframe') dirty.frames.add(target); else if (target?.localName === 'img') dirty.attrs.set(target, new Set(['src'])); else if (target?.localName === 'link') dirty.sheets.add(target); else return; wake(); });
    }
  };
  // Text nodes carrying a registered value, including values split across nodes.
  const markSecrets = () => {
    marked = new WeakSet();
    if (!variants.length) return;
    const runs = []; let text = '';
    const scan = (node, depth) => {
      if (depth > DEPTH) return;
      if (node.nodeType === 3) { runs.push([node, text.length]); text += node.data; return; }
      for (const child of node.childNodes) scan(child, depth + 1);
      if (node.shadowRoot) scan(node.shadowRoot, depth + 1);
      if (node.localName === 'iframe') { let nested = null; try { nested = node.contentDocument; } catch {} if (nested) scan(nested, depth + 1); }
    };
    scan(document, 0);
    for (const value of variants) for (let at = text.indexOf(value); at >= 0; at = text.indexOf(value, at + 1)) {
      for (const [node, start] of runs) if (start < at + value.length && start + node.length > at) marked.add(node);
    }
  };
  let budget = { nodes: 0, bytes: 0 };
  const spend = (bytes) => { budget.nodes++; budget.bytes += bytes; if (budget.nodes > NODE_BUDGET) throw new Error('mirror2:node_budget'); if (budget.bytes > BYTE_BUDGET) throw new Error('mirror2:byte_budget'); };
  const forget = id => {
    const node = nodes.get(id);
    for (const child of kids.get(id) ?? []) forget(child);
    nodes.delete(id); kids.delete(id); parentOf.delete(id); kindOf.delete(id); styleNodes.delete(id); roots.delete(id); pendingHosts.delete(id); foreign.delete(id);
    if (node) ids.delete(node);
  };
  const remember = (record, node) => { nodes.set(record.id, node); parentOf.set(record.id, record.parent); kindOf.set(record.id, record.kind); kids.set(record.id, []); if (record.parent && kids.has(record.parent)) kids.get(record.parent).push(record.id); };
  const opaqueRecord = (node, record, reason) => {
    const [, , width, height] = box(node);
    record.kind = 'tile'; record.reason = reason; record.size = [width, height];
    record.tag = node.namespaceURI === HTML && !['object', 'embed', 'applet', 'frame', 'frameset', 'portal', 'fencedframe'].includes(node.localName) ? node.localName : 'div';
    const attrs = {}; for (const name of ['class', 'id', 'width', 'height']) { const value = node.getAttribute(name); if (value !== null && !tainted(value) && value.length <= 4096) attrs[name] = value; } record.attrs = attrs;
  };
  const maskRecord = (node, record) => {
    const element = node.nodeType === 1;
    const [, , width, height] = element ? box(node) : textBox(node);
    record.kind = 'mask'; record.size = [width, height];
    record.tag = element && node.namespaceURI === HTML && /^[a-z][a-z0-9]*$/.test(node.localName) && !OPAQUE.has(node.localName) && !DROP.has(node.localName) && node.localName !== 'iframe' ? node.localName : 'span';
    if (element) { const attrs = {}; for (const name of ['class', 'id']) { const value = node.getAttribute(name); if (value !== null && !tainted(value) && value.length <= 4096) attrs[name] = value; } record.attrs = attrs; record.display = getComputedStyle(node).display; }
    else record.display = 'inline-block';
  };
  const attributes = (node, base) => {
    const attrs = {}, html = node.namespaceURI === HTML;
    for (const attribute of node.attributes) {
      const name = attribute.name, lower = name.toLowerCase(), value = attribute.value;
      if (!/^[a-zA-Z_:][-a-zA-Z0-9_:.]*$/.test(name) || name.length > 256 || lower.startsWith('on') || value.length > 65536) continue;
      if (lower === 'style') { const style = inlineStyle(node, base); if (style) attrs.style = style; continue; }
      if (DENY_ATTR.has(lower) || !keepAttr(html, lower)) continue;
      // Presentation attributes may reference paint servers by fragment only.
      if (/url\s*\(/i.test(value) && !/^\s*url\(\s*["']?#[\w-]+["']?\s*\)\s*$/.test(value)) continue;
      if (/^\s*javascript:/i.test(value)) continue;
      attrs[name] = value;
    }
    const href = node.getAttribute('href') ?? node.getAttribute('xlink:href');
    if (href !== null) {
      if (!html && href.startsWith('#') && /^#[\w-]*$/.test(href)) attrs.href = href;
      else if (html && ['a', 'area'].includes(node.localName)) attrs.href = '#'; // keeps :link (an empty href would match :visited), never navigates
    }
    if (html && node.localName === 'input' && !INPUT_TYPES.has((attrs.type ?? 'text').toLowerCase())) attrs.type = 'text';
    if (node.hasAttribute('contenteditable')) attrs.contenteditable = ['true', 'false', 'plaintext-only'].includes(node.contentEditable) ? node.contentEditable : (node.isContentEditable ? 'true' : 'false');
    return attrs;
  };
  const formState = node => ({ value: String(node.value ?? '').slice(0, 65536), checked: !!node.checked, selected_index: node.selectedIndex ?? -1, selection_start: node.selectionStart ?? null, selection_end: node.selectionEnd ?? null });
  const adopted = root => { const sheets = []; for (const sheet of root.adoptedStyleSheets ?? []) { try { sheets.push(css(sheet, root.baseURI ?? document.baseURI)); } catch { sheets.push(''); } } return sheets; };
  const adoptedSignature = root => (root.adoptedStyleSheets ?? []).map(sheet => { try { return sheet.cssRules.length; } catch { return -1; } }).join(',');
  const sheetSignature = sheet => { try { return `${sheet.cssRules.length}:${sheet.disabled}`; } catch { return 'x'; } };
  const styleRecord = (node, record) => {
    record.tag = 'style'; record.attrs = {};
    const media = node.getAttribute('media'); if (media && !tainted(media) && media.length <= 4096) record.attrs.media = media;
    const sheet = node.sheet; record.css = '';
    if (sheet) {
      if (sheet.disabled) record.attrs.media = 'not all';
      try { record.css = css(sheet, sheet.href ?? node.baseURI); } catch { if (sheet.href && !tainted(sheet.href)) pendingSheets.push({ id: record.id, url: sheet.href }); }
    }
    styleNodes.set(record.id, { node, signature: sheet ? sheetSignature(sheet) : '', text: record.css });
  };
  // Pre-order records; a record's children are the following records naming it.
  const serialize = (node, parent, out, depth = 0) => {
    const type = node.nodeType;
    if (type !== 1 && type !== 3 && type !== 9 && type !== 11) return null;
    if (depth > DEPTH) return null;
    const id = idOf(node), record = { id, parent, kind: 'element' };
    if (type === 3) {
      if (node.parentNode?.localName === 'style') return null;
      if (marked.has(node)) { maskRecord(node, record); record.kind = 'mask'; }
      else { record.kind = 'text'; record.text = node.data; }
      spend(32 + (record.text?.length ?? 0)); out.push(record); remember(record, node); return id;
    }
    if (type === 9 || type === 11) {
      record.kind = type === 9 ? 'document' : 'shadow';
      const sheets = adopted(node); if (sheets.length) record.adopted = sheets;
      spend(32); out.push(record); remember(record, node);
      watch(node, id); rootSignatures.set(id, adoptedSignature(node));
      for (const child of node.childNodes) { if (type === 9 && child.nodeType !== 1) continue; serialize(child, id, out, depth + 1); }
      return id;
    }
    const tag = node.localName, ns = node.namespaceURI, base = node.baseURI;
    if (ns === HTML) {
      if (DROP.has(tag)) return null;
      if (tag === 'link') { if (!/(^|\s)stylesheet(\s|$)/i.test(node.rel) || /(^|\s)alternate(\s|$)/i.test(node.rel)) return null; }
    } else if (ns === SVG) { if (!SVG_TAGS.has(tag)) { if (tag !== 'foreignObject') return null; } }
    else if (ns === MATH) { if (!MATH_TAGS.has(tag)) return null; }
    else return null;
    if (secretElement(node)) { maskRecord(node, record); spend(64); out.push(record); remember(record, node); return id; }
    if (closedHosts.has(node)) { opaqueRecord(node, record, 'closed_shadow'); spend(64); out.push(record); remember(record, node); return id; }
    if (ns === HTML && (tag === 'style' || tag === 'link')) { styleRecord(node, record); spend(64 + record.css.length); out.push(record); remember(record, node); return id; }
    if (ns === SVG && tag === 'style') { styleRecord(node, record); record.ns = 'svg'; spend(64 + record.css.length); out.push(record); remember(record, node); return id; }
    if (ns === SVG && tag === 'foreignObject' || ns === HTML && OPAQUE.has(tag)) { opaqueRecord(node, record, tag === 'canvas' || tag === 'video' || tag === 'audio' ? 'opaque_media' : 'opaque_plugin'); spend(64); out.push(record); remember(record, node); return id; }
    if (ns === HTML && !/^[a-z][a-z0-9-]*$/.test(tag)) { record.tag = 'span'; } else record.tag = tag;
    if (ns === SVG) record.ns = 'svg'; else if (ns === MATH) record.ns = 'math';
    record.attrs = attributes(node, base);
    if (ns === HTML && tag === 'iframe') {
      let nested = null; try { nested = node.contentDocument; } catch {}
      // Cross-origin: the kernel mirrors it through the frame's own CDP session
      // (or paints it as a protected opaque region when that is unavailable).
      if (!nested?.documentElement) { const [, , width, height] = box(node); record.kind = 'frame'; record.foreign = true; record.size = [width, height]; spend(64); out.push(record); remember(record, node); foreign.add(id); return id; }
      record.kind = 'frame'; record.tag = 'iframe';
      spend(64); out.push(record); remember(record, node);
      serialize(nested, id, out, depth + 1);
      return id;
    }
    if (ns === HTML && tag === 'img' && node.currentSrc) { const key = resource(node.currentSrc, base, 'image'); if (key) record.res = key; }
    if (ns === SVG && tag === 'image') { const href = node.getAttribute('href') ?? node.getAttribute('xlink:href'); const key = href && !href.startsWith('#') ? resource(href, base, 'image') : null; if (key) record.res = key; }
    if (ns === HTML && (tag === 'input' || tag === 'textarea' || tag === 'select')) record.form = formState(node);
    if (node.scrollLeft || node.scrollTop) record.scroll = [node.scrollLeft, node.scrollTop];
    spend(64 + JSON.stringify(record.attrs).length);
    out.push(record); remember(record, node);
    for (const child of node.childNodes) serialize(child, id, out, depth + 1);
    if (node.shadowRoot) serialize(node.shadowRoot, id, out, depth + 1);
    else if (tag.includes('-')) pendingHosts.set(id, node);
    return id;
  };
  const rootSignatures = new Map();
  const replaceNode = node => { const id = mirrored(node); if (!id) return; const parent = nodes.get(parentOf.get(id)); forget(id); if (parent) dirty.children.add(parent); };
  const insideMask = node => { for (let n = node, depth = 0; n && depth < DEPTH; depth++) { const id = mirrored(n); if (id && kindOf.get(id) === 'mask') return n; n = n.parentNode ?? n.host; } return null; };
  const header = () => {
    let focused = document.activeElement;
    for (let i = 0; i < 128; i++) { let child = focused?.shadowRoot?.activeElement; try { child ??= focused?.localName === 'iframe' ? focused.contentDocument?.activeElement : null; } catch {} if (!child || child === focused) break; focused = child; }
    let selection = null;
    const selected = document.getSelection();
    if (selected && !selected.isCollapsed) { const anchor = mirrored(selected.anchorNode), focus = mirrored(selected.focusNode); if (anchor && focus && kindOf.get(anchor) === 'text' && kindOf.get(focus) === 'text') selection = { anchor_id: anchor, anchor_offset: selected.anchorOffset, focus_id: focus, focus_offset: selected.focusOffset }; }
    const focusId = focused ? mirrored(focused) : null;
    return { scroll: [scrollX, scrollY], focused: focusId && kindOf.get(focusId) !== 'mask' ? focusId : null, selection, revision };
  };
  const take = () => { const resources = newResources, sheets = pendingSheets; newResources = []; pendingSheets = []; return { resources, sheets }; };
  const configure = policy => { variants = Array.isArray(policy?.variants) ? policy.variants.filter(v => typeof v === 'string' && v) : []; };
  const resetTargets = () => { targets = new WeakSet(); return true; };
  // Full snapshot: forget every id. Resource keys stay stable per document.
  const snapshot = (policy = {}) => {
    configure(policy);
    observer.disconnect(); records = []; overflow = false;
    for (const key of Object.keys(dirty)) dirty[key].clear();
    for (const id of [...nodes.keys()]) { const node = nodes.get(id); if (node) ids.delete(node); }
    nodes.clear(); kids.clear(); parentOf.clear(); kindOf.clear(); styleNodes.clear(); roots.clear(); pendingHosts.clear(); rootSignatures.clear(); foreign.clear();
    pendingSheets = []; budget = { nodes: 0, bytes: 0 };
    newResources = [...urlKeys.entries()].map(([url, { key, kind }]) => ({ key, url, kind }));
    markSecrets();
    cssAttrs = null;
    const out = [], root = serialize(document, null, out);
    cssAttrs = new Set();
    for (const record of out) { if (record.css) noteCss(record.css); if (record.attrs?.style) noteCss(record.attrs.style); for (const text of record.adopted ?? []) noteCss(text); }
    for (const record of out) if (record.attrs && !record.ns) for (const name of Object.keys(record.attrs)) if (!keepAttr(true, name.toLowerCase())) delete record.attrs[name];
    lastSheetCheck = performance.now();
    return { root, nodes: out, ...take(), ...header() };
  };
  const childList = (node, out, ops) => {
    const id = mirrored(node), list = [];
    if (!id) return;
    const children = node.nodeType === 9 ? [...node.childNodes].filter(child => child.nodeType === 1) : [...node.childNodes];
    for (const child of children) {
      const existing = mirrored(child);
      if (existing && parentOf.get(existing) !== id) { // moved: record under its new parent
        const old = kids.get(parentOf.get(existing)); if (old) old.splice(old.indexOf(existing), 1);
        parentOf.set(existing, id);
      }
      const childId = existing ?? serialize(child, id, out, 1);
      if (childId) list.push(childId);
    }
    if (node.nodeType === 1 && node.shadowRoot) { const shadow = mirrored(node.shadowRoot) ?? serialize(node.shadowRoot, id, out, 1); if (shadow) list.push(shadow); pendingHosts.delete(id); }
    const previous = kids.get(id) ?? [];
    for (const old of previous) if (!list.includes(old) && parentOf.get(old) === id) forget(old);
    kids.set(id, list);
    ops.push({ op: 'children', id, children: list, nodes: out.splice(0) });
  };
  const drain = (policy = {}) => {
    const before = variants.join('\u0000');
    configure(policy);
    if (variants.join('\u0000') !== before) return { resync: 'policy' };
    if (overflow) return { resync: 'overflow' };
    budget = { nodes: nodes.size, bytes: 0 };
    const batch = records; records = [];
    for (const record of batch) {
      const target = record.target;
      if (record.type === 'childList') {
        if (target.localName === 'style' || target.parentNode?.localName === 'style') { dirty.sheets.add(target.localName === 'style' ? target : target.parentNode); continue; }
        dirty.children.add(target);
      } else if (record.type === 'attributes') {
        if (!dirty.attrs.has(target)) dirty.attrs.set(target, new Set());
        dirty.attrs.get(target).add(record.attributeName);
      } else if (record.type === 'characterData') {
        if (target.parentNode?.localName === 'style') dirty.sheets.add(target.parentNode); else dirty.text.add(target);
      }
    }
    if (variants.length && (dirty.text.size || dirty.children.size)) {
      markSecrets();
      for (const [id, node] of nodes) if (node.nodeType === 3 && marked.has(node) !== (kindOf.get(id) === 'mask')) dirty.replace.add(node);
    }
    const ops = [], out = [];
    // Protection status and opaque-kind changes replace the node under a new id.
    for (const [node, names] of dirty.attrs) {
      const id = mirrored(node); if (!id) { const mask = insideMask(node); if (mask) dirty.masks.add(mask); continue; }
      const kind = kindOf.get(id);
      if (node.nodeType !== 1) continue;
      if ((kind === 'mask') !== secretElement(node)) { dirty.replace.add(node); continue; }
      if (kind === 'mask' || kind === 'tile') { if (kind === 'tile') dirty.masks.add(node); continue; }
      if (styleNodes.has(id)) { dirty.sheets.add(node); continue; }
      if (node.localName === 'img' && (names.has('src') || names.has('srcset'))) { const key = node.currentSrc ? resource(node.currentSrc, node.baseURI, 'image') : null; ops.push({ op: 'res', id, res: key }); }
      if (node.localName === 'input' && names.has('type')) { dirty.replace.add(node); continue; }
      const attrs = attributes(node, node.baseURI);
      for (const name of names) {
        if (!keepAttr(node.namespaceURI === HTML, name.toLowerCase()) && name !== 'href') continue;
        if (name === 'href' || name === 'xlink:href') { ops.push({ op: 'attr', id, name: 'href', value: attrs.href ?? null }); continue; }
        ops.push({ op: 'attr', id, name, value: Object.hasOwn(attrs, name) ? attrs[name] : null });
      }
    }
    for (const node of dirty.replace) replaceNode(node);
    for (const node of dirty.frames) replaceNode(node);
    for (const node of dirty.text) {
      const id = mirrored(node); if (!id) { const mask = insideMask(node); if (mask) dirty.masks.add(mask); continue; }
      if (kindOf.get(id) === 'text') ops.push({ op: 'text', id, text: node.data });
    }
    // Upgraded custom elements attach shadow roots without a mutation record.
    for (const [id, host] of pendingHosts) if (host.shadowRoot) dirty.children.add(host);
    for (const node of dirty.children) {
      if (!node.isConnected) continue;
      const id = mirrored(node);
      if (!id) { const mask = insideMask(node); if (mask) dirty.masks.add(mask); continue; }
      const kind = kindOf.get(id);
      if (kind === 'mask' || kind === 'tile') { dirty.masks.add(node); continue; }
      if (!['element', 'document', 'shadow'].includes(kind) || styleNodes.has(id)) continue;
      childList(node, out, ops);
    }
    for (const node of dirty.masks) { const id = mirrored(node); if (id && node.isConnected) { const [, , width, height] = node.nodeType === 1 ? box(node) : textBox(node); ops.push({ op: 'size', id, size: [width, height] }); } }
    for (const node of dirty.form) { const id = mirrored(node); if (id && kindOf.get(id) === 'element' && ['input', 'textarea', 'select'].includes(node.localName)) { if (secretElement(node)) { replaceNode(node); const parent = nodes.get(parentOf.get(id)); if (parent) childList(parent, out, ops); } else ops.push({ op: 'form', id, form: formState(node) }); } }
    // Live focus may change a value without input events (programmatic writes).
    const active = document.activeElement, activeId = active && mirrored(active);
    if (activeId && kindOf.get(activeId) === 'element' && ['input', 'textarea'].includes(active.localName) && !dirty.form.has(active)) ops.push({ op: 'form', id: activeId, form: formState(active) });
    for (const node of dirty.scroll) { const id = node && mirrored(node); if (id && node !== document.documentElement && kindOf.get(id) === 'element') ops.push({ op: 'scroll', id, scroll: [node.scrollLeft, node.scrollTop] }); }
    // CSSOM edits (insertRule/replaceSync) produce no mutation record.
    const now = performance.now(), full = now - lastSheetCheck > 2000; if (full) lastSheetCheck = now;
    for (const [id, entry] of styleNodes) {
      const sheet = entry.node.sheet, signature = sheet ? sheetSignature(sheet) : '';
      if (dirty.sheets.has(entry.node) || signature !== entry.signature || full && entry.node.localName === 'style' && sheet && signature !== 'x' && sheet.cssRules.length <= 4000) {
        let text = ''; let media = entry.node.getAttribute('media');
        if (sheet) { try { text = css(sheet, sheet.href ?? entry.node.baseURI); } catch { if (sheet.href && signature !== entry.signature) pendingSheets.push({ id, url: sheet.href }); text = entry.text; } if (sheet.disabled) media = 'not all'; }
        entry.signature = signature;
        if (text !== entry.text) { entry.text = text; ops.push({ op: 'css', id, css: text }); }
        if (dirty.sheets.has(entry.node)) ops.push({ op: 'attr', id, name: 'media', value: media && !tainted(media) ? media : null });
      }
    }
    for (const [id, root] of roots) {
      const signature = adoptedSignature(root);
      if (signature !== rootSignatures.get(id)) { rootSignatures.set(id, signature); ops.push({ op: 'adopted', id, sheets: adopted(root) }); }
    }
    for (const key of Object.keys(dirty)) dirty[key].clear();
    let grew = false;
    for (const op of ops) { if (op.op === 'css') grew = noteCss(op.css) || grew; else if (op.op === 'adopted') for (const text of op.sheets) grew = noteCss(text) || grew; else if (op.op === 'children') for (const record of op.nodes) { if (record.css) grew = noteCss(record.css) || grew; for (const text of record.adopted ?? []) grew = noteCss(text) || grew; } }
    if (grew) return { resync: 'css_attributes' };
    return { ops, ...take(), ...header() };
  };
  // Opaque regions visible now, in top-level viewport CSS pixels.
  const opaqueBoxes = () => {
    const out = [];
    for (const [id, node] of nodes) {
      if (kindOf.get(id) !== 'tile' && !foreign.has(id) || !node.isConnected) continue;
      let [x, y, width, height] = box(node);
      for (let view = node.ownerDocument.defaultView; view && view !== window; view = view.parent) { const owner = view.frameElement; if (!owner) break; const b = owner.getBoundingClientRect(); x += b.x + owner.clientLeft; y += b.y + owner.clientTop; }
      if (width > 0 && height > 0 && x < innerWidth && y < innerHeight && x + width > 0 && y + height > 0) out.push({ id, box: [x, y, width, height], ...(foreign.has(id) ? { foreign: true } : {}) });
    }
    return out;
  };
  // ---- MP-11 input guards: kernel-authoritative, live, never trusting viewer geometry.
  const protectedAncestor = node => {
    for (let ancestor = node, depth = 0; ancestor && depth < DEPTH; depth++) {
      const id = mirrored(ancestor);
      if (id && kindOf.get(id) === 'mask' || ancestor.nodeType === 1 && secretElement(ancestor)) throw new Error('mirror2 protected input ancestor');
      ancestor = ancestor.parentElement ?? ancestor.getRootNode?.()?.host ?? ancestor.ownerDocument?.defaultView?.frameElement;
    }
  };
  const live = id => {
    const node = nodes.get(id);
    if (!node?.isConnected || node.nodeType !== 1 && node.nodeType !== 3) throw new Error('mirror2 stale node');
    protectedAncestor(node);
    return node;
  };
  const frameOffset = node => {
    let x = 0, y = 0;
    for (let view = node.ownerDocument.defaultView; view !== window; view = view.parent) {
      const owner = view.frameElement; if (!owner) throw new Error('mirror2 frame unavailable');
      if (getComputedStyle(owner).transform !== 'none') throw new Error('mirror2 transformed frame');
      const b = owner.getBoundingClientRect(), style = getComputedStyle(owner);
      x += b.x + owner.clientLeft + (parseFloat(style.paddingLeft) || 0); y += b.y + owner.clientTop + (parseFloat(style.paddingTop) || 0);
    }
    return [x, y];
  };
  // A point inside the live element at the viewer's offset (clamped), hit-tested.
  const point = request => {
    const node = live(request.node_id);
    if (node.nodeType !== 1) throw new Error('mirror2 element required');
    const [bx, by, width, height] = box(node);
    if (!(width > 0 && height > 0)) throw new Error('mirror2 invisible node');
    const ox = Number.isFinite(request.x) ? Math.min(Math.max(request.x, 0.5), width - 0.5) : width / 2;
    const oy = Number.isFinite(request.y) ? Math.min(Math.max(request.y, 0.5), height - 0.5) : height / 2;
    let x = bx + ox, y = by + oy;
    let hit = node.ownerDocument.elementFromPoint(x, y);
    for (let shadow = 0; shadow < 128 && hit?.shadowRoot; shadow++) { const child = hit.shadowRoot.elementFromPoint(x, y); if (!child || child === hit) break; hit = child; }
    if (!hit || hit !== node && !node.contains(hit) && !(node.shadowRoot?.contains(hit))) throw new Error('mirror2 occluded node');
    protectedAncestor(hit);
    const [fx, fy] = frameOffset(node); x += fx; y += fy;
    if (x < 0 || y < 0 || x >= innerWidth || y >= innerHeight) throw new Error('mirror2 offscreen node');
    return { x: Math.floor(x), y: Math.floor(y) };
  };
  // Re-validates at dispatch time: the same live target is still at the point.
  const hitCheck = (pt, id) => {
    const node = live(id); let owner = document, x = pt.x, y = pt.y, hit;
    for (let depth = 0; depth < 128; depth++) {
      hit = owner.elementFromPoint(x, y);
      for (let shadow = 0; shadow < 128 && hit?.shadowRoot; shadow++) { const child = hit.shadowRoot.elementFromPoint(x, y); if (!child || child === hit) break; hit = child; }
      if (hit?.localName !== 'iframe') break;
      let nested = null; try { nested = hit.contentDocument; } catch {} if (!nested) break;
      const r = hit.getBoundingClientRect(), style = getComputedStyle(hit); if (style.transform !== 'none') throw new Error('mirror2 transformed frame');
      x -= r.x + hit.clientLeft + (parseFloat(style.paddingLeft) || 0); y -= r.y + hit.clientTop + (parseFloat(style.paddingTop) || 0); owner = nested;
    }
    if (!hit || hit !== node && !node.contains(hit) && !(node.shadowRoot?.contains(hit))) throw new Error('mirror2 changed live target');
    protectedAncestor(hit);
    return true;
  };
  const activeTarget = (editable = true) => {
    let node = document.activeElement;
    for (let depth = 0; depth < 128; depth++) { let nested = node?.shadowRoot?.activeElement; try { nested ??= node?.localName === 'iframe' ? node.contentDocument?.activeElement : null; } catch {} if (!nested) break; node = nested; }
    const id = node && mirrored(node);
    if (!id || !node.isConnected || editable && !node.isContentEditable && !['input', 'textarea'].includes(node.localName)) throw new Error('mirror2 unavailable native text focus');
    protectedAncestor(node);
    return id;
  };
  const focus = request => { const node = live(request.node_id); if (node.nodeType !== 1) throw new Error('mirror2 element required'); node.focus({ preventScroll: true }); let active = node.ownerDocument.activeElement; while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement; if (active !== node) throw new Error('mirror2 focus redirected'); return true; };
  const select = request => {
    const a = live(request.anchor_id), b = live(request.focus_id);
    if (a.nodeType !== 3 || b.nodeType !== 3 || a.ownerDocument !== b.ownerDocument || !Number.isInteger(request.anchor_offset) || !Number.isInteger(request.focus_offset) || request.anchor_offset < 0 || request.anchor_offset > a.length || request.focus_offset < 0 || request.focus_offset > b.length) throw new Error('mirror2 invalid selection');
    const range = a.ownerDocument.createRange(), pa = a.ownerDocument.createRange(), pb = a.ownerDocument.createRange(); pa.setStart(a, request.anchor_offset); pa.collapse(true); pb.setStart(b, request.focus_offset); pb.collapse(true);
    if (pa.compareBoundaryPoints(Range.START_TO_START, pb) > 0) { range.setStart(b, request.focus_offset); range.setEnd(a, request.anchor_offset); } else { range.setStart(a, request.anchor_offset); range.setEnd(b, request.focus_offset); }
    for (const [id, node] of nodes) if (node.ownerDocument === a.ownerDocument && (kindOf.get(id) === 'mask' || node.nodeType === 1 && node.matches(MARKERS) || variants.some(v => (node.nodeValue ?? node.value ?? '').includes(v))) && range.intersectsNode(node)) throw new Error('mirror2 selection intersects protected content');
    a.ownerDocument.getSelection().setBaseAndExtent(a, request.anchor_offset, b, request.focus_offset); return true;
  };
  // MP-08/MP-10 local scroll (viewer-owned): the kernel follows the viewer.
  const scrollTo = request => {
    if (!request.node_id) { window.scrollTo(request.x, request.y); return [scrollX, scrollY]; }
    const node = nodes.get(request.node_id);
    // A document id scrolls that document's window (same-origin frames too).
    if (node?.nodeType === 9 && node.defaultView) { node.defaultView.scrollTo(request.x, request.y); return [node.defaultView.scrollX, node.defaultView.scrollY]; }
    const element = live(request.node_id); if (element.nodeType !== 1) throw new Error('mirror2 element required');
    element.scrollTo(request.x, request.y); return [element.scrollLeft, element.scrollTop];
  };
  // Trusted-side (DOM.resolveNode): the mirror id of a frame owner element.
  const idOfNode = function () { return this && mirrored(this) || null; };
  // Content-box origin of a cross-origin frame element in this viewport.
  const frameOrigin = id => {
    const node = live(id); if (node.localName !== 'iframe' || !foreign.has(id)) throw new Error('mirror2 frame required');
    const style = getComputedStyle(node); if (style.transform !== 'none') throw new Error('mirror2 transformed frame');
    const r = node.getBoundingClientRect(), [fx, fy] = frameOffset(node);
    return [r.x + node.clientLeft + (parseFloat(style.paddingLeft) || 0) + fx, r.y + node.clientTop + (parseFloat(style.paddingTop) || 0) + fy];
  };
  // Live focus inside a cross-origin frame: the frame element's mirror id.
  const activeForeign = () => {
    let node = document.activeElement;
    for (let depth = 0; depth < 128; depth++) { let nested = node?.shadowRoot?.activeElement; try { nested ??= node?.localName === 'iframe' ? node.contentDocument?.activeElement : null; } catch {} if (!nested) break; node = nested; }
    const id = node && mirrored(node); return id && foreign.has(id) ? id : null;
  };
  const markClosedHost = function () { if (this && this.nodeType === 1) { closedHosts.add(this); replaceNode(this); } return true; };
  // Custom elements without an open root: only then is a DOMSnapshot pass worth it.
  const customHosts = () => { let n = 0; for (const e of document.querySelectorAll('*')) if (e.localName.includes('-') && !e.shadowRoot && ++n) break; return n; };
  // Trusted-side: resolveNode() hands Vault target nodes to this world.
  const protect = function () { if (this && this.nodeType === 1) { targets.add(this); replaceNode(this); } return true; };
  // Diagnostics for the coverage oracle: viewport text runs of mirrored text.
  const textCoverage = () => {
    let mirroredText = 0, total = 0;
    const visit = (node, depth) => {
      if (depth > DEPTH) return;
      if (node.nodeType === 3) { if (!node.data.trim()) return; const [x, y, w, h] = textBox(node); if (w > 0 && h > 0 && x < innerWidth && y < innerHeight && x + w > 0 && y + h > 0) { total++; const id = mirrored(node); if (id && kindOf.get(id) === 'text') mirroredText++; } return; }
      if (node.nodeType === 1 && ['script', 'style', 'noscript', 'template'].includes(node.localName)) return;
      for (const child of node.childNodes) visit(child, depth + 1);
      if (node.shadowRoot) visit(node.shadowRoot, depth + 1);
    };
    visit(document, 0);
    return { mirrored: mirroredText, total };
  };
  // Credit long-poll: resolves on the next page change (or the bound).
  const changed = () => records.length > 0 || overflow || Object.values(dirty).some(set => set.size > 0);
  // Credit long-poll. The delta is taken in the wake microtask, right after our
  // capture-phase listener or MutationObserver callback, so an echo does not
  // wait for the page's own handlers to finish their task. A change flagged
  // between drains (input event, scroll, load) resolves immediately.
  const waitDrain = (ms, policy) => new Promise((resolve, reject) => {
    let settled = false;
    const finish = () => { if (settled) return; settled = true; try { resolve(drain(policy)); } catch (error) { reject(error); } };
    if (changed()) return finish();
    waiters.push(finish); setTimeout(finish, Math.min(Math.max(ms, 1), 2000));
  });
  const sanitize = (text, base) => sanitizeMirrorCss(String(text), String(base), resource, variants);
  // URLs this document actually fetched (Resource Timing): the kernel reads
  // only those bytes, never a URL that a stylesheet merely mentions.
  const loaded = () => { const names = new Set(performance.getEntriesByType('resource').map(entry => entry.name)); for (const img of document.images) if (img.complete && img.naturalWidth && img.currentSrc) names.add(img.currentSrc); return [...names].filter(url => urlKeys.has(url)).map(url => urlKeys.get(url).key); };
  const loadedCount = () => performance.getEntriesByType('resource').length + document.images.length;
  globalThis.__charioxMirror2 = Object.freeze({ snapshot, drain, resetTargets, waitDrain, sanitize, markClosedHost, customHosts, loaded, loadedCount, idOfNode, frameOrigin, activeForeign, opaqueBoxes, point, hitCheck, activeTarget, focus, select, scrollTo, protect, textCoverage, pending: () => records.length > 0 || overflow });
  return true;
}
