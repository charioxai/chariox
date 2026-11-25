// MP-08/MP-10/MP-11: DOM mirror v2 observer (local protocol 489). Installed only
// in the controller's isolated world. One full snapshot per document, then
// MutationObserver deltas (rrweb's model): the page's own stylesheets and
// attributes travel instead of a computed-style dump. Page code never sees
// this object; raw mutation records, URLs and unsanitized values stay here.
import { nativeFontRequirements, nativeAbsentFonts } from './kernel-browser-mirror2-fonts.mjs';
export const mirror2ObserverExpression = () => `(${installMirror2.toString()})(${sanitizeMirrorCss.toString()},${nativeFontRequirements.toString()},${nativeAbsentFonts.toString()})`;

// Pure CSS sanitizer, shared verbatim with the kernel (cross-origin sheets read
// through CDP). Every url() becomes a kernel resource key or `none`; nothing
// executable or network-addressable survives. Input must be CSSOM-serialized.
export function sanitizeMirrorCss(text, base, resource) {
  // Known namespace URIs are identifiers, never fetched; any other @namespace drops.
  const namespaces = new Set(['http://www.w3.org/1999/xhtml', 'http://www.w3.org/2000/svg', 'http://www.w3.org/1998/Math/MathML', 'http://www.w3.org/1999/xlink']);
  // Each rewrite runs only when its token occurs: most sheets have no escapes, namespaces or legacy constructs.
  let out = !/@namespace/i.test(text) ? text : text.replace(/@namespace\s+([a-zA-Z_][\w-]*\s+)?(?:url\(\s*)?["']?([^"')\s;]*)["']?\s*\)?\s*;/gi, (_, prefix = '', uri) => namespaces.has(uri) ? `@namespace ${prefix}"${uri}";` : '');
  // Escaped function names (custom properties keep raw tokens: `u\\72l(`) are
  // decoded first, so the url() rewrite below sees every fetching function. A name is matched from its first
  // character only (a match inside a name implies one a character earlier), not retried at every character of it.
  if (out.includes('\\')) out = out.replace(/(?<![a-zA-Z_-])((?:[a-zA-Z_-]|\\[0-9a-fA-F]{1,6}\s?|\\[^\n0-9a-fA-F])+)\(/g, (match, name) => {
    if (!name.includes('\\')) return match;
    const plain = name.replace(/\\(?:([0-9a-fA-F]{1,6})\s?|([\s\S]))/g, (_, hex, char) => hex ? String.fromCodePoint(Math.min(parseInt(hex, 16), 0x10ffff) || 0xfffd) : char);
    return /^[a-zA-Z_-]+$/.test(plain) ? `${plain}(` : 'x-invalid(';
  });
  // An image-set() option given as a bare string is a URL; a string inside a function (`url("a")`, `type("image/webp")`)
  // stays as written (Chrome serializes `url(a)` as `url("a")`).
  const imageSetUrls = body => {
    let result = '', depth = 0;
    for (let i = 0; i < body.length; i++) {
      const c = body[i];
      if (c === '"' || c === "'") { let end = i + 1; while (end < body.length && body[end] !== c) end += body[end] === '\\' ? 2 : 1; const string = body.slice(i, end + 1); result += depth ? string : `url(${string})`; i = end; continue; }
      if (c === '(') depth++; else if (c === ')') depth--;
      result += c;
    }
    return result;
  };
  // Custom properties and var()-bearing declarations keep their source tokens,
  // so every url() spelling occurs; image-set() strings are URLs too.
  if (/image-set\(/i.test(out)) out = out.replace(/((?:-webkit-)?image-set\()([^()]*(?:\([^()]*\)[^()]*)*)\)/gi, (_, open, body) => `${open}${imageSetUrls(body)})`);
  const urls = /url\(/i.test(out), fontFaces = !urls ? [] : [...out.matchAll(/@font-face\s*\{[^}]*\}/g)].map(match => [match.index, match.index + match[0].length]);
  const unescape = value => value.replace(/\\(?:([0-9a-fA-F]{1,6})\s?|([\s\S]))/g, (_, hex, char) => hex ? String.fromCodePoint(Math.min(parseInt(hex, 16), 0x10ffff) || 0xfffd) : char);
  if (urls) out = out.replace(/url\(\s*(?:"((?:[^"\\]|\\[\s\S])*)"|'((?:[^'\\]|\\[\s\S])*)'|((?:[^)\s"'\\]|\\[0-9a-fA-F]{1,6}\s?|\\[^\n])*))\s*\)/gi, (_, double, single, bare, at) => {
    const value = unescape(double ?? single ?? bare ?? '');
    if (!value) return 'none';
    if (value.startsWith('#')) return `url("#${value.slice(1).replace(/[^\w-]/g, '')}")`;
    const key = resource(value, base, fontFaces.some(([start, end]) => at >= start && at < end) ? 'font' : 'image');
    return key ? `url("mr:${key}")` : 'none';
  });
  // `:target` follows the document fragment (protocol 492 `fragment` header): the viewer marks that element.
  if (/:target/i.test(out)) out = out.replace(/(?<!:):target(?![\w-])/gi, '[chariox-target]');
  // `:defined` follows the kernel's definition state (protocol 492 `state.defined`): the viewer marks undefined
  // custom elements instead of registering tags in a registry its next document would reuse.
  if (/:defined/i.test(out)) out = out.replace(/(?<!:):defined(?![\w-])/gi, ':not([chariox-undefined])');
  // A link travels with href `#` (it never navigates) and its own value as inert `chariox-href`: a selector on the value
  // (`a[href*="cookie" i]`, `:not([href^="#"])`) reads that one, a presence test still the href. Only a selector changes:
  // a `[href=` in a string, a comment, another attribute's value or a declaration (a style attribute has only those)
  // stays as the page wrote it, so a statement's `[href=` is rewritten once its `{` shows it was a selector.
  // An element's src travels as a resource key, so a selector on its value (w3.org's `img[src*="logos/W3C"]`) reads
  // `chariox-src` the same way.
  if (/\[\s*(?:href|src)\s*[~|^$*]?=/i.test(out)) {
    const value = /\[(\s*)(href|src)(?=\s*[~|^$*]?=)/iy, selectors = [];
    let pending = [];
    for (let i = 0; i < out.length; i++) {
      const c = out[i];
      if (c === '\\') i++;
      else if (c === '"' || c === "'") { for (i++; i < out.length && out[i] !== c && out[i] !== '\n'; i++) if (out[i] === '\\') i++; }
      else if (c === '/' && out[i + 1] === '*') { const end = out.indexOf('*/', i + 2); i = end < 0 ? out.length : end + 1; }
      else if (c === '[') { value.lastIndex = i; if (value.test(out)) pending.push(i); }
      else if (c === '{') { selectors.push(...pending); pending = []; }
      else if (c === ';' || c === '}') pending = [];
    }
    let rewritten = '', from = 0;
    for (const at of selectors) {
      value.lastIndex = at;
      const [token, space, name] = value.exec(out);
      rewritten += `${out.slice(from, at)}[${space}chariox-${name.toLowerCase()}`;
      from = at + token.length;
    }
    out = rewritten + out.slice(from);
  }
  // Invalidates (never rewrites) legacy executable/binding constructs.
  if (/expression|behavior|-moz-binding|javascript/i.test(out)) out = out.replace(/expression\s*\(/gi, 'x-expr(').replace(/(^|[;{\s])behavior\s*:/gi, '$1x-behavior:').replace(/-moz-binding/gi, 'x-binding').replace(/javascript\s*:/gi, 'x-js:');
  if (/@import/i.test(out)) out = out.replace(/@import[^;]*;/gi, '');
  // Residual spellings the rewrites cannot parse (an unterminated url( inside a
  // string, a bare @import) are neutralized; the client refuses any that remain.
  if (/url\s*\(|@import/i.test(out)) out = out.replace(/url(\s*)\((?!"(?:mr:r[0-9]{1,9}|#[\w-]*)"\))/gi, 'urlx$1(').replace(/@import/gi, '@x-import');
  return out;
}

// The sanitizer is passed in: the isolated world has no module scope.
export function installMirror2(sanitizeMirrorCss, fontRequirements = nativeFontRequirements, absentFonts = nativeAbsentFonts) {
  if (globalThis.__charioxMirror2) return true;
  // One independent observer per subscription (viewer): its own ids, mutation
  // journal and listeners, so one viewer's snapshot never resets another's.
  // Least recently used first; over the bound (twice the service's subscriptions:
  // only leaked instances) the oldest goes, and its subscription resyncs.
  const instances = new Map();
  // Vault fill targets by node, shared by every viewer's instance: { token (value hash and fill revision), hash,
  // state }. 'pending' until the field first holds the fill (registered before the insertion: masked), 'filled'
  // while it holds it, 'retired' once it no longer does (an edit): a plain field until a refill (new revision).
  const fills = new WeakMap();
  // F5: the @font-face rules of the faces the page made by script (browser-page-fonts.mjs), set by the kernel with the serial
  // of that state; every instance adds them to the top document as its last adopted sheet.
  let scriptFonts = { serial: 0, css: '' };
  globalThis.__charioxMirror2 = Object.freeze({
    get: id => {
      let instance = instances.get(id);
      if (instance) instances.delete(id);
      else { if (instances.size >= 16) { const [oldest, old] = instances.entries().next().value; old.dispose(); instances.delete(oldest); } instance = create(); }
      instances.set(id, instance);
      return instance;
    },
    drop: id => { instances.get(id)?.dispose(); instances.delete(id); return true; },
    // CDP CSS events (kernel-browser-mirror2-css.mjs): a changed sheet's owner node, 'adopted' or 'imports'.
    sheetChanged: owner => { for (const instance of instances.values()) instance.sheetChanged(owner); return true; },
    scriptFonts: (serial, css) => {
      if (serial <= scriptFonts.serial) return true;
      scriptFonts = { serial, css };
      for (const instance of instances.values()) instance.sheetChanged('adopted');
      return true;
    },
  });
  return true;
  // SHA-256 hex of a string's UTF-8 bytes (synchronous: the drain is; crypto.subtle needs a secure context).
  function sha256(text) {
    const prime = n => { for (let d = 2; d * d <= n; d++) if (n % d === 0) return false; return true; }, primes = [];
    for (let n = 2; primes.length < 64; n++) if (prime(n)) primes.push(n);
    const K = primes.map(p => Math.floor((Math.cbrt(p) % 1) * 2 ** 32) | 0), H = primes.slice(0, 8).map(p => Math.floor((Math.sqrt(p) % 1) * 2 ** 32) | 0);
    const bytes = new TextEncoder().encode(text), total = Math.ceil((bytes.length + 9) / 64) * 64, data = new Uint8Array(total), view = new DataView(data.buffer), w = new Int32Array(64);
    data.set(bytes); data[bytes.length] = 0x80; view.setUint32(total - 8, Math.floor(bytes.length / 0x20000000)); view.setUint32(total - 4, bytes.length * 8);
    const rotr = (x, n) => x >>> n | x << (32 - n);
    for (let at = 0; at < total; at += 64) {
      for (let i = 0; i < 64; i++) w[i] = i < 16 ? view.getInt32(at + i * 4) : (rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ w[i - 2] >>> 10) + w[i - 7] + (rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ w[i - 15] >>> 3) + w[i - 16] | 0;
      let [a, b, c, d, e, f, g, h] = H;
      for (let i = 0; i < 64; i++) {
        const t1 = h + (rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)) + (e & f ^ ~e & g) + K[i] + w[i] | 0, t2 = (rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)) + (a & b ^ a & c ^ b & c) | 0;
        h = g; g = f; f = e; e = d + t1 | 0; d = c; c = b; b = a; a = t1 + t2 | 0;
      }
      [a, b, c, d, e, f, g, h].forEach((x, i) => { H[i] = H[i] + x | 0; });
    }
    return H.map(x => (x >>> 0).toString(16).padStart(8, '0')).join('');
  }
  function create() {
    const HTML = 'http://www.w3.org/1999/xhtml', SVG = 'http://www.w3.org/2000/svg', MATH = 'http://www.w3.org/1998/Math/MathML';
    // MP-08/MP-10/MP-11 M2: document bounds are distinct from per-task/packet
    // bounds. A public spec has >337k elements; no single CDP reply contains it.
    const NODE_BUDGET = 900000, BYTE_BUDGET = 64 * 1024 * 1024, HEAD_BYTES = 2 * 1024 * 1024, RESOURCE_BUDGET = 4096, DEPTH = 512;
    const DROP = new Set('script noscript template meta base title param track'.split(' '));
    const OPAQUE = new Set('canvas video audio object embed applet frame frameset portal fencedframe'.split(' '));
    // Built-in elements that accept attachShadow(); custom elements are told apart by their tag.
    const SHADOW_HOSTS = new Set('article aside blockquote body div footer h1 h2 h3 h4 h5 h6 header main nav p section span'.split(' '));
    const SVG_TAGS = new Set('svg g path circle ellipse rect line polyline polygon text tspan textPath use defs symbol clipPath mask marker pattern linearGradient radialGradient stop title desc a image switch view filter feBlend feColorMatrix feComponentTransfer feComposite feConvolveMatrix feDiffuseLighting feDisplacementMap feDistantLight feDropShadow feFlood feFuncA feFuncB feFuncG feFuncR feGaussianBlur feMerge feMergeNode feMorphology feOffset fePointLight feSpecularLighting feSpotLight feTile feTurbulence style'.split(' '));
    const MATH_TAGS = new Set('math mi mo mn ms mtext mrow mfrac msqrt mroot msub msup msubsup munder mover munderover mtable mtr mtd mspace mstyle mpadded mphantom menclose semantics annotation merror'.split(' '));
    const DENY_ATTR = new Set('src srcset href xlink:href action formaction srcdoc ping data codebase nonce integrity poster background lowsrc dynsrc manifest autofocus target download http-equiv is sizes imagesrcset imagesizes chariox-target chariox-undefined chariox-href chariox-src chariox-density'.split(' '));
    // Attribute denylist (audit 2026-10-10): every other attribute travels. Names the UA renders or
    // exposes keep values up to 64 KB; any other value up to 1 KB. Credential-like names never travel.
    const RENDERED = new Set('class id style title lang dir hidden tabindex role alt for placeholder width height colspan rowspan span type value checked selected disabled readonly multiple open start reversed size rows cols wrap label popover inert contenteditable slot part exportparts href align valign bgcolor border cellpadding cellspacing color face nowrap hspace vspace clear noshade frame rules text link vlink alink compact abbr scope summary datetime cite min max low high optimum media headers'.split(' '));
    const INPUT_TYPES = new Set('text search email url number tel checkbox radio range button submit reset date time datetime-local month week color hidden file image'.split(' '));
    const denied = lower => DENY_ATTR.has(lower) || lower.startsWith('on') || !RENDERED.has(lower) && !lower.startsWith('aria-') && /csrf|xsrf|token|nonce|secret|passw/.test(lower);
    // Owner rule (MP-11, 2026-10-09/11): mask only the fields the kernel filled from the Vault, only while
    // they are plain text fields (a password field shows dots; its value never leaves) and only while they
    // hold the fill (fills). No page-text, attribute or CSS value scanning.
    let serial = 0, targets = new Set(), records = [], overflow = false, revision = 0;
    let ids = new WeakMap();
    const nodes = new Map(), kids = new Map(), parentOf = new Map(), kindOf = new Map(), resOf = new Map();
    const styleNodes = new Map(), roots = new Map(), pendingHosts = new Map(), foreign = new Set(), shapes = new Map();
    const checkedClosed = new WeakMap(), closedTodo = new Set();
    const opaque = new Set(); // ids of opaque regions (tiles, foreign frames): none means the kernel has nothing to place
    // Protocol 503: each same-origin frame document's fragment (its `:target`) as last sent; the top document's is a header.
    // The HTML document mode, on snapshots only; limited-quirks (CSS1Compat) is told apart by its doctype, as the parser chose it.
    const documentMode = (doc = document) => doc.compatMode === 'BackCompat' ? 'quirks'
      : /^-\/\/W3C\/\/DTD XHTML 1\.0 (Frameset|Transitional)\/\//i.test(doc.doctype?.publicId ?? '')
        || !!doc.doctype?.systemId && /^-\/\/W3C\/\/DTD HTML 4\.01 (Frameset|Transitional)\/\//i.test(doc.doctype.publicId) ? 'limited-quirks' : 'no-quirks';
    const fragments = new Map(), fragmentOf = doc => { const hash = doc.location?.hash ?? ''; return hash.length > 1 ? hash.slice(1, 2049) : null; };
    // Hosts of closed shadow roots (found by the kernel's trusted DOMSnapshot pass):
    // this world cannot read their content, so they are opaque regions.
    const closedHosts = new WeakSet();
    // Each closed host's root, handed over by the kernel: only its geometry is read, for the region's still.
    const closedRoots = new WeakMap();
    // Resource keys by URL (and kind: an SVG sprite's bytes are its sanitized symbols, not the image), least
    // recently used first: over the budget the oldest mapping goes (a later use gets a new key). Keys are never
    // reused within a document, below the frame key slot.
    const urlKeys = new Map(), usedKeys = new Set(); let snapshotted = false;
    let selectedDoc = null; // the document whose selection changed last
    let newResources = [], pendingSheets = [], adoptedDirty = false, lastFormCheck = 0, formCursor = 0, lastStateCheck = 0, stateCursor = 0, lastHostCheck = 0, hostIterator = null, lastClosedCheck = 0, pressedAt = -Infinity;
    const dirty = { children: new Set(), attrs: new Map(), text: new Set(), form: new Set(), state: new Set(), scroll: new Set(), replace: new Set(), sheets: new Set(), frames: new Set(), masks: new Set() };
    let waiters = [];
    const wake = () => { const list = waiters; waiters = []; for (const resolve of list) resolve(true); };
    // While a snapshot continues, a record about a node it has not reached is moot (the walk reads that node
    // live) unless a protected box or a style element holds it; consecutive child-list changes of one parent are
    // one change. A parsing document (WHATWG, 15.8 MB) otherwise overflows and restarts its whole snapshot.
    const moot = record => {
      if (!continuation || record.target.parentNode?.localName === 'style') return false;
      for (let n = record.target, depth = 0; n && depth < DEPTH; depth++) { const id = mirrored(n); if (id) return n !== record.target && !['mask', 'tile'].includes(kindOf.get(id)); n = n.parentNode ?? n.host; }
      return true;
    };
    // The families a document's stacks name only settle with the document: a class/style/lang/face change or inserted
    // content marks the scan stale, and the next drain after a short settle sweeps again (never per keystroke).
    let stacksDirty = false, stacksScannedAt = -Infinity, stacksTimer = 0;
    const stacksDue = () => stacksDirty && performance.now() - stacksScannedAt >= 400;
    const armStacks = () => { stacksTimer ||= setTimeout(() => { stacksTimer = 0; if (!stacksDirty) return; if (stacksDue()) wake(); else armStacks(); }, 400); };
    const observer = new MutationObserver(list => {
      revision++;
      if (!stacksDirty && list.some(record => record.type === 'attributes' ? ['class', 'style', 'lang', 'face'].includes(record.attributeName) : record.type === 'childList' && record.addedNodes.length)) {
        stacksDirty = true; armStacks();
      }
      for (const record of list) {
        const last = records.at(-1);
        if (record.type === 'childList' && last?.type === 'childList' && last.target === record.target || moot(record)) continue;
        if (records.length >= 100000) { overflow = true; break; }
        records.push(record);
      }
      wake();
    });
    const idOf = node => { let id = ids.get(node); if (!id) { if (serial >= 999999) throw new Error('mirror2:node_budget'); id = `n${++serial}`; ids.set(node, id); } return id; };
    const mirrored = node => { const id = ids.get(node); return id && nodes.get(id) === node ? id : null; };
    const resource = (raw, base, kind) => {
      let url; try { url = new URL(raw, base); } catch { return null; }
      if (!['http:', 'https:', 'data:'].includes(url.protocol) || url.href.length > 4 * 1024 * 1024) return null;
      if (url.protocol === 'data:' && !/^data:(image\/(png|jpeg|gif|webp|svg\+xml)|font\/|application\/(font|x-font)|application\/octet-stream)/i.test(url.href)) return null;
      const slot = kind === 'sprite' ? `sprite:${url.href}` : url.href;
      const known = urlKeys.get(slot); if (known) { urlKeys.delete(slot); urlKeys.set(slot, known); return known.key; }
      if (usedKeys.size >= 999999) return null;
      if (urlKeys.size >= RESOURCE_BUDGET) urlKeys.delete(urlKeys.keys().next().value);
      // A URL's key derives from the URL (r1..r999999; frames rebase by slot), so a sheet that names the same
      // resources is the same text in every document of a site and the viewer's retained copy serves it.
      let number = 2166136261; for (let i = 0; i < slot.length; i++) number = Math.imul(number ^ slot.charCodeAt(i), 16777619);
      number = (number >>> 0) % 999999 + 1;
      while (usedKeys.has(`r${number}`)) number = number % 999999 + 1;
      const key = `r${number}`; usedKeys.add(key); urlKeys.set(slot, { key, kind, url: url.href }); newResources.push({ key, url: url.href, kind }); return key;
    };
    // With `clean`, each sheet's own rules are sanitized against that sheet's base
    // (an imported sheet's url() is relative to the imported sheet).
    // An element's own href or src value travels (as `chariox-href`, `chariox-src`) only once a sheet of this document
    // selects on it: the sanitizer's rewrite shows that, once per sheet it cleans. The elements sent before then re-mark
    // as changes of that value at the next drain (a sheet can turn up in the middle of one, which ends by clearing what is dirty).
    const valueNames = new Set();
    let valueStale = [];
    const valuesSelected = names => {
      for (const name of names) if (!valueNames.has(name)) {
        valueNames.add(name);
        for (const node of nodes.values()) if (node.nodeType === 1 && node.hasAttribute(name)) valueStale.push([node, name === 'href' ? 'href' : 'chariox-src']);
        if (valueStale.length) wake();
      }
    };
    const sheetCss = (text, base, doc) => {
      const out = loadViewerOptions(sanitizeMirrorCss(text, base, resource), doc);
      valuesSelected(['href', 'src'].filter(name => new RegExp(`\\[\\s*chariox-${name}\\s*[~|^$*]?=`, 'i').test(out)));
      return out;
    };
    // A cross-origin sheet the kernel sanitized itself (the same sanitizeMirrorCss, on its cached copy): each part's url()
    // occurrences register here in the sanitizer's own order, so the keys, this registry and its side effects are those of
    // sheetCss. Only the occurrences, the part's image-set() text (occurrence numbers `mr:r<n>`) and its href/src selectors come in.
    const sheetKeys = parts => parts.map(({ urls, imageSets, values }) => {
      const keys = urls.map(([value, base, kind]) => resource(String(value), String(base), String(kind)));
      if (imageSets) loadViewerOptions(String(imageSets).replace(/url\("mr:r(\d+)"\)/g, (_, n) => keys[n - 1] ? `url("mr:${keys[n - 1]}")` : 'none'), document);
      valuesSelected(values.filter(name => name === 'href' || name === 'src'));
      return keys;
    });
    const markValuesStale = () => {
      for (const [node, name] of valueStale) if (mirrored(node)) { if (!dirty.attrs.has(node)) dirty.attrs.set(node, new Set()); dirty.attrs.get(node).add(name); }
      valueStale = [];
    };
    const sheetText = (sheet, base, depth = 0, clean = null, keep = null, rejectLossy = false) => {
      let out = '', chunk = '';
      const flush = () => { out += clean && chunk ? clean(chunk, base) : chunk; chunk = ''; };
      for (const rule of sheet.cssRules) {
        if (rule.type === 3) { // @import: inline the imported sheet when readable, under its layer and conditions.
          if (depth < 8 && rule.styleSheet) {
            let inner = ''; try { inner = sheetText(rule.styleSheet, rule.styleSheet.href ?? base, depth + 1, clean, keep); } catch { inner = ''; }
            const media = rule.media?.mediaText, layer = rule.layerName, supports = rule.supportsText;
            if (media) inner = `@media ${media}{${inner}}`;
            if (supports) inner = `@supports (${supports}){${inner}}`;
            if (layer !== null && layer !== undefined) inner = `@layer ${layer}{${inner}}`; // '' is an anonymous layer
            flush(); out += `${inner}\n`;
          }
          continue;
        }
        const serialized = keep ? pruned(rule, keep) : rule.cssText + '\n';
        if (rejectLossy && LOSSY.test(serialized)) throw new Error('lossy CSSOM serialization');
        chunk += serialized;
      }
      flush();
      return out;
    };
    // GATE-bytes: a first paint carries only the style rules some node under the sheet's root matches now
    // (state pseudo-classes and pseudo-elements ignored; an unparseable selector is kept); every other rule
    // (@font-face, @keyframes, ...) stays. The kernel asks for the full text once the page is quiet.
    const DYNAMIC = /::?(?:before|after|marker|placeholder|selection|first-line|first-letter|backdrop|file-selector-button|-webkit-[\w-]+|-moz-[\w-]+)|:(?:hover|active|focus-visible|focus-within|focus|visited|target)(?![\w-])/g;
    // Top-level pieces of a selector, split where `at` says (commas, or combinators for the rightmost compound).
    const split = (text, at) => { const out = []; let depth = 0, start = 0; for (let i = 0; i < text.length; i++) { const c = text[i]; if (c === '\\') i++; else if (c === '(' || c === '[') depth++; else if (c === ')' || c === ']') depth--; else if (!depth && at(c)) { out.push(text.slice(start, i)); start = i + 1; } } out.push(text.slice(start)); return out; };
    // The document's classes, ids and tags (per root, until the next mutation) reject most unused selectors
    // without a query: a member whose rightmost compound names one the document lacks cannot match. The index also
    // lists the elements per class, id and tag, so a member is checked against the few elements its rightmost compound
    // names instead of a whole-document query per selector (most of a large sheet's pruning time).
    let usedIndex = null;
    const usedBy = root => {
      if (usedIndex?.root !== root || usedIndex.revision !== revision) {
        const classes = new Map(), ids = new Map(), tags = new Map();
        const add = (map, key, element) => { const list = map.get(key); if (list) list.push(element); else map.set(key, [element]); };
        for (const element of root.querySelectorAll('*')) { add(tags, element.localName.toLowerCase(), element); if (element.id) add(ids, element.id, element); for (const name of element.classList) add(classes, name, element); }
        usedIndex = { root, revision, classes, ids, tags, cache: new Map() };
      }
      const { classes, ids, tags, cache } = usedIndex;
      // The shortest element list the member's rightmost compound names, [] when the document lacks one of its names,
      // or null when only a query can tell (escaped or namespaced names, quirks mode, no tag, class or id).
      const candidates = member => {
        const last = split(member, c => c === ' ' || c === '>' || c === '+' || c === '~').at(-1);
        if (/\\[0-9a-fA-F]|\|/.test(last) || document.compatMode === 'BackCompat') return null;
        let flat = '', depth = 0; for (const c of last) { if (c === '(' || c === '[') depth++; else if (c === ')' || c === ']') { depth--; continue; } if (!depth) flat += c; }
        const name = token => token.replace(/\\(.)/g, '$1'), tag = /^[a-zA-Z][\w-]*/.exec(flat)?.[0];
        let best = null;
        const narrow = list => { if (!list) return false; if (!best || list.length < best.length) best = list; return true; };
        if (tag && !narrow(tags.get(tag.toLowerCase()))) return [];
        for (const [, token] of flat.matchAll(/\.((?:[\w-]|\\.)+)/g)) if (!narrow(classes.get(name(token)))) return [];
        for (const [, token] of flat.matchAll(/#((?:[\w-]|\\.)+)/g)) if (!narrow(ids.get(name(token)))) return [];
        return best;
      };
      // `:scope` and pseudo-elements mean something else to Element.matches than to a document query. A long list
      // (a common tag or class) is cheaper to check in one native query than element by element.
      const used = member => {
        const list = /:scope|::/.test(member) ? null : candidates(member);
        return list === null || list.length > 64 ? root.querySelector(member) !== null : list.some(element => element.isConnected && element.matches(member));
      };
      const matches = selector => {
        // A cross-origin sheet is pruned after the sanitizer: its href values and `:target` read as the page wrote them.
        const plain = selector.replace(/\[(\s*)chariox-(href|src)(?=\s*[~|^$*]?=)/gi, '[$1$2').replace(/\[chariox-target\]/g, '').replace(DYNAMIC, '');
        if (!plain.trim() || /(^|,)\s*(,|$)/.test(plain)) return true;
        try { return split(plain, c => c === ',').some(member => used(member.trim())); } catch { return true; }
      };
      return selector => { let used = cache.get(selector); if (used === undefined) cache.set(selector, used = matches(selector)); return used; };
    };
    const pruned = (rule, keep) => {
      if (rule instanceof CSSStyleRule) return keep(rule.selectorText) ? rule.cssText + '\n' : '';
      if (!(rule instanceof CSSMediaRule || rule instanceof CSSSupportsRule || rule instanceof CSSContainerRule || rule instanceof CSSLayerBlockRule)) return rule.cssText + '\n';
      let inner = ''; for (const child of rule.cssRules) inner += pruned(child, keep);
      // A layer block keeps its place in the layer order.
      return inner || rule instanceof CSSLayerBlockRule ? rule.cssText.slice(0, rule.cssText.indexOf('{') + 1) + inner + '}\n' : '';
    };
    // Sheets the viewer retains (digests it advertised for this snapshot): they arrive by reference, never pruned.
    const FIRST_PAINT_SHEET = 32 * 1024, fullSheets = new Map(); let retained = new Set();
    const viewerRetains = text => retained.size > 0 && retained.has(sha256(text).slice(0, 24));
    // A sheet of the top document is pruned against that one document (see usedCss): the kernel reads a repeated link once.
    const topRoot = node => window === window.top && node.getRootNode?.()?.nodeType === 9;
    const firstPaintCss = (node, sheet, base, text) => {
      const root = node.getRootNode?.();
      // Only the top document's own sheets: the kernel asks the top world for full sheets, and a shadow
      // root's :host rules cannot be checked by a query.
      if (text.length < FIRST_PAINT_SHEET || window !== window.top || document.readyState === 'loading' || root?.nodeType !== 9 || viewerRetains(text)) return text;
      const used = sheetText(sheet, base, 0, (chunk, at) => sheetCss(chunk, at, node.ownerDocument), usedBy(root));
      return used.length < text.length ? used : text;
    };
    // Chrome serializes a var() shorthand that a later declaration partly overrides as empty longhands; such
    // a linked sheet is read as source text through the kernel instead of through the CSSOM.
    const LOSSY = /[;{]\s*(?!--)[a-z][a-z-]*:\s*;/;
    // A copy that parses to fewer rules than the page's sheet (imports are inlined, unknown namespaces dropped) would
    // render a different page: a linked sheet is then read as source text through the kernel, an inline one is its own text.
    const wholeCopy = (sheet, text) => {
      let own = 0; for (const rule of sheet.cssRules) if (rule.type !== 3 && rule.type !== 10) own++;
      const copy = new CSSStyleSheet(); copy.replaceSync(text); return copy.cssRules.length >= own;
    };
    // The document a sheet belongs to: an imported sheet's is its importer's.
    const sheetDocument = sheet => { while (sheet.parentStyleSheet) sheet = sheet.parentStyleSheet; return sheet.ownerNode?.ownerDocument ?? document; };
    const css = (sheet, base) => {
      const doc = sheetDocument(sheet), text = sheetText(sheet, base, 0, (chunk, at) => sheetCss(chunk, at, doc), null, Boolean(sheet.href));
      if (!LOSSY.test(text) && wholeCopy(sheet, text)) return text;
      if (sheet.href) throw new Error('incomplete CSSOM serialization');
      // An inline <style>: its own source text is not lossy.
      const source = sheet.ownerNode?.localName === 'style' && !sheet.ownerNode.sheet?.media?.length ? sheet.ownerNode.textContent : null;
      return source ? sheetCss(source, base, doc) : text;
    };
    const inlineStyle = (element, base) => { const live = element.style.cssText; return loadViewerOptions(sanitizeMirrorCss(LOSSY.test(live) ? element.getAttribute('style') ?? live : live, base, resource), element.ownerDocument); };
    const box = node => { const r = node.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; };
    // A fixed closed host with no box of its own (Vimeo's consent manager) sits where the document ends, and the viewer's
    // document ends elsewhere. Its content is fixed too, so its region is anchored at the viewport origin on both sides.
    const viewportHost = node => { if (!closedRoots.has(node) || getComputedStyle(node).position !== 'fixed') return false; const r = node.getBoundingClientRect(); return !(r.width > 0 && r.height > 0); };
    const hostBox = node => viewportHost(node) ? [0, 0, 0, 0] : box(node);
    // Layout size (untransformed: the mirrored DOM applies the same transforms); SVG/text boxes use the rendered box.
    const layoutSize = node => node.nodeType === 1 && node.offsetWidth !== undefined ? [node.offsetWidth, node.offsetHeight] : (node.nodeType === 1 ? box(node) : textBox(node)).slice(2);
    // Row 489: a document's body with no box of its own whose closed root paints (Chromium's PDF document: a 0-high body, its
    // viewer iframe absolute in the root) is laid out in the viewer at what it paints: a 0-high tile never gets a region there.
    // Only the body: any other host keeps its layout size (its painted overflow must not push the content after it).
    const tileSize = node => { const size = layoutSize(node); if (size[0] > 0 && size[1] > 0 || !closedRoots.has(node) || node !== node.ownerDocument.body) return size; const own = box(node), ink = closedInk(node, own); return ink && ink[0] === own[0] && ink[1] === own[1] ? [Math.round(ink[2]), Math.round(ink[3])] : size; };
    const textBox = node => { const range = node.ownerDocument.createRange(); range.selectNodeContents(node); const r = range.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; };
    // Every non-password input counts, as on the pixel path (a filled month/date shows its value too).
    const plainText = node => node.localName === 'input' ? node.type !== 'password' : node.localName === 'textarea' || node.isContentEditable;
    // VAUDIT S3: a captured target may be page text (a revealed key): masked while it shows the value.
    const shownText = node => node.localName !== 'input' && node.localName !== 'textarea' && !node.isContentEditable;
    const secretElement = node => targets.has(node) && (plainText(node) || shownText(node)) && fills.get(node)?.state !== 'retired';
    // A target's fill state follows its value (sampled every drain; input events wake one).
    const verify = node => {
      const fill = fills.get(node); if (!fill?.hash || fill.state === 'retired' || !node.isConnected) return;
      const held = sha256(shownText(node) ? String(node.innerText ?? node.textContent ?? '').trim() : node.isContentEditable ? node.textContent : String(node.value ?? '')) === fill.hash;
      if (held) fill.state = 'filled'; else if (fill.state === 'filled') fill.state = 'retired';
    };
    const listened = new WeakSet(), listening = new AbortController();
    // Loaded resource URLs: a PerformanceObserver still sees entries once the page's
    // Resource Timing buffer is full (250 by default, or whatever the page set).
    const timed = new Set();
    const timing = new PerformanceObserver(list => { for (const entry of list.getEntries()) { timed.delete(entry.name); timed.add(entry.name); if (timed.size > 20000) timed.delete(timed.values().next().value); } });
    try { timing.observe({ type: 'resource', buffered: true }); } catch {}
    const dispose = () => { continuation?.return(); continuation = null; open.clear(); deferred.clear(); progressive = false; observer.disconnect(); timing.disconnect(); listening.abort(); clearTimeout(stacksTimer); records = []; nodes.clear(); kids.clear(); wake(); return true; };
    const changedForm = control => { dirty.form.add(control); if (control.multiple) for (const option of control.options ?? []) dirty.form.add(option); };
    const watch = (root, rootId) => {
      roots.set(rootId, root);
      observer.observe(root, { subtree: true, childList: true, attributes: true, characterData: true });
      // Shadow roots too: scroll, change, reset and load events do not cross their boundary.
      if (!listened.has(root)) {
        listened.add(root);
        if (root.nodeType === 9) for (const type of ['loading', 'loadingdone', 'loadingerror']) root.fonts.addEventListener(type, wake, { signal: listening.signal });
        const listen = (type, fn) => root.addEventListener(type, fn, { capture: true, passive: true, signal: listening.signal });
        if (root.nodeType === 9) listen('readystatechange', wake);
        listen('scroll', event => { const target = event.target; dirty.scroll.add(target.nodeType === 9 ? target.documentElement : target); wake(); });
        // A fragment navigation moves `:target` (the header, or a frame document's fragment op).
        if (root.nodeType === 9) root.defaultView?.addEventListener('hashchange', wake, { passive: true, signal: listening.signal });
        // The composed target: a control inside an open shadow root, not its retargeted host.
        // State the page's own handlers set in reaction (setCustomValidity) is sampled again after them.
        const formEvent = event => { const target = event.composedPath()[0] ?? event.target; changedForm(target); if (stateNodes.has(mirrored(target))) { dirty.state.add(target); setTimeout(() => { dirty.state.add(target); wake(); }, 0); } if (target.type === 'radio' && target.name) for (const radio of target.getRootNode().querySelectorAll?.('input[type=radio]') ?? []) if (radio.name === target.name) dirty.form.add(radio); wake(); };
        listen('input', formEvent); listen('change', formEvent); listen('reset', event => { for (const control of event.target.elements ?? []) changedForm(control); wake(); });
        // Focus and selection taken by a pointer press wait (50 ms at most) for the rest of its click, so the
        // click's own changes (a menu rendered a frame later) leave in the same packet; other moves wake now.
        listen('pointerdown', () => { pressedAt = performance.now(); });
        const moved = () => { const { focused, focus_visible, selection, range } = focusState(); if (JSON.stringify([focused, focus_visible, selection, range]) !== reported) wake(); };
        const soon = () => { if (performance.now() - pressedAt < 100) setTimeout(moved, 50); else moved(); };
        listen('focusin', soon);
        // A key (or a press) can turn the focused element's ring on (off) without moving the focus.
        const later = () => setTimeout(moved, 0); listen('keydown', later); listen('pointerup', later);
        // Popover and dialog transitions (toggle does not bubble: capture sees it); the cursor samples the rest.
        const stateEvent = event => { const target = event.composedPath()[0] ?? event.target; if (target?.nodeType === 1) { dirty.state.add(target); wake(); } };
        listen('toggle', stateEvent); listen('close', stateEvent); listen('selectionchange', event => { selectedDoc = event.target.nodeType === 9 ? event.target : event.target.ownerDocument; soon(); });
        listen('load', event => { const target = event.target; if (target?.localName === 'iframe') dirty.frames.add(target); else if (target?.localName === 'img') dirty.attrs.set(target, new Set(['src'])); else if (target?.localName === 'link') dirty.sheets.add(target); else return; wake(); });
      }
    };
    let budget = { nodes: 0, bytes: 0 }, continuation = null, progressive = false, yielded = false;
    // Nodes the continuation is still walking, and those whose children changed meanwhile: diffed once it leaves them.
    const open = new Set(), deferred = new Set();
    const spend = (bytes) => { if (bytes > 1024 * 1024) throw new Error('mirror2:byte_budget'); budget.nodes++; budget.bytes += bytes; if (budget.nodes > NODE_BUDGET) throw new Error('mirror2:node_budget'); if (budget.bytes > BYTE_BUDGET) throw new Error('mirror2:byte_budget'); };
    const forget = id => {
      const node = nodes.get(id);
      for (const child of kids.get(id) ?? []) forget(child);
      nodes.delete(id); kids.delete(id); parentOf.delete(id); kindOf.delete(id); resOf.delete(id); densityOf.delete(id); formNodes.delete(id); controlShapes.delete(id); imageNodes.delete(id); stateNodes.delete(id); sentStates.delete(id); shapes.delete(id); styleNodes.delete(id); roots.delete(id); reportedRequirements.delete(id); pendingHosts.delete(id); if (node) closedTodo.delete(node); foreign.delete(id); opaque.delete(id); fragments.delete(id);
      if (node) ids.delete(node);
    };
    // Mirrored form controls: property writes (value, checked, selectedIndex) make no mutation record.
    // Each option of a multiple select carries its own selectedness (selectedIndex names only the first).
    const formNodes = new Map(), imageNodes = new Map();
    // The control shape recorded for the current serialized node identity.
    const controlShapes = new Map(), controlShape = node => node.localName === 'input' ? node.type : node.localName === 'select' ? node.multiple : undefined;
    const formControl = node => ['input', 'textarea', 'select'].includes(node.localName) || node.localName === 'option' && node.closest('select')?.multiple === true;
    // A lazy image Chromium has not started loading has no currentSrc yet, but it is laid out as an image (by its
    // width/height), never as its alt text: its src travels as the key until its bytes exist.
    // The capture-phase image load handler above rechecks a late currentSrc selection without an attribute mutation.
    const imageSource = node => imageChoice(node).url;
    // The kernel's page draws at its own density (1 on the owned desktop), the viewer at the subscription's: an image
    // with candidates is drawn from the one Blink picks at the viewer's density (a density-2 viewer drew the 1x one
    // blurred), and that candidate's density travels as inert `chariox-density`, the viewer's intrinsic size for it.
    let viewerDensity = 0;
    const IMAGE_TYPES = /^image\/(png|apng|jpeg|jpg|pjpeg|gif|webp|avif|svg\+xml|bmp|x-icon|vnd\.microsoft\.icon)$/i;
    // Comma-separated parts outside parentheses (a sizes list).
    const topLevel = text => { const out = []; let depth = 0, from = 0; for (let i = 0; i < text.length; i++) { if (text[i] === '(') depth++; else if (text[i] === ')') depth--; else if (text[i] === ',' && !depth) { out.push(text.slice(from, i)); from = i + 1; } } out.push(text.slice(from)); return out; };
    // HTML srcset parsing: a URL is a run of non-space (trailing commas end it), descriptors run to a comma outside parentheses.
    const srcsetEntries = text => {
      const out = []; let i = 0;
      while (i < text.length) {
        while (i < text.length && /[\s,]/.test(text[i])) i++;
        if (i >= text.length) break;
        let from = i; while (i < text.length && !/\s/.test(text[i])) i++;
        let url = text.slice(from, i), descriptors = '';
        if (url.endsWith(',')) url = url.replace(/,+$/, '');
        else { from = i; let depth = 0; while (i < text.length && (text[i] !== ',' || depth)) { if (text[i] === '(') depth++; else if (text[i] === ')') depth--; i++; } descriptors = text.slice(from, i).trim(); i++; }
        const descriptor = descriptors ? descriptors.split(/\s+/).find(part => /^\d*\.?\d+[wx]$/.test(part)) : '1x';
        if (url && descriptor) out.push({ url, value: Number(descriptor.slice(0, -1)), unit: descriptor.at(-1) });
      }
      return out;
    };
    // A sizes length as the image's own document resolves it: its media queries read lengths as sizes does (math
    // functions, the frame's own viewport, font-relative units at the initial font). `min-width: calc(length - X +
    // 100vw)` starts to match 1/64 px below X = length and `max-width` stops 1/64 px above it, so the length is the
    // middle of the two thresholds, held as a float as Blink holds it (a 600px size bisected to 600.00001 sent its
    // candidate's density as 1.9999999 instead of 2). Each (document, viewport, length) is bisected once.
    const lengthCache = new WeakMap();
    const threshold = (view, feature, text) => {
      let low = 0, high = 2 ** 24;
      for (let middle = high / 2; middle > low && middle < high; middle = (low + high) / 2) { if (view.matchMedia(`(${feature}: calc(${text} - ${middle}px + 100vw))`).matches === (feature === 'min-width')) high = middle; else low = middle; }
      return high;
    };
    const lengthPx = (text, node) => {
      if (text === 'auto') return node.getBoundingClientRect().width || null;
      let length; try { length = CSSNumericValue.parse(text); } catch { return null; }
      const type = length.type(), literal = length instanceof CSSUnitValue;
      if (literal && length.unit === 'number') return length.value === 0 ? 0 : null;
      if (Object.keys(type).join() !== 'length' || type.length !== 1 || literal && length.value < 0) return null;
      const view = node.ownerDocument.defaultView, key = `${view.innerWidth}x${view.innerHeight} ${text}`;
      if (!lengthCache.has(node.ownerDocument)) lengthCache.set(node.ownerDocument, new Map());
      const known = lengthCache.get(node.ownerDocument);
      if (!known.has(key)) known.set(key, Math.fround((threshold(view, 'min-width', text) + threshold(view, 'max-width', text)) / 2));
      return known.get(key);
    };
    // The source size: the first sizes entry whose media matches and whose length parses; else 100vw (all in the image's document).
    const sourceSize = (sizes, node) => {
      const view = node.ownerDocument.defaultView;
      for (const entry of topLevel(sizes ?? '')) {
        const text = entry.trim(); if (!text) continue;
        let at = text.length;
        if (text.endsWith(')')) { let depth = 0; do { at--; if (text[at] === ')') depth++; else if (text[at] === '(') depth--; } while (at > 0 && depth); while (at > 0 && /[\w-]/.test(text[at - 1])) at--; }
        else at = text.search(/\S+$/);
        const media = text.slice(0, at).trim(), size = lengthPx(text.slice(at), node);
        if (size !== null && (!media || view.matchMedia(media).matches)) return size;
      }
      return view.innerWidth;
    };
    // An image's candidates with their densities: its picture's first matching source, else its own srcset (with src as 1x).
    const imageCandidates = node => {
      let set = null; const view = node.ownerDocument.defaultView;
      for (const source of node.parentElement?.localName === 'picture' ? node.parentElement.children : []) {
        if (source === node) break;
        const media = source.getAttribute('media'), type = source.getAttribute('type');
        if (source.localName !== 'source' || !source.getAttribute('srcset') || media && !view.matchMedia(media).matches || type && !IMAGE_TYPES.test(type.trim())) continue;
        set = { srcset: source.getAttribute('srcset'), sizes: source.getAttribute('sizes'), src: null }; break;
      }
      set ??= node.getAttribute('srcset') ? { srcset: node.getAttribute('srcset'), sizes: node.getAttribute('sizes'), src: node.getAttribute('src') } : null;
      if (!set) return null;
      const entries = srcsetEntries(set.srcset), widths = entries.some(entry => entry.unit === 'w'), size = widths ? sourceSize(set.sizes, node) : 1;
      const out = entries.map(entry => ({ url: entry.url, density: entry.unit === 'x' ? entry.value : entry.value / size })).filter(entry => entry.density > 0 && Number.isFinite(entry.density));
      if (set.src && !widths && !out.some(entry => entry.density === 1)) out.push({ url: set.src, density: 1 });
      return out.length ? out : null;
    };
    // The candidate at the viewer's density, loaded as the image's own document loads images (its cookies, referrer
    // policy and CORS mode), once the page's own candidate is drawn. Its bytes count only once that load drew it; a
    // failed load (an auth page, an error) leaves the image on the page's candidate.
    const viewerLoads = new WeakMap();
    const viewerLoaded = (node, url) => {
      let load = viewerLoads.get(node);
      if (load?.url !== url) {
        const image = node.ownerDocument.createElement('img');
        image.addEventListener('load', () => { if (viewerLoads.get(node)?.image === image && mirrored(node)) { if (!dirty.attrs.has(node)) dirty.attrs.set(node, new Set()); dirty.attrs.get(node).add('src'); wake(); } });
        image.referrerPolicy = node.referrerPolicy; if (node.crossOrigin !== null) image.crossOrigin = node.crossOrigin;
        image.src = url; load = { url, image }; viewerLoads.set(node, load);
      }
      return load.image.complete && load.image.naturalWidth > 0;
    };
    // The viewer's engine draws an image-set() from the option its own density picks (Blink: the lowest resolution at
    // or above it, else the highest), which the page at its density never fetched. That option loads here, as the page
    // loads its CSS images: in the document that owns the CSS (its cookies, Referer and referrer policy; a same-origin
    // frame's sheets are serialized here too), so the page's loaded resources admit its bytes.
    const RESOLUTION = { x: 1, dppx: 1, dpi: 1 / 96, dpcm: 2.54 / 96 };
    const viewerOptions = new WeakMap();
    const loadViewerOptions = (css, doc) => {
      if (!viewerDensity || viewerDensity === window.devicePixelRatio || !/image-set\(/i.test(css)) return css;
      for (const [, body] of css.matchAll(/image-set\(([^()]*(?:\([^()]*\)[^()]*)*)\)/gi)) {
        const options = topLevel(body).flatMap(option => {
          const key = /url\("mr:(r\d+)"\)/.exec(option)?.[1], type = /type\(\s*["']([^"']*)["']\s*\)/i.exec(option)?.[1];
          const resolution = /(?:^|\s)(\d*\.?\d+)(x|dppx|dpi|dpcm)\s*$/i.exec(option.replace(/(?:url|type)\([^)]*\)/gi, ' '));
          return key && (!type || IMAGE_TYPES.test(type.trim())) ? [{ key, scale: resolution ? resolution[1] * RESOLUTION[resolution[2].toLowerCase()] : 1 }] : [];
        }).sort((a, b) => a.scale - b.scale);
        const pick = options.find(option => option.scale >= viewerDensity) ?? options.at(-1);
        const url = pick && [...urlKeys.values()].find(entry => entry.key === pick.key)?.url;
        if (!viewerOptions.has(doc)) viewerOptions.set(doc, new Map());
        const loads = viewerOptions.get(doc);
        if (!url || url.startsWith('data:') || loads.has(url)) continue;
        const image = doc.createElement('img'); image.src = url; loads.set(url, image);
      }
      return css;
    };
    // Blink's pick, as for image-set(): the lowest density at or above the screen's, else the highest.
    // The page's own candidate (and its density) travels until the viewer's has loaded.
    const imageChoice = node => {
      const candidates = imageCandidates(node), own = node.currentSrc || node.src;
      if (!candidates) return { url: own, density: 1 };
      const resolve = entry => { try { return new URL(entry.url, node.baseURI).href; } catch { return null; } };
      const used = own && candidates.find(entry => resolve(entry) === own), current = { url: own, density: used?.density ?? 1 };
      const target = viewerDensity || node.ownerDocument.defaultView.devicePixelRatio;
      if (target === node.ownerDocument.defaultView.devicePixelRatio) return current;
      const sorted = candidates.sort((a, b) => a.density - b.density);
      const pick = sorted.find(entry => entry.density >= target) ?? sorted.at(-1), url = resolve(pick);
      if (!url || url === own) return url ? { url: own, density: pick.density } : current;
      return node.complete && node.naturalWidth > 0 && viewerLoaded(node, url) ? pick : current;
    };
    // The `chariox-density` value each image record last sent.
    const densityOf = new Map();
    const densityOp = (id, attrs, ops) => { const value = attrs['chariox-density'] ?? null; if (value === (densityOf.get(id) ?? null)) return; if (value) densityOf.set(id, value); else densityOf.delete(id); ops.push({ op: 'attr', id, name: 'chariox-density', value }); };
    const remember = (record, node) => { if (node.nodeType === 1 && formControl(node)) formNodes.set(record.id, node); if (node.localName === 'img') imageNodes.set(record.id, node); if (node.nodeType === 1 && ['input', 'select'].includes(node.localName)) controlShapes.set(record.id, controlShape(node)); nodes.set(record.id, node); parentOf.set(record.id, record.parent); kindOf.set(record.id, record.kind); if (record.kind === 'tile' || record.foreign) opaque.add(record.id); kids.set(record.id, []); if (record.parent && kids.has(record.parent)) kids.get(record.parent).push(record.id); };
    const opaqueRecord = (node, record, reason) => {
      const [width, height] = tileSize(node);
      record.kind = 'tile'; record.reason = reason; record.size = [width, height];
      record.tag = node.namespaceURI === HTML && !['object', 'embed', 'applet', 'frame', 'frameset', 'portal', 'fencedframe'].includes(node.localName) ? node.localName : 'div';
      shapes.set(record.id, { size: record.size, display: null });
      // MP-08/MP-11: opaque pixels still occupy the author's admitted DOM layout.
      record.attrs = attributes(node, node.baseURI);
    };
    const maskRecord = (node, record) => {
      const element = node.nodeType === 1;
      const [width, height] = layoutSize(node);
      record.kind = 'mask'; record.size = [width, height];
      record.tag = element && node.namespaceURI === HTML && /^[a-z][a-z0-9]*$/.test(node.localName) && !OPAQUE.has(node.localName) && !DROP.has(node.localName) && node.localName !== 'iframe' ? node.localName : 'span';
      if (element) { const attrs = {}; for (const name of ['class', 'id']) { const value = node.getAttribute(name); if (value !== null && value.length <= 4096) attrs[name] = value; } record.attrs = attrs; record.display = getComputedStyle(node).display; }
      else record.display = 'inline-block';
      shapes.set(record.id, { size: record.size, display: element ? record.display : null });
    };
    const attributes = (node, base) => {
      const attrs = {}, html = node.namespaceURI === HTML;
      for (const attribute of node.attributes) {
        // GATE-bytes: a tooltip shows far less than 1 KB (GitHub rows carry 64 KiB commit-message titles).
        const name = attribute.name, lower = name.toLowerCase(), value = lower === 'title' ? attribute.value.slice(0, 1024) : attribute.value;
        if (!/^[a-zA-Z_:][-a-zA-Z0-9_:.]*$/.test(name) || name.length > 256 || value.length > 65536) continue;
        if (lower === 'style') { const style = inlineStyle(node, base); if (style) attrs.style = style; continue; }
        // MP-08/MP-11: SVG geometry is rendered content, not arbitrary metadata.
        // Keep the common 64 KiB bound above; HTML and unknown attributes still
        // use the smaller privacy cap.
        const geometry = node.namespaceURI === SVG && (lower === 'd' || lower === 'points');
        if (denied(lower) || !RENDERED.has(lower) && !geometry && !lower.startsWith('aria-') && value.length > 1024) continue;
        // Password and hidden values never leave (form state carries dots / nothing).
        if (lower === 'value' && html && node.localName === 'input' && (node.type === 'password' || node.type === 'hidden')) continue;
        // Presentation attributes may reference paint servers by fragment only.
        if (/url\s*\(/i.test(value) && !/^\s*url\(\s*["']?#[\w-]+["']?\s*\)\s*$/.test(value)) continue;
        if (/javascript:/i.test(value)) continue;
        attrs[name] = value;
      }
      const href = node.getAttribute('href') ?? node.getAttribute('xlink:href');
      if (href !== null) {
        if (!html && href.startsWith('#') && /^#[\w-]*$/.test(href)) attrs.href = href;
        // An external sprite (protocol 501): the fragment travels; the file is a sanitized resource (`res`).
        else if (!html && node.localName === 'use' && spriteKey(node) && /^#[\w-]+$/.test(href.slice(href.indexOf('#')))) attrs.href = href.slice(href.indexOf('#'));
        // `#` keeps :link and never navigates. Protocol 629: a link the kernel saw visited from this origin is sent as
        // `""` instead, which the viewer draws :visited (its own document's URL).
        else if (html && ['a', 'area'].includes(node.localName)) attrs.href = visitedLinks?.has(node.href) ? '' : '#';
        // The value a stylesheet's href selector reads (see sanitizeMirrorCss), under the bounds of any other attribute.
        const own = node.getAttribute('href');
        if (valueNames.has('href') && own !== null && own.length <= 1024 && !/javascript:|url\s*\(/i.test(own)) attrs['chariox-href'] = own;
      }
      const src = valueNames.has('src') ? node.getAttribute('src') : null;
      if (src !== null && src.length <= 1024 && !/javascript:|url\s*\(/i.test(src)) attrs['chariox-src'] = src;
      if (html && node.localName === 'img') { const { density } = imageChoice(node); if (density !== 1) attrs['chariox-density'] = String(density); }
      if (html && node.localName === 'input' && !INPUT_TYPES.has((attrs.type ?? 'text').toLowerCase())) attrs.type = 'text';
      if (viewportHost(node)) attrs.style = `${attrs.style ? `${attrs.style};` : ''}position:fixed!important;inset:0 auto auto 0!important`;
      if (node.hasAttribute('contenteditable')) attrs.contenteditable = ['true', 'false', 'plaintext-only'].includes(node.contentEditable) ? node.contentEditable : (node.isContentEditable ? 'true' : 'false');
      return attrs;
    };
    // A password field renders dots of the same length; a hidden input is never shown; a file input's path never leaves.
    const formValue = node => node.type === 'password' ? '\u2022'.repeat(Math.min(String(node.value ?? '').length, 65536)) : node.type === 'hidden' || node.type === 'file' ? '' : String(node.value ?? '').slice(0, 65536);
    const formState = node => ({ value: formValue(node), checked: node.localName === 'option' ? node.selected : !!node.checked, selected_index: node.selectedIndex ?? -1, selection_start: node.selectionStart ?? null, selection_end: node.selectionEnd ?? null });
    // Last form state sent per field: a form op travels only when it changed (a
    // focused field would otherwise answer every credit at once: a busy loop),
    // and carries only the changed properties (protocol 489).
    let sentForms = new WeakMap();
    const formRecord = node => { const form = formState(node); sentForms.set(node, form); return form; };
    const formOp = (node, id, ops) => {
      const form = formState(node), sent = sentForms.get(node), changed = Object.fromEntries(Object.entries(form).filter(([name, value]) => !sent || sent[name] !== value));
      if (!Object.keys(changed).length) return;
      sentForms.set(node, form); ops.push({ op: 'form', id, form: changed });
    };
    // Protocol 492: UA state without an attribute footprint (open popover, modal dialog, indeterminate
    // checkbox, custom element definition), sampled like form state; protocol 501: custom validity
    // (setCustomValidity: :invalid with no message travelling). Records carry non-default values only.
    const stateNodes = new Map(), STATE_DEFAULT = { popover_open: false, modal: false, indeterminate: false, defined: true, custom_invalid: false };
    const VALIDATED = new Set(['input', 'select', 'textarea', 'button']);
    let sentStates = new Map();
    const stateful = node => node.nodeType === 1 && node.namespaceURI === HTML && (node.hasAttribute('popover') || node.localName === 'dialog' || VALIDATED.has(node.localName) || node.localName.includes('-'));
    const stateOf = node => {
      const out = {};
      if (VALIDATED.has(node.localName)) out.custom_invalid = node.validity.customError;
      if (node.hasAttribute('popover')) out.popover_open = node.matches(':popover-open');
      if (node.localName === 'dialog') out.modal = node.matches(':modal');
      if (node.localName === 'input' && node.type === 'checkbox') out.indeterminate = node.indeterminate;
      if (node.localName.includes('-')) out.defined = node.matches(':defined');
      return out;
    };
    const stateRecord = (node, record) => {
      const state = stateOf(node), shown = Object.fromEntries(Object.entries(state).filter(([name, value]) => value !== STATE_DEFAULT[name]));
      stateNodes.set(record.id, node); sentStates.set(record.id, state);
      if (Object.keys(shown).length) record.state = shown;
    };
    const stateOp = (node, id, ops) => {
      const state = stateOf(node), sent = sentStates.get(id) ?? {}, changed = Object.fromEntries(Object.entries(state).filter(([name, value]) => (sent[name] ?? STATE_DEFAULT[name]) !== value));
      stateNodes.set(id, node); sentStates.set(id, state);
      if (Object.keys(changed).length) ops.push({ op: 'state', id, state: changed });
    };
    // Adopted sheets have no mutation records: identity + rule count + disabled state + media each
    // drain, and their rule text when CDP reports a constructed sheet changed (replaceSync, rule edits).
    // A disabled sheet keeps its ordered slot as an empty sheet; a sheet's media list is its @media condition.
    const sheetIds = new WeakMap(), adoptedTexts = new WeakMap(); let sheetSerial = 0;
    const adopted = root => { const sheets = []; for (const sheet of root.adoptedStyleSheets ?? []) { try { const base = root.baseURI ?? document.baseURI, text = sheetText(sheet, base), media = sheet.media.mediaText; adoptedTexts.set(sheet, text); sheets.push(sheet.disabled ? '' : sheetCss(media ? `@media ${media}{${text}}` : text, base, root.ownerDocument ?? root)); } catch { sheets.push(''); } }
      if (root === document && scriptFonts.css) sheets.push(sheetCss(scriptFonts.css, document.baseURI, document));
      return sheets; };
    const adoptedSignature = root => (root.adoptedStyleSheets ?? []).map(sheet => { let id = sheetIds.get(sheet); if (!id) sheetIds.set(sheet, id = ++sheetSerial); return `${id}:${sheetSignature(sheet)}`; }).join(',') + (root === document && scriptFonts.css ? `,fonts:${scriptFonts.serial}` : '');
    const adoptedEdited = () => {
      const edited = new Map(), out = new Set();
      for (const [id, root] of roots) for (const sheet of root.adoptedStyleSheets ?? []) {
        if (!edited.has(sheet)) { let text = adoptedTexts.get(sheet); try { text = sheetText(sheet, root.baseURI ?? document.baseURI); } catch {} edited.set(sheet, text !== adoptedTexts.get(sheet)); }
        if (edited.get(sheet)) out.add(id);
      }
      return out;
    };
    // Rule count ('x' when cross-origin rules are unreadable) and, independently, the disabled state and the
    // CSSOM media list (`sheet.media.mediaText = ...` / appendMedium make no mutation and no CSS event).
    const ruleCount = sheet => { try { return String(sheet.cssRules.length); } catch { return 'x'; } };
    const sheetSignature = sheet => `${ruleCount(sheet)}:${sheet.disabled}:${sheet.media.mediaText}`;
    // @import rules lead, after any @layer statements (`@layer a; @import ...`).
    const imports = sheet => { try { for (const rule of sheet?.cssRules ?? []) { if (rule.type === 3) return true; if (!(rule instanceof CSSLayerStatementRule)) return false; } } catch {} return false; };
    const styleRecord = (node, record) => {
      record.tag = 'style'; record.attrs = {};
      const sheet = node.sheet, media = sheet ? sheet.media.mediaText : node.getAttribute('media'); record.css = '';
      if (media && media.length <= 4096) record.attrs.media = media;
      if (sheet) {
        if (sheet.disabled) record.attrs.media = 'not all';
        try { record.css = css(sheet, sheet.href ?? node.baseURI); } catch { if (sheet.href) pendingSheets.push({ id: record.id, url: sheet.href, top: topRoot(node) }); }
      }
      styleNodes.set(record.id, { node, signature: sheet ? sheetSignature(sheet) : '', text: record.css, media: record.attrs.media ?? null, imports: imports(sheet) });
      if (sheet && record.css) { const used = firstPaintCss(node, sheet, sheet.href ?? node.baseURI, record.css); if (used !== record.css) { fullSheets.set(record.id, record.css); record.css = used; } }
    };
    // One node of a walk: its record (null when it is not mirrored) and, in `walk.then`, what follows it (its children).
    const visit = (node, parent, depth, walk) => {
      if (parent && (!nodes.has(parent) || ['tile', 'mask'].includes(kindOf.get(parent)))) return null;
      const type = node.nodeType;
      if (type !== 1 && type !== 3 && type !== 9 && type !== 11) return null;
      if (depth > DEPTH) return null;
      const id = idOf(node), record = { id, parent, kind: 'element' };
      if (type === 3) {
        if (node.parentNode?.localName === 'style') return null;
        record.kind = 'text'; record.text = node.data;
        spend(32 + (record.text?.length ?? 0)); remember(record, node); return record;
      }
      if (type === 9 || type === 11) {
        record.kind = type === 9 ? 'document' : 'shadow';
        const sheets = adopted(node); if (sheets.length) record.adopted = sheets;
        // Protocol 981: a frame document's own mode (the viewer builds frame documents in no-quirks mode unless told).
        if (type === 9) { const mode = documentMode(node); if (mode !== 'no-quirks') record.document_mode = mode; }
        if (type === 9 && node !== document) { const fragment = fragmentOf(node); fragments.set(id, fragment); if (fragment) record.fragment = fragment; }
        spend(32); remember(record, node);
        walk.then = { node, id, kind: record.kind, depth, root: true }; return record;
      }
      const tag = node.localName, ns = node.namespaceURI, base = node.baseURI;
      if (ns === HTML) {
        if (DROP.has(tag)) return null;
        if (tag === 'link') { if (!/(^|\s)stylesheet(\s|$)/i.test(node.rel) || /(^|\s)alternate(\s|$)/i.test(node.rel)) return null; }
      } else if (ns === SVG) { if (!SVG_TAGS.has(tag)) { if (tag !== 'foreignObject') return null; } }
      else if (ns === MATH) { if (!MATH_TAGS.has(tag)) return null; }
      else return null;
      if (secretElement(node)) { maskRecord(node, record); spend(64); remember(record, node); return record; }
      if (closedHosts.has(node)) { opaqueRecord(node, record, 'closed_shadow'); spend(64); remember(record, node); return record; }
      if (ns === HTML && (tag === 'style' || tag === 'link')) { styleRecord(node, record); spend(64 + record.css.length); remember(record, node); return record; }
      if (ns === SVG && tag === 'style') { styleRecord(node, record); record.ns = 'svg'; spend(64 + record.css.length); remember(record, node); return record; }
      if (ns === SVG && tag === 'foreignObject' || ns === HTML && OPAQUE.has(tag)) {
        opaqueRecord(node, record, tag === 'canvas' || tag === 'video' || tag === 'audio' ? 'opaque_media' : 'opaque_plugin');
        // A video's poster travels as a resource key, as an image's source does: a tile without a still (a transformed or
        // cross-origin video, airbnb's icons) shows what Chrome draws before playback instead of an empty box.
        const poster = posterKey(node); if (poster) { record.res = poster; resOf.set(id, poster); }
        spend(64); remember(record, node); return record;
      }
      if (ns === HTML && !/^[a-z][a-z0-9-]*$/.test(tag)) { record.tag = 'span'; } else record.tag = tag;
      if (ns === SVG) record.ns = 'svg'; else if (ns === MATH) record.ns = 'math';
      record.attrs = attributes(node, base);
      if (ns === HTML && tag === 'iframe') {
        let nested = null; try { nested = node.contentDocument; } catch {}
        // Cross-origin: the kernel mirrors it through the frame's own CDP session
        // (or paints it as a protected opaque region when that is unavailable).
        if (!nested?.documentElement) { const [, , width, height] = box(node); record.kind = 'frame'; record.foreign = true; record.size = [width, height]; spend(64); remember(record, node); foreign.add(id); return record; }
        record.kind = 'frame'; record.tag = 'iframe';
        spend(64); remember(record, node);
        walk.then = { id, depth, nested }; return record;
      }
      if (ns === HTML && tag === 'img' && imageSource(node)) { const key = resource(imageSource(node), base, 'image'); if (key) { record.res = key; resOf.set(id, key); } }
      if (record.attrs['chariox-density']) densityOf.set(id, record.attrs['chariox-density']);
      if (ns === SVG && (tag === 'image' || tag === 'use')) { const key = tag === 'image' ? svgImageKey(node) : spriteKey(node); if (key) { record.res = key; resOf.set(id, key); } }
      if (ns === HTML && formControl(node)) record.form = formRecord(node);
      if (stateful(node)) stateRecord(node, record);
      if (node.scrollLeft || node.scrollTop) record.scroll = [node.scrollLeft, node.scrollTop];
      spend(64 + JSON.stringify(record.attrs).length);
      remember(record, node);
      if (tag.includes('-') && !node.shadowRoot) { pendingHosts.set(id, node); closedTodo.add(node); }
      // A built-in host's closed root leaves its light DOM empty (a consent manager's banner, for one): the kernel checks those once.
      // Row 489: empty includes whitespace text only (Chromium's PDF document is such a body, its viewer iframe in the closed root).
      else if (ns === HTML && SHADOW_HOSTS.has(tag) && !node.shadowRoot && !node.firstElementChild && !/\S/.test(node.textContent)) closedTodo.add(node);
      if (walk.defer && ns === HTML && node.firstChild && !formControl(node)) { const box = deferredBox(node, walk.defer); if (box) record.deferred = box; }
      walk.then = { node, id, kind: record.kind, depth, root: false, later: !!record.deferred }; return record;
    };
    // The children a walk reaches: a snapshot of the list, then those appended while it ran (a parsing document).
    // A child removed, moved or already serialized by a drain's diff meanwhile is skipped, never introduced twice.
    const nextChild = frame => {
      const { node, id, list } = frame;
      while (!frame.done) {
        if (frame.index === list.length) {
          // Children appended (or the list changed) while the walk was suspended: walk the ones not seen yet.
          const live = node.childNodes;
          if (live.length === list.length && list.every((child, i) => live[i] === child)) { frame.done = true; break; }
          frame.seen ??= new Set(); for (const child of list) frame.seen.add(child);
          frame.list = [...live]; frame.index = 0;
          return nextChild(frame);
        }
        if (nodes.get(id) !== node || kindOf.get(id) !== frame.kind) { frame.done = true; break; }
        const child = list[frame.index++];
        if (frame.seen?.has(child) || child.parentNode !== node || node.nodeType === 9 && child.nodeType !== 1) continue;
        // Published elsewhere and moved here: this parent's diff carries the move once the walk closes it.
        const existing = mirrored(child);
        if (existing) { if (parentOf.get(existing) !== id) dirty.children.add(node); continue; }
        return child;
      }
      if (frame.shadow) { frame.shadow = false; const root = node.shadowRoot; if (root && nodes.get(id) === node && !mirrored(root)) return root; }
      return null;
    };
    // Pre-order records; a record's children are the following records naming it. An explicit stack (not nested
    // generators, whose every record passes through each enclosing level). Each visit first yields null: even
    // dropped nodes consume a bounded amount of the caller's task.
    // A viewport-first walk (`defer`) queues the frames of deferred elements and walks them, in document order,
    // once everything else is serialized: `defer.walked` then marks the viewport complete.
    const serializeTree = (node, parent, depth = 0, defer = null) => {
      const stack = [], later = [], walk = { then: null, defer };
      let pending = { node, parent, depth }, fresh = true;
      const close = frame => { if (frame.opened) open.delete(frame.id); };
      const iterator = {
        [Symbol.iterator]() { return iterator; },
        next() {
          for (;;) {
            if (fresh) { fresh = false; return { value: null, done: false }; }
            const then = walk.then;
            if (then) {
              walk.then = null;
              if (then.nested) { if (!mirrored(then.nested)) { open.add(then.id); stack.push({ id: then.id, depth: then.depth, nested: then.nested, opened: true }); } continue; }
              if (nodes.get(then.id) !== then.node || !then.root && kindOf.get(then.id) !== then.kind) continue;
              if (then.root) { watch(then.node, then.id); rootSignatures.set(then.id, adoptedSignature(then.node)); }
              open.add(then.id);
              (then.later ? later : stack).push({ node: then.node, id: then.id, kind: then.kind, depth: then.depth, list: [...then.node.childNodes], index: 0, seen: null, done: false, shadow: !then.root, opened: true });
              continue;
            }
            if (pending) {
              const { node: next, parent: under, depth: at } = pending; pending = null;
              const record = visit(next, under, at, walk);
              if (record) return { value: record, done: false };
              continue;
            }
            const frame = stack.at(-1);
            if (!frame) {
              if (defer) defer.walked = true;
              if (later.length) { const next = later.shift(); next.list = [...next.node.childNodes]; stack.push(next); continue; }
              return { value: undefined, done: true };
            }
            let child = null;
            if (frame.nested) { child = frame.nested; frame.nested = null; } else if (frame.list) child = nextChild(frame);
            if (child) { pending = { node: child, parent: frame.id, depth: frame.depth + 1 }; fresh = true; continue; }
            stack.pop(); close(frame);
          }
        },
        return() { for (const frame of [...stack, ...later]) close(frame); stack.length = 0; later.length = 0; pending = null; walk.then = null; fresh = false; return { value: undefined, done: true }; },
      };
      return iterator;
    };
    // MP-08/MP-10 protocol 629: the elements whose box meets the viewport (a quarter screen above and below), with
    // their ancestors across shadow roots; every element the scan saw that is not among them may be deferred.
    // Sheets and SVG definitions style or paint what is on screen from anywhere: they and their ancestors are walked.
    let viewportFirst = false, deferral = null, visitedLinks = null;
    // Protocol 629: the kernel saw `url` visited from `origin` after the snapshot; this page's links to it re-mark as
    // ordinary href changes.
    const visited = (origin, url) => {
      if (!visitedLinks || origin !== location.origin || visitedLinks.has(url)) return false;
      visitedLinks.add(url);
      let marked = false;
      for (const root of roots.values()) for (const link of root.querySelectorAll?.('a[href], area[href]') ?? []) {
        if (link.href !== url || !mirrored(link)) continue;
        if (!dirty.attrs.has(link)) dirty.attrs.set(link, new Set());
        dirty.attrs.get(link).add('href'); marked = true;
      }
      if (marked) wake();
      return marked;
    };
    const viewportScan = () => {
      const scanned = new WeakSet(), shown = new WeakSet(), top = -innerHeight / 4, bottom = innerHeight * 1.25, right = innerWidth;
      const show = element => { for (let n = element; n && !shown.has(n); n = n.parentNode ?? n.host ?? null) shown.add(n); };
      const scan = list => {
        for (const element of list) {
          scanned.add(element);
          const r = element.getBoundingClientRect();
          if (r.bottom > top && r.top < bottom && r.right > 0 && r.left < right) show(element);
          if (element.shadowRoot) scan(element.shadowRoot.querySelectorAll('*'));
        }
      };
      scan(document.getElementsByTagName('*'));
      if (document.head) show(document.head);
      for (const element of document.querySelectorAll('style, link[rel~="stylesheet" i], svg defs, svg symbol')) show(element);
      return { scanned, shown, flow: new Map(), walked: false };
    };
    // A deferred element keeps its box while its children wait: [x, y, width, height] in document CSS px, the CSS
    // height (its own box-sizing) that holds it, and its top and bottom margins with those of the children that
    // collapse through it; no box (null) for display: none. Only a block-level box in normal block flow whose
    // off-screen ancestors are too, so its width comes from what stays on screen; the viewer checks every box before
    // it hands over. Values are layout units (1/64 px): rounding would add up over hundreds of boxes.
    const deferredBox = (node, defer) => {
      if (!defer.scanned.has(node) || defer.shown.has(node) || node.shadowRoot || node.getRootNode() !== document) return null;
      const style = getComputedStyle(node), round = v => Math.round(v * 64) / 64;
      if (style.display === 'none') return [0, 0, 0, 0, null, 0, 0];
      const parent = node.parentElement; if (!parent) return null;
      if (!inFlow(parent, defer) || !['block', 'flow-root', 'list-item', 'flex', 'grid', 'table'].includes(style.display) || style.float !== 'none' || !['static', 'relative'].includes(style.position) || style.transform !== 'none') return null;
      const r = node.getBoundingClientRect(); if (!r.height) return null;
      const top = collapsed(node, style, r, 'Top'), bottom = collapsed(node, style, r, 'Bottom'); if (top === null || bottom === null) return null;
      const edges = style.boxSizing === 'border-box' ? 0 : ['paddingTop', 'paddingBottom', 'borderTopWidth', 'borderBottomWidth'].reduce((sum, name) => sum + parseFloat(style[name]), 0);
      return [round(r.x + scrollX), round(r.y + scrollY), round(r.width), round(r.height), round(Math.max(0, r.height - edges)), round(top), round(bottom)];
    };
    // A container whose children's widths come from it: a block container, on screen or itself sized by such a one.
    const inFlow = (element, defer) => {
      let flow = defer.flow.get(element);
      if (flow === undefined) {
        const style = getComputedStyle(element), parent = element.parentElement;
        flow = ['block', 'flow-root', 'list-item'].includes(style.display) && (defer.shown.has(element) || !parent || style.float === 'none' && ['static', 'relative'].includes(style.position) && inFlow(parent, defer));
        defer.flow.set(element, flow);
      }
      return flow;
    };
    // The margin a box and its first (last) in-flow children share on one side: a child flush with that edge of a box
    // without padding or border there collapses through it. A negative margin in the chain is not deferred (null).
    const collapsed = (node, style, rect, side) => {
      let margin = parseFloat(style[`margin${side}`]), at = node, atStyle = style, atRect = rect;
      for (let depth = 0; depth < 32 && margin >= 0; depth++) {
        if (parseFloat(atStyle[`padding${side}`]) || parseFloat(atStyle[`border${side}Width`])) break;
        let child = side === 'Top' ? at.firstChild : at.lastChild;
        while (child && (child.nodeType === 8 || child.nodeType === 3 && !child.data.trim())) child = side === 'Top' ? child.nextSibling : child.previousSibling;
        if (child?.nodeType !== 1) break;
        const childStyle = getComputedStyle(child), childRect = child.getBoundingClientRect();
        if (!['block', 'flow-root', 'list-item', 'flex', 'grid', 'table'].includes(childStyle.display) || childStyle.float !== 'none' || !['static', 'relative'].includes(childStyle.position)) break;
        if (Math.abs(side === 'Top' ? childRect.top - atRect.top : childRect.bottom - atRect.bottom) > 0.01) break;
        const own = parseFloat(childStyle[`margin${side}`]); if (own < 0) return null;
        margin = Math.max(margin, own);
        at = child; atStyle = childStyle; atRect = childRect;
      }
      return margin >= 0 ? margin : null;
    };
    const serialize = (node, parent, out, depth = 0) => {
      for (const record of serializeTree(node, parent, depth)) if (record) out.push(record);
      return mirrored(node) ?? null;
    };
    // MP-08/MP-10: ordinary documents stay atomic up to 8 MiB of records. A
    // head/header prefix is not a usable navigation. Large documents keep the
    // bounded viewport prefix and viewer-driven continuations. The 532 transport
    // interleaves every reply in 16 KiB frames; atomic here never means one wire write.
    const chunk = (visits = 4096, minimum = 256) => {
      const atomic = !progressive, limit = atomic ? 8 * 1024 * 1024 : 128 * 1024;
      const out = [], end = performance.now() + 8, viewport = deferral && !deferral.walked; let bytes = budget.bytes;
      for (let i = 0; continuation && i < (atomic ? 40000 : visits); i++) {
        const item = continuation.next();
        if (item.done) { continuation = null; break; }
        if (item.value) out.push(item.value);
        // The viewport is complete: the deferred subtrees follow in bounded credits, after the handover.
        if (viewport && deferral.walked) { progressive = true; break; }
        // Carry bounded head CSS with the first large-document viewport prefix
        // (browserux 91599e472); ordinary resets count it inside their 8 MiB cap.
        if (!atomic && budget.bytes <= HEAD_BYTES && !(document.body && mirrored(document.body))) bytes = budget.bytes;
        // Open shadow trees are absent from the native element count. The node
        // backstop and byte cap switch those documents to bounded continuation too.
        if (budget.nodes >= 20000 || budget.bytes - bytes >= limit) progressive = true;
        if (!atomic && i >= minimum - 1 && performance.now() >= end || budget.bytes - bytes >= limit || atomic && budget.nodes >= 20000) break;
      }
      return out;
    };
    const continuationOps = records => {
      const ops = []; let op, fresh;
      for (const record of records) {
        if (!op || record.parent !== op.id && !fresh.has(record.parent)) {
          op = { op: 'children', id: record.parent, children: [], nodes: [] };
          fresh = new Set(); ops.push(op);
        }
        op.nodes.push(record); fresh.add(record.id);
      }
      for (const op of ops) op.children = [...kids.get(op.id) ?? []];
      return ops;
    };
    const rootSignatures = new Map();
    const replaceNode = node => { const id = mirrored(node); if (!id) return; const parent = nodes.get(parentOf.get(id)); forget(id); if (parent) dirty.children.add(parent); };
    const insideMask = node => { for (let n = node, depth = 0; n && depth < DEPTH; depth++) { const id = mirrored(n); if (id && kindOf.get(id) === 'mask') return n; n = n.parentNode ?? n.host; } return null; };
    const focusState = () => {
      let focused = document.activeElement;
      for (let i = 0; i < 128; i++) { let child = focused?.shadowRoot?.activeElement; try { child ??= focused?.localName === 'iframe' ? focused.contentDocument?.activeElement : null; } catch {} if (!child || child === focused) break; focused = child; }
      // Same-origin child documents have their own selection: the one changed last, else any observed one (top first).
      // A collapsed one is the caret: it travels only inside editable content.
      let selection = null;
      for (const doc of new Set([selectedDoc, document, ...roots.values()])) {
        const selected = doc?.nodeType === 9 ? doc.getSelection() : null;
        if (!selected?.rangeCount || selected.isCollapsed && !selected.anchorNode?.parentElement?.isContentEditable) continue;
        const anchor = mirrored(selected.anchorNode), focus = mirrored(selected.focusNode);
        if (anchor && focus && kindOf.get(anchor) === 'text' && kindOf.get(focus) === 'text') { selection = { anchor_id: anchor, anchor_offset: selected.anchorOffset, focus_id: focus, focus_offset: selected.focusOffset }; break; }
      }
      // Nothing focused (the page's body or root element) is null: the viewer blurs too (protocol 492).
      const focusId = focused && focused !== focused.ownerDocument.body && focused !== focused.ownerDocument.documentElement ? mirrored(focused) : null;
      // A focused text control's own range travels as a form op: part of the state a focus or selection event may move.
      const range = ['input', 'textarea'].includes(focused?.localName) ? [focused.selectionStart, focused.selectionEnd, focused.selectionDirection] : null;
      const shown = focusId && kindOf.get(focusId) !== 'mask' ? focusId : null;
      // Whether the page's browser draws its focus ring (:focus-visible: a key, not a mouse press, moved or followed it):
      // the viewer focuses its copy the same way instead of guessing from how the focus reached it.
      return { focused: shown, focus_visible: shown !== null && focused.matches(':focus-visible'), selection, range };
    };
    // MP-08/MP-10: the focus/selection state the last drain reported. A focus or selection event that leaves it
    // unchanged (focus emulation restoring the same element) does not end the long poll before the input it precedes.
    // MP-08/MP-10/MP-11: certify only descriptors for which every native
    // face failed. An unloaded/loading/loaded duplicate prevents certification.
    const fontDescriptor = font => JSON.stringify([font.family.replace(/^(['"])(.*)\1$/, '$2').trim().toLowerCase(), font.style.trim().replace(/\s+/g, ' '), font.weight === 'normal' ? '400' : font.weight.trim().replace(/\s+/g, ' '), font.stretch === 'normal' ? '100%' : font.stretch.trim(), font.unicodeRange.replace(/\s+/g, '').toUpperCase()]);
    const fontFailures = () => {
      if (document.fonts.size > 4096) return [];
      const groups = new Map();
      for (const font of document.fonts) { const key = fontDescriptor(font); if (key.length > 4096) continue; groups.set(key, (groups.get(key) ?? true) && font.status === 'error'); }
      return [...groups].filter(([, failed]) => failed).map(([key]) => sha256(key)).sort().slice(0, 128);
    };
    let reportedFonts = '[]';
    const reportedRequirements = new Map(), absentKeys = new Map(), absentOf = new Map(), installedFamilies = new Map();
    const fontOps = (ops, force = false) => {
      const due = stacksDue();
      if (due || force) { stacksDirty = false; stacksScannedAt = performance.now(); }
      for (const [id, root] of roots) {
        if (root.nodeType !== 9) continue;
        const required = fontRequirements(root, fontDescriptor, sha256), signature = JSON.stringify(required);
        // The stacks in use only settle with the document: sweep again when the requirements or completeness change.
        const key = `${signature}|${complete()}`; let changed = false;
        if (force || due || absentKeys.get(id) !== key) { absentKeys.set(id, key); const absent = absentFonts(root, installedFamilies); changed = JSON.stringify(absent) !== JSON.stringify(absentOf.get(id) ?? []); absentOf.set(id, absent); }
        if (force || changed || reportedRequirements.get(id) !== signature) { reportedRequirements.set(id, signature); const absent = absentOf.get(id); ops.push(absent.length ? { op: 'fonts', id, required, absent } : { op: 'fonts', id, required }); }
      }
      return ops;
    };
    const fontsChanged = () => [...roots].some(([id, root]) => root.nodeType === 9 && reportedRequirements.get(id) !== JSON.stringify(fontRequirements(root, fontDescriptor, sha256)));
    let reported = '', reportedComplete = false;
    const complete = () => continuation === null && document.readyState === 'complete';
    const header = () => {
      const { focused, focus_visible, selection, range } = focusState(); reported = JSON.stringify([focused, focus_visible, selection, range]);
      reportedComplete = complete(); const font_failures = fontFailures(); reportedFonts = JSON.stringify(font_failures);
      return { title: document.title, snapshot_complete: reportedComplete, ...(viewportFirst ? { snapshot_viewport_complete: (deferral ? deferral.walked : continuation === null) && document.readyState !== 'loading' } : {}), font_failures, scroll: [scrollX, scrollY], focused, focus_visible, selection, fragment: fragmentOf(document), revision, opaque: opaque.size > 0, closed_candidates: closedTodo.size > 0, full_pending: fullSheets.size };
    };
    // The next pruned sheet's full text, while its node still has that text (a changed sheet already sent its own).
    const fullSheet = () => { for (const [id, text] of fullSheets) { fullSheets.delete(id); if (styleNodes.get(id)?.text === text) return { id, css: text }; } return null; };
    const take = () => {
      const resources = newResources, sheets = pendingSheets; newResources = []; pendingSheets = []; return { resources, sheets };
    };
    const resetTargets = () => { targets = new Set(); return true; };
    // Full snapshot: forget every id. Resource keys stay stable per document.
    const snapshot = (known = [], options = {}) => {
      retained = new Set(known); viewportFirst = options.viewport_first === true; visitedLinks = Array.isArray(options.visited_links) ? new Set(options.visited_links) : null; viewerDensity = [1, 2].includes(options.density) ? options.density : 0;
      observer.disconnect(); records = []; overflow = false; sentForms = new WeakMap(); sentStates = new Map(); snapshotted = true;
      valueStale = [];
      for (const key of Object.keys(dirty)) dirty[key].clear();
      // The producer fences the old epoch before calling snapshot. Reclaim the
      // local id range without walking the previous (possibly huge) document.
      ids = new WeakMap(); serial = 0;
      reportedRequirements.clear(); absentKeys.clear(); absentOf.clear(); nodes.clear(); kids.clear(); parentOf.clear(); kindOf.clear(); resOf.clear(); formNodes.clear(); controlShapes.clear(); imageNodes.clear(); stateNodes.clear(); shapes.clear(); styleNodes.clear(); roots.clear(); pendingHosts.clear(); closedTodo.clear(); rootSignatures.clear(); foreign.clear(); opaque.clear(); fragments.clear();
      pendingSheets = []; fullSheets.clear(); hostIterator = null; budget = { nodes: 0, bytes: 0 };
      newResources = [...urlKeys.values()].map(({ key, kind, url }) => ({ key, url, kind }));
      // A parsing document has no settled viewport: it is walked in document order.
      deferral = viewportFirst && document.readyState !== 'loading' ? viewportScan() : null;
      continuation?.return(); open.clear(); deferred.clear(); continuation = serializeTree(document, null, 0, deferral);
      progressive = document.getElementsByTagName('*').length > 10000;
      // A slow task must not publish only half a detailed viewport header.
      // The first prefix has a fixed 1024-visit ceiling; later credits yield
      // by time after 256 visits, with the same byte/document bounds.
      const root = idOf(document), out = chunk(1024, 1024);
      adoptedDirty = false;
      return { root, nodes: out, ops: fontOps([], true), ...take(), ...header(), document_mode: documentMode() };
    };
    const svgImageKey = node => { const href = node.getAttribute('href') ?? node.getAttribute('xlink:href'); return href && !href.startsWith('#') ? resource(href, node.baseURI, 'image') : null; };
    // Protocol 501: `<use href="sprite.svg#icon">` names a symbol in another document (no fragment: nothing).
    const spriteKey = node => { const href = node.getAttribute('href') ?? node.getAttribute('xlink:href'), hash = href?.indexOf('#') ?? -1; return hash > 0 ? resource(href.slice(0, hash), node.baseURI, 'sprite') : null; };
    const posterKey = node => node.namespaceURI === HTML && node.localName === 'video' && node.getAttribute('poster') ? resource(node.getAttribute('poster'), node.baseURI, 'image') : null;
    const imageOp = (node, id, ops, key = imageSource(node) ? resource(imageSource(node), node.baseURI, 'image') : null) => { if ((key ?? null) === (resOf.get(id) ?? null)) return; if (key) resOf.set(id, key); else resOf.delete(id); ops.push({ op: 'res', id, res: key }); };
    // Morph (keystroke/typeahead bytes): a page that rebuilds a subtree with the
    // same shape (innerHTML of a suggestion list) keeps the viewer's nodes. A new
    // node of the same kind takes over a removed node's id; only attribute, text
    // and image differences travel. Special kinds (masks, form fields, frames,
    // styles, opaque or custom elements, shadow hosts) are never morphed.
    let rebound = [];
    const morphable = node => node.nodeType === 3 ? node.parentNode?.localName !== 'style' : node.nodeType === 1 && !node.shadowRoot && !node.localName.includes('-') && !secretElement(node) && !closedHosts.has(node)
      && (node.namespaceURI === HTML ? !DROP.has(node.localName) && !OPAQUE.has(node.localName) && !['style', 'link', 'iframe'].includes(node.localName) && !formControl(node) : node.namespaceURI === SVG && SVG_TAGS.has(node.localName) && !['style', 'image', 'use'].includes(node.localName));
    const morph = (oldId, node, ops, depth) => {
      const old = nodes.get(oldId), kind = kindOf.get(oldId);
      if (!old || old.isConnected || depth > DEPTH || !morphable(node) || node.nodeType !== old.nodeType || (node.nodeType === 3 ? kind !== 'text' : kind !== 'element' || node.localName !== old.localName || node.namespaceURI !== old.namespaceURI || (kids.get(oldId) ?? []).some(child => kindOf.get(child) === 'shadow'))) return false;
      rebound.push(oldId);
      if (node.nodeType === 3) { if (node.data !== old.data) ops.push({ op: 'text', id: oldId, text: node.data }); }
      else {
        const before = attributes(old, old.baseURI), after = attributes(node, node.baseURI);
        for (const name of new Set([...Object.keys(before), ...Object.keys(after)])) if (before[name] !== after[name] && name !== 'chariox-density') ops.push({ op: 'attr', id: oldId, name, value: after[name] ?? null });
        if (node.localName === 'img') { densityOp(oldId, after, ops); imageOp(node, oldId, ops); }
        if (stateful(node)) dirty.state.add(node); else { stateNodes.delete(oldId); sentStates.delete(oldId); }
        if (node.scrollLeft || node.scrollTop) ops.push({ op: 'scroll', id: oldId, scroll: [node.scrollLeft, node.scrollTop] });
      }
      ids.delete(old); ids.set(node, oldId); nodes.set(oldId, node);
      if (node.nodeType === 1) {
        // Children pair by position; the rest serialize fresh / are forgotten.
        const previous = kids.get(oldId) ?? [], list = [], out = [];
        [...node.childNodes].forEach((child, i) => {
          // Replacing an ancestor can move its still-live form and text nodes.
          // Re-serializing their IDs resets child lists and empties the viewer.
          const existing = mirrored(child);
          if (existing) {
            if (parentOf.get(existing) !== oldId) {
              const siblings = kids.get(parentOf.get(existing)), index = siblings?.indexOf(existing) ?? -1;
              if (index >= 0) siblings.splice(index, 1);
              parentOf.set(existing, oldId);
            }
            list.push(existing);
            return;
          }
          const reuse = previous[i] !== undefined && morph(previous[i], child, ops, depth + 1) ? previous[i] : null;
          const childId = reuse ?? serialize(child, oldId, out, depth + 1);
          if (childId) list.push(childId);
        });
        for (const child of previous) if (!list.includes(child) && parentOf.get(child) === oldId) forget(child);
        kids.set(oldId, list);
        if (out.length || list.length !== previous.length || list.some((child, i) => child !== previous[i])) ops.push({ op: 'children', id: oldId, children: list, nodes: out });
      }
      return true;
    };
    const childList = (node, out, ops) => {
      const id = mirrored(node), list = [];
      if (!id) return;
      const children = node.nodeType === 9 ? [...node.childNodes].filter(child => child.nodeType === 1) : [...node.childNodes];
      const previous = kids.get(id) ?? [];
      // Removed (detached) children of this parent, in order: morph candidates.
      const listed = new Set(children.map(child => mirrored(child)).filter(Boolean));
      const spare = previous.filter(old => !listed.has(old) && parentOf.get(old) === id && !nodes.get(old)?.isConnected);
      for (const child of children) {
        const existing = mirrored(child);
        if (existing && parentOf.get(existing) !== id) { // moved: record under its new parent
          const old = kids.get(parentOf.get(existing)); if (old) old.splice(old.indexOf(existing), 1);
          parentOf.set(existing, id);
        }
        let childId = existing;
        if (!childId && spare.length && morph(spare[0], child, ops, 1)) childId = spare.shift();
        childId ??= serialize(child, id, out, 1);
        if (childId) list.push(childId);
      }
      if (node.nodeType === 1 && node.shadowRoot) { const shadow = mirrored(node.shadowRoot) ?? serialize(node.shadowRoot, id, out, 1); if (shadow) list.push(shadow); pendingHosts.delete(id); }
      const kept = new Set(list); for (const old of previous) if (!kept.has(old) && parentOf.get(old) === id) forget(old);
      kids.set(id, list);
      // A child list that ends as it was (a node added and removed) sends nothing.
      if (!out.length && list.length === previous.length && list.every((child, i) => child === previous[i])) return;
      ops.push({ op: 'children', id, children: list, nodes: out.splice(0) });
    };
    const drain = () => {
      if (!snapshotted) return { resync: 'evicted' }; // a new instance for a running subscription
      if (overflow) return { resync: 'overflow' };
      if (!progressive && document.getElementsByTagName('*').length > 10000) return { resync: 'growing' };
      markValuesStale();
      const urgent = records.length > 0 || Object.values(dirty).some(set => set.size > 0);
      if (!continuation) budget = { nodes: nodes.size, bytes: 0 };
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
      const ops = [], out = []; rebound = [];
      for (const node of targets) {
        verify(node);
        const id = mirrored(node);
        // Fill retirement is shared; each viewer still reconciles its own recorded mask.
        if (id && (kindOf.get(id) === 'mask') !== secretElement(node)) dirty.replace.add(node);
      }
      // Protection status and opaque-kind changes replace the node under a new id.
      for (const [node, names] of dirty.attrs) {
        const id = mirrored(node); if (!id) { const mask = insideMask(node); if (mask) dirty.masks.add(mask); continue; }
        const kind = kindOf.get(id);
        if (node.nodeType !== 1) continue;
        if ((kind === 'mask') !== secretElement(node)) { dirty.replace.add(node); continue; }
        if (kind === 'mask') continue; // protected boxes retain only their admitted mask geometry
        if (styleNodes.has(id)) { dirty.sheets.add(node); continue; }
        const image = node.localName === 'img' && (names.has('src') || names.has('srcset') || names.has('sizes'));
        if (image) imageOp(node, id, ops);
        if (kind === 'tile' && node.localName === 'video' && names.has('poster')) imageOp(node, id, ops, posterKey(node));
        if (names.has('popover') || names.has('open')) { if (stateful(node)) dirty.state.add(node); else { stateNodes.delete(id); sentStates.delete(id); } }
        // A new input type, or a select gaining/losing `multiple` (its options become/stop being form controls), re-serializes.
        // Pages re-set the same type on every render (github.com): that keeps the viewer's node (and its focus/caret).
        if ((node.localName === 'input' && names.has('type') || node.localName === 'select' && names.has('multiple')) && controlShape(node) !== controlShapes.get(id)) { dirty.replace.add(node); continue; }
        const attrs = attributes(node, node.baseURI);
        if (image) densityOp(id, attrs, ops);
        for (const name of names) {
          if (name === 'href' || name === 'xlink:href') {
            // An SVG image's external URL travels as a resource key (the viewer sets href to the bytes); a
            // sprite's as a key plus its fragment.
            const keyed = node.namespaceURI === SVG && ['image', 'use'].includes(node.localName), key = keyed ? (node.localName === 'image' ? svgImageKey(node) : spriteKey(node)) : null;
            if (keyed) imageOp(node, id, ops, key);
            if (!key || node.localName === 'use') ops.push({ op: 'attr', id, name: 'href', value: attrs.href ?? null });
            if (valueNames.has('href')) ops.push({ op: 'attr', id, name: 'chariox-href', value: attrs['chariox-href'] ?? null });
            continue;
          }
          if ((name === 'src' || name === 'chariox-src') && valueNames.has('src')) ops.push({ op: 'attr', id, name: 'chariox-src', value: attrs['chariox-src'] ?? null });
          if (denied(name.toLowerCase())) continue; // never travels (the viewer's own src/handlers stay)
          ops.push({ op: 'attr', id, name, value: Object.hasOwn(attrs, name) ? attrs[name] : null });
        }
      }
      for (const node of dirty.replace) replaceNode(node);
      for (const node of dirty.frames) replaceNode(node);
      for (const node of dirty.text) {
        const id = mirrored(node); if (!id) { const mask = insideMask(node); if (mask) dirty.masks.add(mask); continue; }
        if (kindOf.get(id) === 'text') ops.push({ op: 'text', id, text: node.data });
      }
      // Upgraded custom elements attach shadow roots without a mutation record; so do ordinary hosts
      // attaching one late (a bounded rotating check, 5000 nodes per 2 s).
      for (const [id, host] of pendingHosts) if (host.shadowRoot) dirty.children.add(host);
      const hostNow = performance.now();
      if (hostNow - lastHostCheck > 2000 && nodes.size) { lastHostCheck = hostNow; for (let i = 0; i < Math.min(5000, nodes.size); i++) { hostIterator ??= nodes.keys(); let item = hostIterator.next(); if (item.done) { hostIterator = nodes.keys(); item = hostIterator.next(); } const id = item.value, node = nodes.get(id); if (node && kindOf.get(id) === 'element') { if (node.shadowRoot && !mirrored(node.shadowRoot)) dirty.children.add(node); else if (pendingHosts.has(id) && (checkedClosed.get(node) !== node.matches(':defined') || node.matches(':defined') && hostNow - lastClosedCheck >= 2000)) closedTodo.add(node); } } }
      // A snapshot spanning tasks is never restarted by a mutation (MP-10, BROWSER-UX-09): the
      // continuation reads each node when it reaches it. Children of nodes it already left are diffed
      // now; those of a node it is still walking wait until it closes, but drop forgotten children now.
      for (const node of deferred) { const id = mirrored(node); if (!id) deferred.delete(node); else if (!open.has(id)) { deferred.delete(node); dirty.children.add(node); } }
      for (const node of dirty.children) {
        const id = mirrored(node); if (!id || !open.has(id)) continue;
        dirty.children.delete(node); deferred.add(node);
        const list = kids.get(id), kept = list.filter(child => nodes.has(child));
        if (kept.length !== list.length) { kids.set(id, kept); ops.push({ op: 'children', id, children: [...kept], nodes: [] }); }
      }
      for (const node of dirty.children) {
        if (!node.isConnected) continue;
        const id = mirrored(node);
        if (!id) { const mask = insideMask(node); if (mask) dirty.masks.add(mask); continue; }
        const kind = kindOf.get(id);
        if (kind === 'mask' || kind === 'tile') { dirty.masks.add(node); continue; }
        if (!['element', 'document', 'shadow'].includes(kind) || styleNodes.has(id)) continue;
        childList(node, out, ops);
      }
      // Masks and opaque regions (few): their size, and a mask's display, follow layout and style without
      // records (a Vault-filled field toggling display, a canvas resized by CSS); a new display re-serializes.
      for (const id of shapes.keys()) {
        const node = nodes.get(id), kind = kindOf.get(id); if (!node?.isConnected) continue;
        const size = kind === 'tile' ? tileSize(node) : layoutSize(node), display = kind === 'mask' && node.nodeType === 1 ? getComputedStyle(node).display : null, last = shapes.get(id);
        if (last && display !== last.display) { const parent = nodes.get(parentOf.get(id)); replaceNode(node); if (parent) childList(parent, out, ops); continue; }
        if (last && (last.size[0] !== size[0] || last.size[1] !== size[1])) ops.push({ op: 'size', id, size });
        shapes.set(id, { size, display });
      }
      // Bounded reconciliation, 2000 controls per check from a rotating cursor (every control is
      // eventually sampled); unchanged controls stay silent (formOp compares with the last sent state).
      const formNow = performance.now();
      if (formNow - lastFormCheck > 2000 && formNodes.size) { lastFormCheck = formNow; const list = [...formNodes.values()]; formCursor %= list.length; for (let i = 0; i < Math.min(2000, list.length); i++) dirty.form.add(list[(formCursor + i) % list.length]); formCursor += 2000; }
      if (formNow - lastStateCheck > 2000 && stateNodes.size) { lastStateCheck = formNow; const list = [...stateNodes.values()]; stateCursor %= list.length; for (let i = 0; i < Math.min(2000, list.length); i++) dirty.state.add(list[(stateCursor + i) % list.length]); stateCursor += 2000; }
      for (const node of dirty.state) { const id = mirrored(node); if (id && kindOf.get(id) === 'element' && stateful(node)) stateOp(node, id, ops); }
      for (const node of dirty.form) { const id = mirrored(node); if (id && kindOf.get(id) === 'element' && formControl(node)) { if (secretElement(node)) { const parent = nodes.get(parentOf.get(id)); replaceNode(node); if (parent) childList(parent, out, ops); } else formOp(node, id, ops); } }
      // Live focus may change a value without input events (programmatic writes).
      let active = document.activeElement; while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
      const activeId = active && mirrored(active);
      if (activeId && kindOf.get(activeId) === 'element' && ['input', 'textarea'].includes(active.localName) && !dirty.form.has(active)) formOp(active, activeId, ops);
      for (const node of dirty.scroll) { const id = node && mirrored(node); if (id && node !== document.documentElement && kindOf.get(id) === 'element') ops.push({ op: 'scroll', id, scroll: [node.scrollLeft, node.scrollTop] }); }
      // CSSOM edits (insertRule/replaceSync, rule.style writes) produce no mutation record: CDP reports
      // them (sheetChanged); the cheap signature (rule count, disabled) still runs every drain.
      for (const [id, entry] of styleNodes) {
        const sheet = entry.node.sheet, signature = sheet ? sheetSignature(sheet) : '';
        if (dirty.sheets.has(entry.node) || signature !== entry.signature) {
          let text = '', media = sheet ? sheet.media.mediaText : entry.node.getAttribute('media'); if (media && media.length > 4096) media = null;
          // An unreadable sheet is re-read through CDP when it (re)loaded, not when only its disabled state changed.
          if (sheet) { try { text = css(sheet, sheet.href ?? entry.node.baseURI); } catch { if (sheet.href && (dirty.sheets.has(entry.node) || !entry.signature.startsWith('x:'))) pendingSheets.push({ id, url: sheet.href }); text = entry.text; } if (sheet.disabled) media = 'not all'; }
          entry.signature = signature; entry.imports = imports(sheet);
          if (text !== entry.text) { entry.text = text; ops.push({ op: 'css', id, css: text }); }
          // Effective media: the CSSOM media list and `sheet.disabled` change it without any mutation or event.
          if ((media || null) !== entry.media) { entry.media = media || null; ops.push({ op: 'attr', id, name: 'media', value: entry.media }); }
        }
      }
      const edited = adoptedDirty ? adoptedEdited() : new Set(); adoptedDirty = false;
      for (const [id, root] of roots) {
        const signature = adoptedSignature(root);
        if (signature !== rootSignatures.get(id) || edited.has(id)) { rootSignatures.set(id, signature); ops.push({ op: 'adopted', id, sheets: adopted(root) }); }
      }
      for (const [id, last] of fragments) { const doc = nodes.get(id), now = doc ? fragmentOf(doc) : last; if (now !== last) { fragments.set(id, now); ops.push({ op: 'fragment', id, fragment: now }); } }
      for (const key of Object.keys(dirty)) dirty[key].clear();
      // Echoes and page changes go alone for one credit; the continuation then resumes even on a busy page.
      if (continuation && (yielded || !urgent && !ops.length)) { yielded = false; ops.push(...continuationOps(chunk())); } else if (continuation) yielded = true;
      return { ops: fontOps(ops), changed: rebound, ...take(), ...header(), closed_candidates: closedTodo.size > 0, pressed: Math.max(0, Math.ceil(pressedAt + 50 - performance.now())), loaded: loadedCount() };
    };
    const topBox = node => {
      let [x, y, width, height] = hostBox(node);
      for (let view = node.ownerDocument.defaultView; view && view !== window; view = view.parent) { const owner = view.frameElement; if (!owner) break; const b = owner.getBoundingClientRect(); x += b.x + owner.clientLeft; y += b.y + owner.clientTop; }
      return [x, y, width, height];
    };
    // What a closed root's content paints, which can lie outside its host's own box (a 0x0 host with fixed content),
    // in the host's coordinates; null when it adds nothing to the host's box.
    const closedInk = (node, [x, y, width, height]) => {
      const root = closedRoots.get(node); if (!root) return null;
      // The host's own frame offset (topBox) applies to its content as well.
      const [dx, dy] = [x - hostBox(node)[0], y - hostBox(node)[1]];
      let left = x, top = y, right = x + width, bottom = y + height, empty = !(width > 0 && height > 0), grown = false, seen = 0;
      for (const element of root.querySelectorAll('*')) {
        if (++seen > 4096) break;
        const r = element.getBoundingClientRect();
        if (!(r.width > 0 && r.height > 0) || invisible(element)) continue;
        const [l, t] = [r.x + dx, r.y + dy];
        if (empty) { [left, top, right, bottom, empty] = [l, t, l + r.width, t + r.height, false]; grown = true; continue; }
        if (l < left || t < top || l + r.width > right || t + r.height > bottom) grown = true;
        left = Math.min(left, l); top = Math.min(top, t); right = Math.max(right, l + r.width); bottom = Math.max(bottom, t + r.height);
      }
      return grown ? [left, top, right - left, bottom - top] : null;
    };
    // Opaque regions visible now, in top-level viewport CSS pixels. A closed root's region also gives the box its content
    // paints (ink), which its still covers.
    const opaqueBoxes = () => {
      const out = [];
      for (const [id, node] of nodes) {
        if (kindOf.get(id) !== 'tile' && !foreign.has(id) || !node.isConnected) continue;
        const own = topBox(node), ink = closedInk(node, own), [x, y, width, height] = ink ?? own;
        if (width > 0 && height > 0 && x < innerWidth && y < innerHeight && x + width > 0 && y + height > 0 && !invisible(node)) { const cause = distorted(node); out.push({ id, box: own, ...(ink ? { ink } : {}), view: [innerWidth, innerHeight], ...(foreign.has(id) ? { foreign: true } : {}), ...(cause ? { distorted: cause, tag: node.localName } : {}) }); }
      }
      return out;
    };
    // §2a lease geometry: live protection/identity and every owning viewport.
    // Private values only; no new packet fields or client-selected rectangles.
    const regionShape = id => {
      const node = live(id), kind = kindOf.get(id);
      if (kind !== 'tile' && !(kind === 'frame' && foreign.has(id))) throw new Error('mirror2 region required');
      const intersect = (a,b) => { const x=Math.max(a[0],b[0]),y=Math.max(a[1],b[1]);return [x,y,Math.max(0,Math.min(a[0]+a[2],b[0]+b[2])-x),Math.max(0,Math.min(a[1]+a[3],b[1]+b[3])-y)]; };
      // Row 489: a closed root's content can paint outside its host's box (Chromium's PDF body is 0 high, its viewer
      // iframe absolute in the root): the region covers that ink, as opaqueBoxes reports it.
      // The host's own box stays the origin (element offset, input); only the captured area grows to the root's ink.
      let bounds=box(node),clip=intersect(closedInk(node,bounds)??bounds,[0,0,node.ownerDocument.defaultView.innerWidth,node.ownerDocument.defaultView.innerHeight]);
      for(let view=node.ownerDocument.defaultView;view !== window;view=view.parent){
        const owner=view.frameElement;if(!owner)throw new Error('mirror2 frame unavailable');
        // The owner's origin maps the child by translation only: a rotated/scaled owner (or ancestor of it) would misplace the crop.
        if(moved(owner))throw new Error('mirror2 region frame transformed');
        const r=owner.getBoundingClientRect(),style=getComputedStyle(owner),x=r.x+owner.clientLeft+(parseFloat(style.paddingLeft)||0),y=r.y+owner.clientTop+(parseFloat(style.paddingTop)||0);
        bounds=[bounds[0]+x,bounds[1]+y,bounds[2],bounds[3]];clip=[clip[0]+x,clip[1]+y,clip[2],clip[3]];
        clip=intersect(clip,[0,0,view.parent.innerWidth,view.parent.innerHeight]);
      }
      if(!(clip[2]>0&&clip[3]>0))throw new Error('mirror2 region offscreen');
      const out={id,box:bounds,clip,distorted:distorted(node),media:node.namespaceURI===HTML&&['video','canvas','object','embed'].includes(node.localName)};
      if(foreign.has(id)){
        out.moved=moved(node);
        const style=getComputedStyle(node),left=node.clientLeft+(parseFloat(style.paddingLeft)||0),top=node.clientTop+(parseFloat(style.paddingTop)||0);
        const width=node.clientWidth-(parseFloat(style.paddingLeft)||0)-(parseFloat(style.paddingRight)||0),height=node.clientHeight-(parseFloat(style.paddingTop)||0)-(parseFloat(style.paddingBottom)||0);
        out.contentBox=[bounds[0]+left,bounds[1]+top,width,height];
      }
      return out;
    };
    const mediaNode = id => {
      const node=live(id);
      return kindOf.get(id)==='tile'&&node.namespaceURI===HTML&&['video','canvas','object','embed'].includes(node.localName)?node:null;
    };
    const regionBoxes = () => {
      const out=[];
      for(const [id] of nodes) if(kindOf.get(id)==='tile'||foreign.has(id)) {
        try{out.push(regionShape(id));}catch{} // disconnected/protected/fully clipped => no capability
      }
      return out;
    };
    // A viewport screenshot of a region already holds its compositing (any transform beyond a translation,
    // also size-preserving ones like rotate(180deg) or scaleX(-1); a filter; opacity or blending on it, an
    // ancestor or a same-origin frame owner), which the mirrored DOM applies again: such regions are flagged (the service then needs the
    // element's own pixels, else labelled video).
    const linear = transform => { if (transform === 'none') return false; const m = new DOMMatrixReadOnly(transform); return !(m.is2D && m.a === 1 && m.b === 0 && m.c === 0 && m.d === 1); };
    // A fully transparent or hidden element paints nothing: there is no pixel to capture, and the mirrored element
    // keeps its own opacity/visibility (DuckDuckGo result videos sit at opacity 0 until hovered).
    const invisible = node => {
      if (node.nodeType !== 1 || getComputedStyle(node).visibility !== 'visible') return true;
      for (let at = node, depth = 0; at && depth < DEPTH; depth++, at = at.parentElement ?? at.getRootNode?.()?.host ?? at.ownerDocument?.defaultView?.frameElement) if (at.nodeType === 1 && Number(getComputedStyle(at).opacity) === 0) return true;
      return false;
    };
    // The cause (a fixed token, never page text) or false: diagnostics and the service's settle window.
    const distorted = node => {
      const [, , width, height] = box(node), [layoutWidth, layoutHeight] = layoutSize(node);
      if (Math.abs(width - layoutWidth) > 1 || Math.abs(height - layoutHeight) > 1) return 'size';
      for (let at = node, depth = 0; at && depth < DEPTH; depth++, at = at.parentElement ?? at.getRootNode?.()?.host ?? at.ownerDocument?.defaultView?.frameElement) {
        if (at.nodeType !== 1) continue;
        const style = getComputedStyle(at), cause = style.filter !== 'none' ? 'filter' : linear(style.transform) ? 'transform' : style.rotate !== 'none' ? 'rotate' : style.scale !== 'none' ? 'scale' : Number(style.opacity) < 1 ? 'opacity' : style.mixBlendMode !== 'normal' ? 'blend' : '';
        if (cause) return cause + (at === node ? '' : '_ancestor');
      }
      // A script-disabled mirror cannot reliably paint video inside canvas fallback content.
      // Its region is already composited by Chrome and must use a root video patch.
      return node.namespaceURI === HTML && node.localName === 'canvas' ? 'canvas' : false;
    };
    // Whether the node or an ancestor (through frame owners) moves or resizes pixels, independent of the one cause distorted() names:
    // a crop of the frame inside maps into the page by translation only.
    const moved = node => {
      const [, , width, height] = box(node), [layoutWidth, layoutHeight] = layoutSize(node);
      if (Math.abs(width - layoutWidth) > 1 || Math.abs(height - layoutHeight) > 1) return true;
      for (let at = node, depth = 0; at && depth < DEPTH; depth++, at = at.parentElement ?? at.getRootNode?.()?.host ?? at.ownerDocument?.defaultView?.frameElement) {
        if (at.nodeType !== 1) continue;
        const style = getComputedStyle(at);
        if (linear(style.transform) || style.rotate !== 'none' || style.scale !== 'none') return true;
      }
      return false;
    };
    // An opaque region's own pixels (a canvas bitmap, a video's current frame) at the viewer's scale:
    // overlays and sticky headers above it stay mirrored DOM, never baked in. Null when tainted
    // (cross-origin pixels never leave this way), blank (WebGL without preserveDrawingBuffer), without a
    // frame yet, or another kind (plugins, closed roots, frames: masked screenshots).
    // The still keeps its CSS box: past the packet's byte budget it travels at a lower pixel density, the one its measured size
    // predicts (side x 0.9 x sqrt(budget / bytes), drawn again from the full-size pixels), again until it fits. Halving left a
    // video frame that missed the budget by 1% at a quarter of its density (subvysion, T3).
    const drawnCanvases = new WeakSet();
    const elementPixels = async (id, scale, budget) => {
      const node = nodes.get(id);
      if (!node?.isConnected || kindOf.get(id) !== 'tile' || node.namespaceURI !== HTML || !['canvas', 'video'].includes(node.localName) || node.localName === 'video' && node.readyState < 2) return null;
      const style = getComputedStyle(node), px = name => parseFloat(style[name]) || 0, video = node.localName === 'video';
      const x = node.clientLeft + px('paddingLeft'), y = node.clientTop + px('paddingTop'), width = node.clientWidth - px('paddingLeft') - px('paddingRight'), height = node.clientHeight - px('paddingTop') - px('paddingBottom');
      const sw = video ? node.videoWidth : node.width, sh = video ? node.videoHeight : node.height;
      if (!(width > 0 && height > 0 && sw > 0 && sh > 0)) return null;
      const W = Math.min(4096, Math.round(width * scale)), H = Math.min(4096, Math.round(height * scale)), canvas = new OffscreenCanvas(W, H), g = canvas.getContext('2d');
      const fit = style.objectFit, k = fit === 'fill' ? null : fit === 'cover' ? Math.max(W / sw, H / sh) : Math.min(W / sw, H / sh);
      const [dw, dh] = k ? [sw * k, sh * k] : [W, H];
      try {
        // T3: the frame is read once, at its source size when that is within COVER_PROBE_PIXELS, and the still is drawn from that
        // copy: `opaque` (every source pixel's alpha, as coveredBy proves it) is then the opacity of exactly the frame that goes
        // out, even while a video plays on. A larger source is drawn directly and proves nothing.
        let source = node, opaque = false;
        if (sw * sh <= COVER_PROBE_PIXELS) {
          const copy = new OffscreenCanvas(sw, sh), c = copy.getContext('2d'); c.drawImage(node, 0, 0);
          const data = c.getImageData(0, 0, sw, sh).data; let all = true;
          for (let i = 3; i < data.length && all; i += 4) all = data[i] === 255;
          source = copy; opaque = all;
        }
        g.drawImage(source, (W - dw) / 2, (H - dh) / 2, dw, dh);
        // WebGL's cleared buffer reads as all transparent, or all opaque black with alpha:false (three.js's default).
        // All black is the page's own pixels only on a 2D canvas (getContext('2d') returns its context, null for WebGL;
        // a canvas without a context read transparent above, so this creates none).
        if (!video) {
          const data = g.getImageData(0, 0, W, H).data; let clear = true, black = true;
          for (let i = 0; i < data.length && (clear || black); i += 4) { clear &&= !data[i + 3]; black &&= !data[i] && !data[i + 1] && !data[i + 2] && data[i + 3] === 255; }
          // Nothing drawn (or cleared to transparent): the page shows through, as natively. The kernel decides the still.
          // A composited WebGL buffer (preserveDrawingBuffer false) reads clear too, while the page still shows its frame:
          // a canvas that once read real pixels has a context, so asking for '2d' creates none and names a 2D one; any
          // other context keeps the still it has.
          // `flat`: proven a 2D canvas the page cleared (only asked of a canvas with a context): the kernel may then replace a real
          // still with a clear one; otherwise it keeps whatever real still it has (one from the rendered page included).
          if (clear) { if (drawnCanvases.has(node)) { let flat = false; try { flat = !!node.getContext('2d'); } catch {} return flat ? { clear: true, flat: true } : null; } return { clear: true }; }
          drawnCanvases.add(node);
          if (black) { let flat = false; try { flat = !!node.getContext('2d'); } catch {} if (!flat) return null; }
        }
        for (let still = canvas; ;) {
          const bytes = new Uint8Array(await (await still.convertToBlob({ type: 'image/png' })).arrayBuffer());
          const encoded = 4 * Math.ceil(bytes.length / 3);
          if (encoded <= budget) { let binary = ''; for (let i = 0; i < bytes.length; i += 32768) binary += String.fromCharCode(...bytes.subarray(i, i + 32768)); return { x, y, width, height, data_base64: btoa(binary), opaque }; }
          if (still.width === 1 && still.height === 1) return null;
          const ratio = 0.9 * Math.sqrt(Math.max(0, budget) / encoded);
          const smaller = new OffscreenCanvas(Math.max(1, Math.min(still.width - 1, Math.floor(still.width * ratio))), Math.max(1, Math.min(still.height - 1, Math.floor(still.height * ratio))));
          smaller.getContext('2d').drawImage(canvas, 0, 0, smaller.width, smaller.height); still = smaller;
        }
      } catch { return null; }
    };
    // T3 (subvysion): a region whose still would never show, because another region's opaque still covers all of it from above
    // (a page's hidden <video> under the canvas that paints its frames), names that cover; the kernel then spends the packet's
    // still budget on the cover alone. Only what is proven counts: the cover contains the box, is above it at its centre and four
    // inset corners, is fully opaque (a video frame or canvas bitmap filling its box, the canvas's readback opaque everywhere),
    // and it and its own wrappers draw without transparency (opacity, blending, alpha filters, clip, mask) or a rotation.
    // Anything else (unreadable pixels, a hole, a cover below) names no cover.
    // 4 MP: a 2560x1600 canvas (a full DPR2 viewport) is read whole; larger ones keep both stills.
    const COVER_PROBE_PIXELS = 4 * 1024 * 1024;
    const coveredBy = list => {
      const out = {};
      // Compositing that keeps every pixel's alpha: colour-only filter functions, no clip or mask, full opacity, normal blending.
      const alphaKept = style => style.opacity === '1' && style.mixBlendMode === 'normal' && style.clipPath === 'none' && (style.maskImage ?? style.webkitMaskImage ?? 'none') === 'none'
        && (style.filter === 'none' || /^(?:\s*(?:brightness|contrast|grayscale|hue-rotate|invert|saturate|sepia)\([^)]*\))+\s*$/.test(style.filter));
      // The cover's pixels fill its whole box: no padding or border around them, and a bitmap that object-fit stretches or
      // crops over the box (contain only when the bitmap has the box's own aspect ratio).
      const fillsBox = (node, style, w, h) => {
        if (['paddingLeft', 'paddingRight', 'paddingTop', 'paddingBottom', 'borderLeftWidth', 'borderRightWidth', 'borderTopWidth', 'borderBottomWidth'].some(name => parseFloat(style[name]) > 0)) return false;
        if (['fill', 'cover'].includes(style.objectFit)) return true;
        return style.objectFit === 'contain' && w > 0 && h > 0 && node.clientWidth > 0 && node.clientHeight > 0 && Math.abs(w / h - node.clientWidth / node.clientHeight) < 0.005 * (w / h);
      };
      // Every ancestor of the cover below the one it shares with the covered region composites it the same way: a half-transparent
      // or filtered wrapper of the cover alone lets the covered region show through.
      const composedOpaque = (cover, below) => {
        const up = e => e.parentElement ?? (e.parentNode instanceof ShadowRoot ? e.parentNode.host : null), shared = new Set();
        for (let at = below; at; at = up(at)) shared.add(at);
        for (let at = up(cover); at && !shared.has(at); at = up(at)) if (!alphaKept(getComputedStyle(at))) return false;
        return true;
      };
      const opaque = new Map(), isOpaque = (id, node) => {
        if (opaque.has(id)) return opaque.get(id);
        let yes = false;
        try {
          const style = getComputedStyle(node);
          // An axis-aligned transform (subvysion's canvas has matrix(1, 0, 0, 1, 0, 0)) paints exactly its bounding box.
          const m = /^matrix\(([^)]*)\)$/.exec(style.transform)?.[1].split(',').map(Number), aligned = style.transform === 'none' || m?.length === 6 && m[1] === 0 && m[2] === 0;
          if (alphaKept(style) && aligned && style.visibility === 'visible') {
            // A video can carry transparency (VP9 with alpha): its current frame's alpha is proven like a canvas bitmap's.
            if (node.localName === 'video') {
              if (node.readyState >= 2 && ['fill', 'cover'].includes(style.objectFit) && fillsBox(node, style, node.videoWidth, node.videoHeight)) {
                if (node.videoWidth * node.videoHeight > COVER_PROBE_PIXELS) throw Error('cover too large to prove');
                const probe = new OffscreenCanvas(node.videoWidth, node.videoHeight), g = probe.getContext('2d');
                g.drawImage(node, 0, 0); const data = g.getImageData(0, 0, node.videoWidth, node.videoHeight).data; let all = true;
                for (let i = 3; i < data.length && all; i += 4) all = data[i] === 255;
                yes = all;
              }
            }
            else if (node.localName === 'canvas' && node.width > 0 && node.height > 0 && fillsBox(node, style, node.width, node.height)) {
              // Every source pixel's alpha, never a resized probe (a narrow transparent strip can fall between its samples); a bitmap
              // too large to read whole within COVER_PROBE_PIXELS proves nothing and covers nothing.
              if (node.width * node.height > COVER_PROBE_PIXELS) throw Error('cover too large to prove');
              const w = node.width, h = node.height, probe = new OffscreenCanvas(w, h), g = probe.getContext('2d');
              g.drawImage(node, 0, 0); const data = g.getImageData(0, 0, w, h).data; let all = true, black = true;
              for (let i = 0; i < data.length && all; i += 4) { all = data[i + 3] === 255; black &&= !data[i] && !data[i + 1] && !data[i + 2]; }
              // An all-black readback of a WebGL canvas is its cleared buffer, not what it shows (as elementPixels).
              yes = all && !(black && !(() => { try { return !!node.getContext('2d'); } catch { return false; } })());
            }
          }
        } catch { yes = false; }
        opaque.set(id, yes); return yes;
      };
      // A region hit testing cannot see (pointer-events: none) is ordered by CSS painting order in one provable case: it paints at a
      // negative z-index directly in the root stacking context (no ancestor makes one), so anything whose root-level painter is
      // in flow or at z-index >= 0 paints above it (subvysion's fixed z-index -1 video under its z-index 4 canvas).
      const makesContext = e => {
        if (e === e.ownerDocument.documentElement) return true;
        const st = getComputedStyle(e), parent = e.parentElement && getComputedStyle(e.parentElement).display;
        return st.position === 'fixed' || st.position === 'sticky' || st.zIndex !== 'auto' && (st.position !== 'static' || /flex|grid/.test(parent ?? '')) || st.opacity !== '1' || st.transform !== 'none' || st.filter !== 'none' || st.backdropFilter !== 'none' || st.mixBlendMode !== 'normal' || st.isolation === 'isolate' || st.clipPath !== 'none' || st.maskImage !== 'none' || st.perspective !== 'none' || /paint|layout|strict|content/.test(st.contain) || /transform|opacity|filter|z-index|position/.test(st.willChange);
      };
      const rootPainter = e => { let out = e; for (let at = e.parentElement; at && at !== e.ownerDocument.documentElement; at = at.parentElement) if (makesContext(at)) out = at; return out; };
      const belowAll = node => { const st = getComputedStyle(node); return st.position !== 'static' && parseInt(st.zIndex, 10) < 0 && rootPainter(node) === node; };
      const notBelow = e => { const p = rootPainter(e), st = getComputedStyle(p); return st.position === 'static' && !makesContext(p) || st.zIndex === 'auto' || parseInt(st.zIndex, 10) >= 0; };
      for (const id of list) {
        const node = nodes.get(id);
        if (!node?.isConnected || kindOf.get(id) !== 'tile' || node.namespaceURI !== HTML) continue;
        const r = node.getBoundingClientRect(), doc = node.ownerDocument, view = doc.defaultView;
        if (!(r.width > 2 && r.height > 2)) continue;
        const left = Math.max(r.left + 1, 0), top = Math.max(r.top + 1, 0), right = Math.min(r.right - 1, view.innerWidth - 1), bottom = Math.min(r.bottom - 1, view.innerHeight - 1);
        if (right <= left || bottom <= top) continue;
        let cover = null;
        for (const [x, y] of [[(left + right) / 2, (top + bottom) / 2], [left, top], [right, top], [left, bottom], [right, bottom]]) {
          const stack = doc.elementsFromPoint(x, y), at = stack.indexOf(node), hidden = at < 0 && belowAll(node);
          const above = at > 0 || hidden ? stack.slice(0, hidden ? stack.length : at).find(e => { if (hidden && !notBelow(e)) return false; const eid = mirrored(e); if (!eid || eid === id || kindOf.get(eid) !== 'tile' || e.namespaceURI !== HTML) return false; const b = e.getBoundingClientRect(); return b.left <= r.left + 0.5 && b.top <= r.top + 0.5 && b.right >= r.right - 0.5 && b.bottom >= r.bottom - 0.5 && isOpaque(eid, e); }) : null;
          if (!above || cover && above !== cover) { cover = null; break; }
          cover = above;
        }
        if (cover && composedOpaque(cover, node)) out[id] = mirrored(cover);
      }
      return out;
    };
    // MP-08 / MP-11: input follows live geometry; Vault protection applies to model observations.
    const live = id => {
      const node = nodes.get(id);
      if (!node?.isConnected || node.nodeType !== 1 && node.nodeType !== 3) throw new Error('mirror2 stale node');
      return node;
    };
    // One frame element's content box in its parent's viewport: origin and scale (a frame scaled by CSS maps its points
    // by that scale). A rotated or skewed frame has no such map; its clicks are refused.
    const frameStep = owner => {
      const b = owner.getBoundingClientRect(), style = getComputedStyle(owner), m = style.transform === 'none' ? null : new DOMMatrix(style.transform);
      if (m && (Math.abs(m.b) > 1e-6 || Math.abs(m.c) > 1e-6)) throw new Error('mirror2 transformed frame');
      const sx = owner.offsetWidth > 0 ? b.width / owner.offsetWidth : 1, sy = owner.offsetHeight > 0 ? b.height / owner.offsetHeight : 1;
      return { x: b.x + (owner.clientLeft + (parseFloat(style.paddingLeft) || 0)) * sx, y: b.y + (owner.clientTop + (parseFloat(style.paddingTop) || 0)) * sy, sx, sy };
    };
    // The map from a node's own viewport to this (top) viewport: top = t + s * own, composed through every frame.
    const frameMap = node => {
      let tx = 0, ty = 0, sx = 1, sy = 1;
      for (let view = node.ownerDocument.defaultView; view !== window; view = view.parent) {
        const owner = view.frameElement; if (!owner) throw new Error('mirror2 frame unavailable');
        const step = frameStep(owner);
        tx = step.x + step.sx * tx; ty = step.y + step.sy * ty; sx *= step.sx; sy *= step.sy;
      }
      return { tx, ty, sx, sy };
    };
    // A point (in the node's own viewport) outside what its scrolling ancestors and every enclosing frame show is not
    // reachable as it stands: the page scrolls it into view first (reveal), as for a viewer that scrolled ahead.
    // Clip boxes are taken in viewport coordinates, so a CSS-scaled pane clips by its scaled padding box.
    // With `nearest` the point moves to the closest visible one instead (the step stays inside the same panes and frames).
    // Returns the point in the top viewport, or null when it is clipped.
    const clip = (node, x, y, nearest = false) => {
      const into = (value, low, high) => value >= low && value < high ? value : nearest ? Math.min(Math.max(value, low), high - 1) : null;
      for (let view = node.ownerDocument.defaultView; ; view = view.parent) {
        x = into(x, 0, view.innerWidth); y = into(y, 0, view.innerHeight);
        for (let el = node.parentElement ?? node.getRootNode()?.host; x !== null && y !== null && el && el !== el.ownerDocument.documentElement && el !== el.ownerDocument.body; el = el.parentElement ?? el.getRootNode()?.host) {
          const style = getComputedStyle(el);
          if (style.position === 'fixed') break;
          if (style.overflowX === 'visible' && style.overflowY === 'visible') continue;
          const r = el.getBoundingClientRect(), sx = el.offsetWidth > 0 ? r.width / el.offsetWidth : 1, sy = el.offsetHeight > 0 ? r.height / el.offsetHeight : 1;
          const left = r.x + el.clientLeft * sx, top = r.y + el.clientTop * sy;
          x = into(x, left, left + el.clientWidth * sx); y = into(y, top, top + el.clientHeight * sy);
        }
        if (x === null || y === null) return null;
        if (view === window) return { x, y };
        node = view.frameElement; if (!node) throw new Error('mirror2 frame unavailable');
        const step = frameStep(node); x = step.x + step.sx * x; y = step.y + step.sy * y;
      }
    };
    // A point inside the live element at the viewer's offset (clamped). A node that lost its box since the
    // viewer's frame is clicked at the centre of its nearest boxed ancestor. Like Chrome, the click goes to whatever
    // the native page has at that point (an overlay included); only a protected target is refused.
    const point = request => {
      let node = live(request.node_id);
      if (node.nodeType !== 1) throw new Error('mirror2 element required');
      let [bx, by, width, height] = hostBox(node), moved = false;
      // A closed root's content can paint outside its host's box: the viewer's offset stays relative to the host.
      const ink = closedInk(node, [bx, by, width, height]), [ix, iy] = ink ? [ink[0] - bx, ink[1] - by] : [0, 0];
      if (ink) [bx, by, width, height] = ink;
      for (let up = 0; !(width > 0 && height > 0) && node.parentElement && up < 32; up++) { node = node.parentElement; [bx, by, width, height] = box(node); moved = true; }
      if (!(width > 0 && height > 0)) throw new Error('mirror2 invisible node');
      const ox = !moved && Number.isFinite(request.x) ? Math.min(Math.max(request.x - ix, 0.5), width - 0.5) : width / 2;
      const oy = !moved && Number.isFinite(request.y) ? Math.min(Math.max(request.y - iy, 0.5), height - 0.5) : height / 2;
      let x = bx + ox, y = by + oy;
      // A wheel step aimed at a node the page has scrolled past goes to the nearest visible point of the same panes and
      // frames: the viewer's earlier steps moved the node, and showing it again would take them back.
      const at = clip(node, x, y, request.wheel === true);
      if (!at) throw new Error('mirror2 offscreen node');
      return { x: Math.floor(at.x), y: Math.floor(at.y) };
    };
    // The viewer is scrolled ahead of the native page: bring the node into the native viewport and say where the page now is.
    const reveal = request => {
      const node = live(request.node_id); if (node.nodeType !== 1) throw new Error('mirror2 element required');
      node.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
      return [scrollX, scrollY];
    };
    // Re-validates at dispatch time: nothing protected is at the point now.
    const hitCheck = (pt, id) => {
      live(id);
      if (![pt.x, pt.y].every(Number.isFinite) || pt.x < 0 || pt.y < 0 || pt.x >= innerWidth || pt.y >= innerHeight) throw new Error('mirror2 offscreen node');
      return true;
    };
    // B4: Chrome's own context menu would open in the kernel's window: the right click's contextmenu is cancelled in the
    // window of the document it fires in (a frame's contextmenu never reaches its parent), after the page's listeners.
    const menuOff = id => {
      const view = live(id).ownerDocument?.defaultView; if (!view) return false;
      const off = event => event.preventDefault();
      view.addEventListener('contextmenu', off, { once: true }); setTimeout(() => view.removeEventListener('contextmenu', off), 1000);
      return true;
    };
    const activeTarget = (editable = true) => {
      let node = document.activeElement;
      for (let depth = 0; depth < 128; depth++) { let nested = node?.shadowRoot?.activeElement; try { nested ??= node?.localName === 'iframe' ? node.contentDocument?.activeElement : null; } catch {} if (!nested) break; node = nested; }
      const id = node && mirrored(node);
      if (!id || !node.isConnected || editable && !node.isContentEditable && !['input', 'textarea'].includes(node.localName)) throw new Error('mirror2 unavailable native text focus');
      return ancestry(id);
    };
    // The target and every mirrored ancestor, leaf first (the kernel checks each against the viewer's epoch).
    const ancestry = id => { const out = []; for (let at = id; at && out.length < 4096; at = parentOf.get(at)) out.push(at); return out; };
    const focus = request => { const node = live(request.node_id); if (node.nodeType !== 1) throw new Error('mirror2 element required'); node.focus({ preventScroll: true }); let active = node.ownerDocument.activeElement; while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement; if (active !== node) throw new Error('mirror2 focus redirected'); return true; };
    // Protocol 492: the viewer's focus left this control; only while it is still the live focus.
    const blur = request => { const node = live(request.node_id); let active = node.ownerDocument.activeElement; while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement; if (active !== node) return false; node.blur(); return true; };
    // Protocol 492: control state committed by the viewer's own UI (select pick, date/color/range
    // picker, a multiple select's option). Written here; the page sees input + change.
    const PICKED = new Set('date time datetime-local month week color range'.split(' '));
    const set = request => {
      const node = live(request.node_id), keys = ['value', 'checked', 'selected_index'].filter(key => request[key] !== undefined).join();
      const control = node.localName === 'option' ? node.closest('select') : node;
      if (node.nodeType !== 1 || !control || control.matches(':disabled') || node.matches?.(':disabled') || control.readOnly) throw new Error('mirror2 control unavailable');
      if (node.localName === 'select' && !node.multiple && keys === 'selected_index' && Number.isInteger(request.selected_index) && request.selected_index >= -1 && request.selected_index < node.options.length) node.selectedIndex = request.selected_index;
      else if (node.localName === 'option' && control.localName === 'select' && control.multiple && keys === 'checked' && typeof request.checked === 'boolean') node.selected = request.checked;
      else if (node.localName === 'input' && PICKED.has(node.type) && keys === 'value' && typeof request.value === 'string' && request.value.length <= 256) node.value = request.value;
      else throw new Error('mirror2 control state refused');
      control.dispatchEvent(new Event('input', { bubbles: true, composed: true })); control.dispatchEvent(new Event('change', { bubbles: true }));
      return true;
    };
    const select = request => {
      const a = live(request.anchor_id), b = live(request.focus_id);
      const offsets = [request.anchor_offset, request.focus_offset];
      // A text control's own range (select-all, a drag inside it): offsets into its value.
      if (a === b && a.nodeType === 1 && ['input', 'textarea'].includes(a.localName) && a.selectionStart !== null) {
        if (!offsets.every(offset => Number.isInteger(offset) && offset >= 0 && offset <= a.value.length)) throw new Error('mirror2 invalid selection');
        // It is the viewer's focused control (a drag focuses one without a click): the next text and keys edit it here too.
        focus({ node_id: request.anchor_id });
        a.setSelectionRange(Math.min(...offsets), Math.max(...offsets), offsets[0] > offsets[1] ? 'backward' : 'forward'); return true;
      }
      if (a.nodeType !== 3 || b.nodeType !== 3 || a.ownerDocument !== b.ownerDocument || !Number.isInteger(request.anchor_offset) || !Number.isInteger(request.focus_offset) || request.anchor_offset < 0 || request.anchor_offset > a.length || request.focus_offset < 0 || request.focus_offset > b.length) throw new Error('mirror2 invalid selection');
      const range = a.ownerDocument.createRange(), pa = a.ownerDocument.createRange(), pb = a.ownerDocument.createRange(); pa.setStart(a, request.anchor_offset); pa.collapse(true); pb.setStart(b, request.focus_offset); pb.collapse(true);
      if (pa.compareBoundaryPoints(Range.START_TO_START, pb) > 0) { range.setStart(b, request.focus_offset); range.setEnd(a, request.anchor_offset); } else { range.setStart(a, request.anchor_offset); range.setEnd(b, request.focus_offset); }
      for (const [id, node] of nodes) if (node.ownerDocument === a.ownerDocument && kindOf.get(id) === 'mask' && range.intersectsNode(node)) throw new Error('mirror2 selection intersects protected content');
      a.ownerDocument.getSelection().setBaseAndExtent(a, request.anchor_offset, b, request.focus_offset); return true;
    };
    // MP-08/MP-10 local scroll (viewer-owned): the kernel follows the viewer. Y3-R1 (6thsense, tensr): at once, whatever
    // scroll-behavior the page asks for. The viewer already moved its copy the page's way; a smooth kernel scroll lagged
    // behind it and its start position came back to the viewer as where the page stood.
    const at = (x, y) => ({ left: x, top: y, behavior: 'instant' });
    const scrollTo = request => {
      if (!request.node_id) { window.scrollTo(at(request.x, request.y)); return [scrollX, scrollY]; }
      const node = nodes.get(request.node_id);
      // A document id scrolls that document's window (same-origin frames too).
      if (node?.nodeType === 9 && node.defaultView) { node.defaultView.scrollTo(at(request.x, request.y)); return [node.defaultView.scrollX, node.defaultView.scrollY]; }
      const element = live(request.node_id); if (element.nodeType !== 1) throw new Error('mirror2 element required');
      element.scrollTo(at(request.x, request.y)); return [element.scrollLeft, element.scrollTop];
    };
    // Trusted-side (DOM.resolveNode): the mirror id of a frame owner element.
    const idOfNode = function () { return this && mirrored(this) || null; };
    // Content-box origin of a cross-origin frame element in this viewport.
    // With the point the viewer clicked in that frame's own viewport, a frame clipped away at it is reported offscreen
    // (a wheel step moves to the nearest visible point instead); the point in this viewport follows the origin and scale.
    const frameOrigin = (id, local, wheel = false) => {
      const node = live(id); if (node.localName !== 'iframe' || !foreign.has(id)) throw new Error('mirror2 frame required');
      const up = frameMap(node), own = frameStep(node), origin = [up.tx + up.sx * own.x, up.ty + up.sy * own.y, up.sx * own.sx, up.sy * own.sy];
      if (!local) return origin;
      const at = clip(node, own.x + own.sx * local[0], own.y + own.sy * local[1], wheel);
      if (!at) throw new Error('mirror2 offscreen node');
      return [...origin, at.x, at.y];
    };
    // Live focus inside a cross-origin frame: the frame element's mirror id.
    const activeForeign = () => {
      let node = document.activeElement;
      for (let depth = 0; depth < 128; depth++) { let nested = node?.shadowRoot?.activeElement; try { nested ??= node?.localName === 'iframe' ? node.contentDocument?.activeElement : null; } catch {} if (!nested) break; node = nested; }
      const id = node && mirrored(node); return id && foreign.has(id) ? id : null;
    };
    // CDP describes only these already serialized hosts. Unpublished records
    // can become opaque under the same id before their packet leaves the kernel.
    const markClosedHost = function (fresh = false, root = null) {
      if (!this || this.nodeType !== 1) return true;
      closedHosts.add(this); closedTodo.delete(this); if (root) closedRoots.set(this, root);
      const id = mirrored(this);
      if (!fresh || !id) { replaceNode(this); return true; }
      const removed = [], collect = at => { removed.push(at); for (const child of kids.get(at) ?? []) collect(child); };
      for (const child of kids.get(id) ?? []) collect(child);
      for (const child of [...kids.get(id) ?? []]) forget(child);
      kids.set(id, []); pendingHosts.delete(id); stateNodes.delete(id); sentStates.delete(id);
      const record = { id, parent: parentOf.get(id), kind: 'element' }; opaqueRecord(this, record, 'closed_shadow');
      kindOf.set(id, 'tile'); opaque.add(id);
      return { record, removed };
    };
    const customCandidates = () => {
      const hosts = [];
      for (const node of closedTodo) {
        if (!node.isConnected || node.shadowRoot || closedHosts.has(node)) { closedTodo.delete(node); continue; }
        hosts.push(node); if (hosts.length === 256) break;
      }
      return hosts;
    };
    const customHosts = () => closedTodo.size;
    const closedChecked = function () { lastClosedCheck = performance.now(); if (this?.nodeType === 1) { checkedClosed.set(this, this.matches(':defined')); closedTodo.delete(this); } return true; };
    // Trusted-side: resolveNode() hands Vault target nodes to this world, with the fill's value hash and revision.
    const protect = function (hash = null, fillRevision = null) {
      if (!this || this.nodeType !== 1) return true;
      const token = `${hash}:${fillRevision}`;
      if (fills.get(this)?.token !== token) fills.set(this, { token, hash: typeof hash === 'string' && /^[0-9a-f]{64}$/.test(hash) ? hash : null, state: 'pending' });
      targets.add(this); verify(this); replaceNode(this); return true;
    };
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
    // Every character the document can show now, for the font subsetter: text nodes (shadow roots too), the values
    // and placeholders of fields, and the text pseudo-elements generate (an icon font's glyphs). Bounded: past the
    // budget the rest of the text is not read.
    const textCodePoints = () => {
      const seen = new Set(); let budget = 2_000_000;
      const take = text => { if (typeof text !== 'string') return; budget -= text.length; for (const char of text) seen.add(char.codePointAt(0)); };
      const visit = (node, depth) => {
        if (depth > DEPTH || budget <= 0) return;
        if (node.nodeType === 3) { take(node.data); return; }
        if (node.nodeType === 1) {
          if (['script', 'style', 'noscript', 'template'].includes(node.localName)) return;
          if (node.localName === 'input' || node.localName === 'textarea') { take(node.value); take(node.placeholder); }
        }
        for (const child of node.childNodes) visit(child, depth + 1);
        if (node.shadowRoot) visit(node.shadowRoot, depth + 1);
      };
      visit(document, 0);
      const all = document.getElementsByTagName('*'), deadline = performance.now() + 40;
      for (let at = 0; at < all.length && at < 6000; at++) {
        if ((at & 63) === 63 && performance.now() > deadline) break;
        for (const pseudo of ['::before', '::after']) { const content = getComputedStyle(all[at], pseudo).content; if (content !== 'none' && content !== 'normal') take(content); }
      }
      return [...seen];
    };
    const sheetChanged = owner => {
      if (owner === 'adopted') adoptedDirty = true;
      else if (owner === 'imports') { for (const entry of styleNodes.values()) if (entry.imports) dirty.sheets.add(entry.node); }
      else { const id = owner && mirrored(owner); if (id && styleNodes.has(id)) dirty.sheets.add(owner); }
      wake(); return true;
    };
    // Credit long-poll: resolves on the next page change (or the bound).
    const changed = () => fontsChanged() || stacksDue() || complete() !== reportedComplete || JSON.stringify(fontFailures()) !== reportedFonts || continuation !== null || deferred.size > 0 || records.length > 0 || overflow || adoptedDirty || valueStale.length > 0 || Object.values(dirty).some(set => set.size > 0);
    // Credit long-poll. The delta is taken in the wake microtask, right after our
    // capture-phase listener or MutationObserver callback, so an echo does not
    // wait for the page's own handlers to finish their task. A change flagged
    // between drains (input event, scroll, load) resolves immediately.
    const waitDrain = ms => new Promise((resolve, reject) => {
      let settled = false;
      const finish = () => { if (settled) return; settled = true; try { resolve(drain()); } catch (error) { reject(error); } };
      if (changed()) return finish();
      waiters.push(finish); setTimeout(finish, Math.min(Math.max(ms, 1), 2000));
    });
    // A sheet the kernel read is shared by URL across this world's documents: its viewer options load in this world's document.
    const sanitize = (text, base) => sheetCss(String(text), String(base), document);
    // A cross-origin sheet the kernel read (already sanitized): its rules in use now, as above.
    const usedCss = (id, text) => {
      const root = nodes.get(id)?.getRootNode?.();
      if (window !== window.top || document.readyState === 'loading' || root?.nodeType !== 9 || viewerRetains(String(text))) return null;
      const sheet = new CSSStyleSheet(); sheet.replaceSync(String(text));
      const keep = usedBy(root); let out = ''; for (const rule of sheet.cssRules) out += pruned(rule, keep);
      return out;
    };
    // The same check from a cross-origin sheet's selector list (the kernel holds the sheet and keeps the rules): '1' where
    // a selector is in use, '0' where not, as usedCss would keep or drop its rule. Null where usedCss answers null.
    const usedSelectors = (id, selectors) => {
      const root = nodes.get(id)?.getRootNode?.();
      if (window !== window.top || document.readyState === 'loading' || root?.nodeType !== 9) return null;
      const keep = usedBy(root); let out = ''; for (const selector of selectors) out += keep(String(selector)) ? '1' : '0';
      return out;
    };
    // URLs this document actually fetched (Resource Timing): the kernel reads
    // only those bytes, never a URL that a stylesheet merely mentions.
    const resourceDocuments = () => new Set([document, ...[...roots.values()].filter(root => root.nodeType === 9)]);
    const resourceEntries = () => [...resourceDocuments()].flatMap(doc => { try { return doc.defaultView?.performance.getEntriesByType('resource') ?? []; } catch { return []; } });
    const resourceImages = () => imageNodes.values();
    // W0-T1: a file loads once whatever fragments name it (an SVG stack's icons): each fragment's key counts as loaded with it.
    const unhashed = url => { const at = url.indexOf('#'); return at < 0 ? url : url.slice(0, at); };
    const loaded = () => { const names = new Set([...timed, ...resourceEntries().map(entry => entry.name)]); for (const img of resourceImages()) if (img.complete && img.naturalWidth && img.currentSrc) { names.add(img.currentSrc); try { names.add(new URL(imageSource(img), img.baseURI).href); } catch {} }
      const files = new Set([...names].map(unhashed)), keys = [];
      for (const [slot, entry] of urlKeys) if (files.has(unhashed(slot.startsWith('sprite:') ? slot.slice(7) : slot))) keys.push(entry.key);
      return keys; };
    // Changes when the page loads something (an image may complete without a new entry).
    const loadedCount = () => { let complete = 0; for (const img of resourceImages()) if (img.complete) complete++; return timed.size + resourceEntries().length + complete; };
    // MP-10 wheel bytes: images (by element) near the viewport, with their CSS
    // size; element images elsewhere wait until the view comes near them.
    const nearImages = margin => {
      const near = [], all = [];
      for (const [id, key] of resOf) {
        const node = nodes.get(id); if (!node?.isConnected) continue;
        all.push(key);
        const r = node.getBoundingClientRect(), view = node.ownerDocument.defaultView ?? window;
        if (r.width > 0 && r.height > 0 && r.bottom > -margin && r.top < view.innerHeight + margin && r.right > 0 && r.left < view.innerWidth) near.push([key, Math.ceil(r.width), Math.ceil(r.height)]);
      }
      return { near, all };
    };
    // Protocol 501: an external sprite file as the viewer may use it: its SVG shapes, paint servers and styles
    // rebuilt element by element (no scripts, foreign content, images, links or animation; url() only to
    // fragments; href only to fragments), parsed inert. Null when it is no SVG or over the bounds.
    const SPRITE_TAGS = new Set([...SVG_TAGS].filter(tag => !['image', 'a'].includes(tag)));
    const sprite = text => {
      const source = new DOMParser().parseFromString(String(text), 'image/svg+xml').documentElement;
      if (source?.namespaceURI !== SVG || source.localName !== 'svg') return null;
      const doc = document.implementation.createDocument(SVG, 'svg', null); let count = 0;
      const copy = (from, to, depth) => {
        for (const child of from.childNodes) {
          if (++count > 20000 || depth > 64) throw new Error('mirror2:sprite_budget');
          if (child.nodeType === 3) { to.appendChild(doc.createTextNode(to.localName === 'style' ? sanitizeMirrorCss(child.data, location.href, () => null) : child.data)); continue; }
          if (child.nodeType !== 1 || child.namespaceURI !== SVG || !SPRITE_TAGS.has(child.localName)) continue;
          const element = doc.createElementNS(SVG, child.localName);
          for (const { name, value } of child.attributes) {
            const lower = name.toLowerCase();
            if (!/^[a-zA-Z_][-a-zA-Z0-9_.]*$/.test(name) && lower !== 'xlink:href' || value.length > 65536 || lower.startsWith('on') || /^\s*javascript:/i.test(value)) continue;
            if (lower === 'href' || lower === 'xlink:href') { if (/^#[\w-]+$/.test(value)) element.setAttribute('href', value); continue; }
            if (lower === 'style') { element.setAttribute('style', sanitizeMirrorCss(value, location.href, () => null)); continue; }
            if (DENY_ATTR.has(lower) || /url\s*\(/i.test(value) && !/^\s*url\(\s*["']?#[\w-]+["']?\s*\)\s*$/.test(value)) continue;
            element.setAttribute(name, value);
          }
          to.appendChild(element); copy(child, element, depth + 1);
        }
      };
      copy(source, doc.documentElement, 0);
      const out = new XMLSerializer().serializeToString(doc);
      return out.length <= 4 * 1024 * 1024 ? out : null;
    };
    // A low-detail stand-in for page image bytes while the view moves: decoded
    // here (the bytes the page loaded), downscaled to half its CSS size, WebP; the
    // SVG keeps the original pixel size so the mirrored layout does not change.
    const preview = async (data, type, width, height) => {
      const bitmap = await createImageBitmap(new Blob([Uint8Array.from(atob(data), c => c.charCodeAt(0))], { type }));
      const W = bitmap.width, H = bitmap.height, scale = Math.min(1, Math.max(8, width / 2) / W, Math.max(8, height / 2) / H);
      const canvas = new OffscreenCanvas(Math.max(1, Math.round(W * scale)), Math.max(1, Math.round(H * scale)));
      canvas.getContext('2d').drawImage(bitmap, 0, 0, canvas.width, canvas.height); bitmap.close();
      const bytes = new Uint8Array(await (await canvas.convertToBlob({ type: 'image/webp', quality: 0.5 })).arrayBuffer());
      let binary = ''; for (let i = 0; i < bytes.length; i += 32768) binary += String.fromCharCode(...bytes.subarray(i, i + 32768));
      return `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none"><image width="${W}" height="${H}" preserveAspectRatio="none" href="data:image/webp;base64,${btoa(binary)}"/></svg>`;
    };
    return Object.freeze({ sheetChanged, dispose, snapshot, visited, drain, resetTargets, waitDrain, wake, sanitize, sheetKeys, usedCss, usedSelectors, fullSheet, markClosedHost, reveal, customHosts, customCandidates, closedChecked, loaded, loadedCount, nearImages, preview, sprite, idOfNode, frameOrigin, activeForeign, opaqueBoxes, regionShape, regionBoxes, mediaNode, elementPixels, coveredBy, point, hitCheck, menuOff, activeTarget, ancestry, focus, blur, set, select, scrollTo, protect, textCoverage, textCodePoints, pending: () => continuation !== null || deferred.size > 0 || records.length > 0 || overflow });
  }
}
