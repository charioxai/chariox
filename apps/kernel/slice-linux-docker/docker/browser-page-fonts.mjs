import { installScriptFontRecorder } from './browser-page-font-recorder.mjs';

// browserfidelity r19 F5: the faces a page made by script, per page target, as @font-face rules the mirror adds to its
// document as one more adopted sheet (kernel-browser-mirror2.mjs), so their bytes travel the font path of any sheet.
export const scriptFontsBinding = '__charioxScriptFonts';
const MAX_FACES = 256, MAX_CSS = 24 * 1024 * 1024;

const cssString = value => `"${value.replace(/[\\"\n\r\f]/g, c => `\\${c.charCodeAt(0).toString(16)} `)}"`;
// A descriptor or src value is page text placed inside a rule: anything that could end the rule or open a comment is refused.
const plain = value => typeof value === 'string' && value.length > 0 && value.length <= 65536 && !/[{};<\\]|\/\*/.test(value);
const absolute = (src, base) => src.replace(/url\(\s*(['"]?)([^'")]*)\1\s*\)/gi, (whole, quote, url) => { try { return `url(${cssString(new URL(url, base).href)})`; } catch { return whole; } });
const descriptors = [['style', 'font-style'], ['weight', 'font-weight'], ['stretch', 'font-stretch'], ['unicodeRange', 'unicode-range'], ['featureSettings', 'font-feature-settings'], ['display', 'font-display']];

/** @font-face rules for the faces the page world reported (browser-page-font-recorder.mjs); a face it cannot place is left out. */
export function scriptFontsCss(faces) {
  const rules = [];
  for (const face of faces.slice(0, MAX_FACES)) {
    if (!face || typeof face !== 'object') continue;
    const family = typeof face.family === 'string' ? face.family.trim().replace(/^(['"])(.*)\1$/, '$2') : '';
    if (!family || family.length > 256 || /[\n\r\f]/.test(family)) continue;
    let src;
    if (typeof face.data === 'string') { if (!/^[A-Za-z0-9+/]+=*$/.test(face.data)) continue; src = `url("data:application/octet-stream;base64,${face.data}")`; }
    else if (plain(face.src) && typeof face.base === 'string') src = absolute(face.src, face.base);
    else continue;
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
