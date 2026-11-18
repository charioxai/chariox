// A small element builder. The page runs under a CSP without inline styles or HTML parsing of publisher
// text, so everything from GitHub is placed with textContent and every icon is created as an SVG node.
import { icons } from 'octicons-subset';

const SVG = 'http://www.w3.org/2000/svg';

/** h('div.Box.p-3', { 'aria-label': 'x', on: { click } }, 'text', child) */
export function h(spec, props = {}, ...children) {
  const [tag, ...classes] = spec.split('.');
  const el = document.createElement(tag || 'div');
  if (classes.length) el.className = classes.join(' ');
  for (const [key, value] of Object.entries(props ?? {})) {
    if (value === undefined || value === null || value === false) continue;
    if (key === 'on') for (const [event, handler] of Object.entries(value)) el.addEventListener(event, handler);
    else if (key === 'class') el.classList.add(...String(value).split(' ').filter(Boolean));
    else if (key === 'text') el.textContent = value;
    else el.setAttribute(key, value === true ? '' : String(value));
  }
  append(el, children);
  return el;
}

export function append(el, children) {
  for (const child of children.flat(Infinity)) {
    if (child === undefined || child === null || child === false) continue;
    el.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return el;
}

export const clear = el => { while (el.firstChild) el.firstChild.remove(); return el; };

export function icon(name, { size = 16, label, className = '' } = {}) {
  const data = icons[name];
  if (!data) throw new Error(`unknown icon ${name}`);
  const svg = document.createElementNS(SVG, 'svg');
  svg.setAttribute('class', `octicon octicon-${name} ${className}`.trim());
  svg.setAttribute('viewBox', `0 0 ${data.width} ${data.height}`);
  svg.setAttribute('width', String(size));
  svg.setAttribute('height', String(size));
  svg.setAttribute('fill', 'currentColor');
  if (label) { svg.setAttribute('role', 'img'); svg.setAttribute('aria-label', label); } else svg.setAttribute('aria-hidden', 'true');
  for (const attributes of data.paths) {
    const path = document.createElementNS(SVG, 'path');
    for (const [key, value] of Object.entries(attributes)) path.setAttribute(key, value);
    svg.append(path);
  }
  return svg;
}
