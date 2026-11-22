import { installScriptFontRecorder } from './browser-page-font-recorder.mjs';

// browserfidelity r19 F5: the faces a page made by script, per page target, as @font-face rules the mirror adds to its
// document as one more adopted sheet (kernel-browser-mirror2.mjs), so their bytes travel the font path of any sheet.
export const scriptFontsBinding = '__charioxScriptFonts';
const MAX_FACES = 256, MAX_CSS = 24 * 1024 * 1024, MAX_SRC = 4 * 1024 * 1024; // MAX_SRC: a data: URL of a 3 MB font (the bytes limit)

const cssString = value => `"${value.replace(/[\\"\n\r\f]/g, c => `\\${c.charCodeAt(0).toString(16)} `)}"`;
// A descriptor value is page text placed inside a rule: anything that could end the rule or open a comment is refused.
const plain = value => typeof value === 'string' && value.length > 0 && value.length <= 65536 && !/[{};<\\]|\/\*/.test(value);
// A src value is a list of url(...) or local(...) items, each with optional format(...) and tech(...) hints (#1617 review: a
// data: URL holds ';' and ','). Each item is written back from its parts, its URL made absolute and every URL and name quoted by
// cssString, so nothing in them can end the rule. A value outside that grammar gives no face, as Chrome gives it no font.
function srcCss(src, base) {
  if (typeof src !== 'string' || src.length === 0 || src.length > MAX_SRC || typeof base !== 'string') return null;
  const items = [];
  let depth = 0, quote = '', start = 0;
  for (let i = 0; i <= src.length; i++) {
    const c = src[i];
    if (quote) { if (c === quote) quote = ''; continue; }
    if (c === '"' || c === "'") quote = c;
    else if (c === '(') depth++;
    else if (c === ')') depth--;
    else if ((c === ',' && depth === 0) || i === src.length) { const item = srcItem(src.slice(start, i), base); if (!item) return null; items.push(item); start = i + 1; }
    if (depth < 0) return null;
  }
  return quote || depth ? null : items.join(',');
}
const sourcePart = /^\s*(url|local)\(\s*(?:"([^"\\]*)"|'([^'\\]*)'|([^'"()\\]*?))\s*\)/i, hintPart = /^\s*(format|tech)\(([^()]*)\)/i;
function srcItem(text, base) {
  const source = sourcePart.exec(text);
  if (!source) return null;
  const value = source[2] ?? source[3] ?? source[4], kind = source[1].toLowerCase();
  if (kind === 'url' && /\s/.test(source[4] ?? '')) return null;
  let css;
  if (kind === 'local') css = `local(${cssString(value)})`;
  else { try { css = `url(${cssString(new URL(value, base).href)})`; } catch { return null; } }
  let rest = text.slice(source[0].length);
  for (let hint; (hint = hintPart.exec(rest)); rest = rest.slice(hint[0].length)) {
    const names = hint[2].split(',').map(name => name.trim().replace(/^(['"])([\w.-]*)\1$/, '$2'));
    if (!names.every(name => /^[\w.-]+$/.test(name))) return null;
    css += hint[1].toLowerCase() === 'format' ? ` format(${names.map(cssString).join(',')})` : ` tech(${names.join(',')})`;
  }
  return rest.trim() === '' ? css : null;
}
const descriptors = [['style', 'font-style'], ['weight', 'font-weight'], ['stretch', 'font-stretch'], ['unicodeRange', 'unicode-range'], ['featureSettings', 'font-feature-settings'], ['display', 'font-display'],
  // #1617 review 2: the metric overrides set the face's baseline and line height.
  ['ascentOverride', 'ascent-override'], ['descentOverride', 'descent-override'], ['lineGapOverride', 'line-gap-override'], ['sizeAdjust', 'size-adjust']];

/** @font-face rules for the faces the page world reported (browser-page-font-recorder.mjs); a face it cannot place is left out. */
export function scriptFontsCss(faces) {
  const rules = [];
  for (const face of faces.slice(0, MAX_FACES)) {
    if (!face || typeof face !== 'object') continue;
    const family = typeof face.family === 'string' ? face.family.trim().replace(/^(['"])(.*)\1$/, '$2') : '';
    if (!family || family.length > 256 || /[\n\r\f]/.test(family)) continue;
    let src;
    if (typeof face.data === 'string') { if (!/^[A-Za-z0-9+/]+=*$/.test(face.data)) continue; src = `url("data:application/octet-stream;base64,${face.data}")`; }
    else if (!(src = srcCss(face.src, face.base))) continue;
    const parts = [`font-family:${cssString(family)}`, `src:${src}`];
    for (const [key, property] of descriptors) if (plain(face[key])) parts.push(`${property}:${face[key]}`);
    rules.push(`@font-face{${parts.join(';')}}`);
  }
  const css = rules.join('\n');
  return css.length <= MAX_CSS ? css : '';
}

// Owned by the kernel browser's CDP client, beside BrowserPagePermissions. Only the main frame's page world reports: a page
// target's main frame id is its target id. A new document starts empty; `serial` grows with every change of any target.
export class BrowserPageFonts {
  constructor(owner) { this.owner = owner; this.contexts = new Map(); this.byTarget = new Map(); this.serial = 0; }
  async install(connection, sessionId) {
    await connection.send('Runtime.addBinding', { name: scriptFontsBinding }, sessionId);
    await connection.send('Page.addScriptToEvaluateOnNewDocument', { source: `(${installScriptFontRecorder})(${JSON.stringify(scriptFontsBinding)})`, runImmediately: true }, sessionId);
  }
  observe(message) {
    const { method, params = {}, sessionId } = message ?? {};
    if (method === 'Runtime.executionContextCreated') {
      const context = params.context, target = this.owner.targetsBySession?.get(sessionId);
      if (target && context?.auxData?.isDefault && context.auxData.frameId === target) this.contexts.set(`${sessionId}:${context.id}`, target);
    } else if (method === 'Runtime.executionContextDestroyed') this.dropContext(`${sessionId}:${params.executionContextId}`);
    else if (method === 'Runtime.executionContextsCleared') { for (const key of [...this.contexts.keys()]) if (key.startsWith(`${sessionId}:`)) this.dropContext(key); }
    else if (method === 'Runtime.bindingCalled' && params.name === scriptFontsBinding) {
      const key = `${sessionId}:${params.executionContextId}`, target = this.contexts.get(key);
      if (!target || typeof params.payload !== 'string' || params.payload.length > MAX_CSS) return;
      let faces; try { faces = JSON.parse(params.payload); } catch { return; }
      if (!Array.isArray(faces)) return;
      const css = scriptFontsCss(faces), current = this.byTarget.get(target);
      if (current?.context !== key || current.css !== css) this.byTarget.set(target, { context: key, serial: ++this.serial, css });
    }
  }
  dropContext(key) {
    const target = this.contexts.get(key); this.contexts.delete(key);
    if (target && this.byTarget.get(target)?.context === key) this.byTarget.set(target, { context: null, serial: ++this.serial, css: '' });
  }
  dropTarget(target) { this.byTarget.delete(target); for (const [key, owner] of this.contexts) if (owner === target) this.contexts.delete(key); }
  /** The target's current script-made faces as CSS and the serial of that state, or null when its page made none. */
  of(target) { return this.byTarget.get(target) ?? null; }
  clear() { this.contexts.clear(); this.byTarget.clear(); }
}
