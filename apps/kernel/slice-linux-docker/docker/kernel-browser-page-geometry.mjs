// MP-08/MP-10/MP-11: native page bounds from the owned Chrome window's GTK
// insets and browser-owned Tab dimensions, independent of page emulation.
export function nativePageRect(window, viewport, scale) {
  const insets = window.frame_insets;
  if (![1, 2].includes(scale) || !Array.isArray(insets) || insets.length !== 4 ||
      !insets.every(n => Number.isSafeInteger(n) && n >= 0 && n <= 64) ||
      ![window.width, window.height, viewport?.width, viewport?.height].every(n => Number.isSafeInteger(n) && n > 0))
    throw Error('MP-11: native page origin unavailable');
  const [left, right, minimumTop, bottom] = insets;
  const width = viewport.width * scale, height = viewport.height * scale;
  const top = window.height - bottom - height;
  if (width < 2 || height < 2 || width > 2560 || height > 1600 ||
      left + width + right !== window.width || top < minimumTop || top <= 0 || top > 400 * scale)
    throw Error('MP-11: native page origin differed');
  return Object.freeze({left, top, width, height});
}
