// GitHub's dropdown (a <details> with a SelectMenu), closed by a click anywhere else.
import { h, icon, clear } from './dom.mjs';

const open = new Set();
document.addEventListener('click', event => {
  for (const menu of open) if (!menu.contains(event.target)) menu.removeAttribute('open');
});

/** `items()` -> [{ key, text, meta?, selected?, onSelect }]; `filter` adds a search box over the items. */
export function selectMenu({ summary, title, items, filter, empty = 'Nothing found', footer, align = '' }) {
  const list = h('div.SelectMenu-list');
  const search = filter ? h('input.form-control.input-sm.width-full', { type: 'search', placeholder: filter, 'aria-label': filter, autocomplete: 'off' }) : null;
  const details = h(`details.details-reset.details-overlay.d-inline-block.position-relative`, {},
    h('summary.btn.btn-sm', { 'aria-haspopup': 'menu' }, summary, h('span.dropdown-caret.ml-1')),
    h(`div.SelectMenu${align ? '.' + align : ''}`, {}, h('div.SelectMenu-modal', {},
      h('header.SelectMenu-header', {}, h('span.SelectMenu-title', {}, title)),
      search ? h('div.SelectMenu-filter', {}, search) : null,
      list, footer ? h('footer.SelectMenu-footer', {}, footer) : null)));
  const fill = () => {
    clear(list);
    const entries = items(search?.value.trim().toLowerCase() ?? '');
    if (!entries.length) list.append(h('div.SelectMenu-blankslate', {}, empty));
    for (const entry of entries) {
      list.append(h('button.SelectMenu-item', { type: 'button', role: 'menuitemradio', 'aria-checked': entry.selected ? 'true' : 'false', on: { click: () => { details.removeAttribute('open'); entry.onSelect(); } } },
        icon('check', { size: 16, className: 'SelectMenu-icon SelectMenu-icon--check' }),
        h('span.flex-1', {}, entry.text), entry.meta ? h('span.color-fg-muted.text-small.ml-2', {}, entry.meta) : null));
    }
  };
  details.addEventListener('toggle', () => { if (details.open) { open.add(details); fill(); search?.focus(); } else open.delete(details); });
  search?.addEventListener('input', fill);
  return { element: details, refill: fill };
}
