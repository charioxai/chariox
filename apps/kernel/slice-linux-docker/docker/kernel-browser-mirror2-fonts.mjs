// MP-08/MP-10/MP-11: positive native font availability, scoped to a document.
// A failed, unused fallback is not a required font. Unknown/oversized state
// remains pending rather than certifying an incomplete requirement list.
export function nativeFontRequirements(document, descriptor, hash) {
  if (document.fonts.status !== 'loaded' || document.fonts.size > 4096) return null;
  const required = new Set();
  for (const font of document.fonts) {
    if (font.status !== 'loaded') continue;
    const value = descriptor(font);
    if (value.length > 4096) return null;
    required.add(hash(value));
    if (required.size > 128) return null;
  }
  return [...required].sort();
}

// MP-08/MP-10/MP-11: families a document's font stacks name that the native machine does not have. The viewer shadows
// them, so its stack falls through exactly where the native one did instead of landing on a family only it installed.
// `installed` memoizes the per-machine answer. Self-contained: it is injected into the isolated world as source.
export function nativeAbsentFonts(root, installed) {
  const view = root.defaultView;
  if (!view || root.fonts.size > 4096) return [];
  const generic = new Set(['serif', 'sans-serif', 'monospace', 'cursive', 'fantasy', 'system-ui', 'ui-serif', 'ui-sans-serif', 'ui-monospace', 'ui-rounded', 'math', 'emoji', 'fangsong', '-webkit-body', 'inherit', 'initial', 'unset']);
  const faces = new Set();
  for (const font of root.fonts) faces.add(font.family.replace(/^(['"])(.*)\1$/, '$2').trim().toLowerCase());
  const stacks = new Set(), all = root.getElementsByTagName('*'), deadline = performance.now() + 30;
  for (let i = 0; i < all.length && i < 4000 && stacks.size < 256; i++) {
    if ((i & 63) === 63 && performance.now() > deadline) break;
    stacks.add(view.getComputedStyle(all[i]).fontFamily);
  }
  let context = null;
  const width = family => { context.font = `64px ${family}`; return context.measureText('mmmmmmmmmmlliWi@#').width; };
  const present = name => {
    if (!installed.has(name)) {
      context ??= root.createElement('canvas').getContext('2d');
      const quoted = JSON.stringify(name);
      installed.set(name, !context || width(`${quoted}, monospace`) !== width('monospace') || width(`${quoted}, serif`) !== width('serif'));
    }
    return installed.get(name);
  };
  const absent = new Set();
  for (const stack of stacks) {
    for (const [, quoted, bare] of stack.matchAll(/"((?:[^"\\]|\\.)*)"|([^,\s][^,]*)/g)) {
      const name = (quoted ?? bare).trim().toLowerCase();
      if (!/^[^\x00-\x1f"'\\{}<>;`]{1,128}$/.test(name) || generic.has(name) || faces.has(name) || absent.has(name)) continue;
      if (!present(name)) absent.add(name);
      if (absent.size >= 64) return [...absent].sort();
    }
  }
  return [...absent].sort();
}
