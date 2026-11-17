// A label pill coloured as GitHub colours it: the label's own colour, text black or white by perceived lightness.
export function labelColors(hex) {
  const value = /^[0-9a-f]{6}$/i.test(hex ?? '') ? hex : 'ededed';
  const [r, g, b] = [0, 2, 4].map(i => parseInt(value.slice(i, i + 2), 16));
  const lightness = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  return { background: `#${value}`, color: lightness > 0.6 ? '#1f2328' : '#ffffff', border: `rgba(${r}, ${g}, ${b}, 0.3)` };
}

/** An avatar placeholder: the page may not load images, so a person is a coloured disc with their initial. */
export function avatarColor(login = '?') {
  let hash = 0;
  for (const ch of login) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return `hsl(${hash % 360}, 45%, 42%)`;
}
