// browserfidelity r19 F5: faces a page makes by script (new FontFace + document.fonts.add, as tldraw does) have no stylesheet
// rule, so the mirror had nothing to carry and the viewer drew their text in a fallback font. This main-world script records
// each script-made face in the document's own FontFaceSet and reports the current list through a CDP binding
// (browser-page-fonts.mjs). Like the permission shim (browser-page-permission-shim.mjs) it keeps Chromium's shapes: the
// same prototype objects, names, lengths and property flags, and native source text. Self-contained: injected as source.
export function installScriptFontRecorder(bindingName) {
  const binding = globalThis[bindingName];
  if (typeof binding !== 'function' || typeof globalThis.FontFace !== 'function' || !globalThis.document?.fonts) return;
  // The binding was only needed to capture the function; no page script can find it by name.
  delete globalThis[bindingName];
  const NativeFontFace = globalThis.FontFace, fontSet = Object.getPrototypeOf(document.fonts);
  // A page may replace these later; the recorder keeps the ones it started with.
  const stringify = JSON.stringify, encode = globalThis.btoa, fromCharCode = String.fromCharCode, later = globalThis.queueMicrotask, forEach = fontSet.forEach;
  const nativeSource = new WeakMap();
  const define = (target, key, change) => {
    const descriptor = Object.getOwnPropertyDescriptor(target, key);
    if (!descriptor) return;
    change(descriptor);
    Object.defineProperty(target, key, descriptor);
  };
  const toString = Function.prototype.toString;
  const patchedToString = new Proxy(toString, {apply: (target, self, args) => nativeSource.get(self) ?? Reflect.apply(target, self, args)});
  nativeSource.set(patchedToString, 'function toString() { [native code] }');
  define(Function.prototype, 'toString', descriptor => { descriptor.value = patchedToString; });

  // Source of each face made here: a CSS src value (string) or its bytes (base64), read once at construction. Faces made in
  // another document (another frame) are not in this document's set and are never reported here.
  const sources = new WeakMap();
  const base64 = bytes => { let binary = ''; for (let i = 0; i < bytes.length; i += 32768) binary += fromCharCode(...bytes.subarray(i, i + 32768)); return encode(binary); };
  const MAX_BYTES = 3 * 1024 * 1024, MAX_SRC = 4 * 1024 * 1024; // a src string may be a data: URL of a font as large
  const sourceOf = value => {
    if (typeof value === 'string') return value.length <= MAX_SRC ? { src: value, base: document.baseURI } : null;
    const view = value instanceof ArrayBuffer ? new Uint8Array(value) : ArrayBuffer.isView(value) ? new Uint8Array(value.buffer, value.byteOffset, value.byteLength) : null;
    return view && view.byteLength <= MAX_BYTES ? { data: base64(view) } : null;
  };
  const RecordedFontFace = new Proxy(NativeFontFace, {
    construct(target, args, newTarget) {
      const face = Reflect.construct(target, args, newTarget === RecordedFontFace ? target : newTarget);
      try { const source = args.length >= 2 ? sourceOf(args[1]) : null; if (source) sources.set(face, source); } catch {}
      return face;
    },
  });
  nativeSource.set(RecordedFontFace, 'function FontFace() { [native code] }');
  define(globalThis, 'FontFace', descriptor => { descriptor.value = RecordedFontFace; });
  define(NativeFontFace.prototype, 'constructor', descriptor => { descriptor.value = RecordedFontFace; });

  // The current list after each change of the set, once per task.
  let queued = false;
  const report = () => {
    if (queued) return; queued = true;
    later(() => {
      queued = false;
      const faces = [];
      Reflect.apply(forEach, document.fonts, [face => {
        const source = sources.get(face); if (!source) return;
        faces[faces.length] = ({ ...source, family: face.family, style: face.style, weight: face.weight, stretch: face.stretch, unicodeRange: face.unicodeRange, featureSettings: face.featureSettings, display: face.display, ascentOverride: face.ascentOverride, descentOverride: face.descentOverride, lineGapOverride: face.lineGapOverride, sizeAdjust: face.sizeAdjust });
      }]);
      try { binding(stringify(faces)); } catch {}
    });
  };
  // A face already in the set can change its descriptors later with no set operation (#1617 review): a set on a script-made
  // face in the document's set reports the list again. The setters keep Chromium's shapes ('set family', length 1).
  const has = fontSet.has;
  for (const key of ['family', 'style', 'weight', 'stretch', 'unicodeRange', 'featureSettings', 'display', 'ascentOverride', 'descentOverride', 'lineGapOverride', 'sizeAdjust']) {
    const original = Object.getOwnPropertyDescriptor(NativeFontFace.prototype, key)?.set;
    if (typeof original !== 'function') continue;
    const wrapped = Object.getOwnPropertyDescriptor({ set [key](value) { Reflect.apply(original, this, [value]); if (sources.has(this) && Reflect.apply(has, document.fonts, [this])) report(); } }, key).set;
    nativeSource.set(wrapped, `function set ${key}() { [native code] }`);
    define(NativeFontFace.prototype, key, descriptor => { descriptor.set = wrapped; });
  }
  const asMethod = (name, impl) => ({[name](...args) { return Reflect.apply(impl, this, args); }})[name];
  for (const key of ['add', 'delete', 'clear']) {
    const original = fontSet[key];
    if (typeof original !== 'function') continue;
    const wrapped = asMethod(key, function (...args) { const result = Reflect.apply(original, this, args); if (this === document.fonts && (key !== 'add' || sources.has(args[0]))) report(); return result; });
    Object.defineProperty(wrapped, 'length', { value: original.length, configurable: true });
    nativeSource.set(wrapped, `function ${key}() { [native code] }`);
    define(fontSet, key, descriptor => { descriptor.value = wrapped; });
  }
}
