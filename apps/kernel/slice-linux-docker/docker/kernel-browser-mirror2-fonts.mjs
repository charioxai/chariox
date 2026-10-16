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
