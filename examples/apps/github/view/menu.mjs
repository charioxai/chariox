// GitHub's dropdown (a <details> with a SelectMenu), closed by a click anywhere else.
import { h, icon, clear } from './dom.mjs';

const open = new Set();
document.addEventListener('click', event => {
  for (const menu of open) if (!menu.contains(event.target)) menu.removeAttribute('open');
});

/** A details element that closes on a click outside it and on Escape, like GitHub's own dropdowns. */
export function overlay(details) {
  details.addEventListener('toggle', () => { if (details.open) open.add(details); else open.delete(details); });
  return details;
}

document.addEventListener('keydown', event => {
  if (event.key === 'Escape') for (const menu of open) menu.removeAttribute('open');
});

/**
 * `items(query)` -> [{ key, text, meta?, selected?, keepOpen?, onSelect }], or `null` while they are still loading;
 * `filter` adds a search box over the items; `onOpen(refill)` runs each time the menu opens; `plain` is a gear-like
 * icon button instead of GitHub's button with a caret.
 */
export function selectMenu({ summary, title, items, filter, empty = 'Nothing found', footer, align = '', onOpen, plain = false }) {
  const list = h('div.SelectMenu-list');
  const search = filter ? h('input.form-control.input-sm.width-full', { type: 'search', placeholder: filter, 'aria-label': filter, autocomplete: 'off' }) : null;
  const details = h(`details.details-reset.details-overlay.d-inline-block.position-relative`, {},
    plain ? h('summary.btn-octicon', { 'aria-haspopup': 'menu', 'aria-label': title }, summary) : h('summary.btn.btn-sm', { 'aria-haspopup': 'menu' }, summary, h('span.dropdown-caret.ml-1')),
    h(`div.SelectMenu${align ? '.' + align : ''}`, {}, h('div.SelectMenu-modal', {},
      h('header.SelectMenu-header', {}, h('span.SelectMenu-title', {}, title)),
      search ? h('div.SelectMenu-filter', {}, search) : null,
      list, footer ? h('footer.SelectMenu-footer', {}, footer) : null)));
  const fill = () => {
    clear(list);
    const entries = items(search?.value.trim().toLowerCase() ?? '');
    if (!entries) { list.append(h('div.SelectMenu-blankslate', {}, 'Loading…')); return; }
    if (!entries.length) list.append(h('div.SelectMenu-blankslate', {}, empty));
    for (const entry of entries) {
      list.append(h('button.SelectMenu-item', { type: 'button', role: 'menuitemradio', 'aria-checked': entry.selected ? 'true' : 'false', on: { click: () => { if (!entry.keepOpen) details.removeAttribute('open'); entry.onSelect(); } } },
        icon('check', { size: 16, className: 'SelectMenu-icon SelectMenu-icon--check' }),
        h('span.flex-1', {}, entry.text), entry.meta ? h('span.color-fg-muted.text-small.ml-2', {}, entry.meta) : null));
    }
  };
  overlay(details).addEventListener('toggle', () => { if (details.open) { fill(); search?.focus(); onOpen?.(fill); } });
  search?.addEventListener('input', fill);
  return { element: details, refill: fill };
}
