// GitHub's comment box: Write and Preview tabs over one text area. What is typed is kept by the window (`ctx.drafts`)
// under `key`, so reloading the page after a change does not lose it.
import { h } from './dom.mjs';
import { markdown } from './markdown-render.mjs';

export function composer(ctx, key, { placeholder = 'Leave a comment', rows = 5 } = {}) {
  const area = h('textarea.form-control.width-full', { rows, placeholder, 'aria-label': placeholder });
  area.value = ctx.drafts.get(key) ?? '';
  area.addEventListener('input', () => ctx.drafts.set(key, area.value));
  const preview = h('div.markdown-body.p-3.border.rounded-2', { hidden: true });
  const tab = (text, on) => h('button.btn-link.UnderlineNav-item.px-3', { type: 'button', on: { click: () => show(on) } }, text);
  const write = tab('Write', 'write'), view = tab('Preview', 'preview');
  function show(which) {
    const previewing = which === 'preview';
    area.hidden = previewing;
    preview.hidden = !previewing;
    write.classList.toggle('selected', !previewing);
    view.classList.toggle('selected', previewing);
    if (previewing) { preview.replaceChildren(area.value.trim() ? markdown(area.value) : h('span.color-fg-muted', {}, 'Nothing to preview')); }
  }
  show('write');
  const element = h('div', {}, h('nav.UnderlineNav-body.mb-2', { 'aria-label': 'Comment' }, write, view), area, preview);
  return {
    element, area,
    value: () => area.value.trim(),
    clear: () => { area.value = ''; ctx.drafts.delete(key); show('write'); },
  };
}
