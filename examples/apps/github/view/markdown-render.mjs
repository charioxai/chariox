// Markdown as nodes: textContent and createElement only, never HTML. Links keep their address as a title; the page
// cannot navigate.
import { h } from './dom.mjs';
import { parse } from './markdown.mjs';

const SAFE_URL = /^(https?:\/\/|mailto:)/i;

const inline = nodes => nodes.map(node => {
  switch (node.t) {
    case 'text': return node.text;
    case 'code': return h('code', { text: node.text });
    case 'strong': return h('strong', {}, inline(node.children));
    case 'em': return h('em', {}, inline(node.children));
    case 'del': return h('del', {}, inline(node.children));
    case 'image': return h('span', { class: 'color-fg-muted' }, node.text || 'image');
    case 'mention': return h('span', { class: 'user-mention' }, node.text);
    case 'ref': return h('span', { class: 'issue-link' }, node.text);
    // The page cannot navigate; the address is kept as the link's title.
    case 'link': return h('a', { class: SAFE_URL.test(node.href) ? '' : 'color-fg-muted', title: SAFE_URL.test(node.href) ? node.href : undefined, on: { click: event => event.preventDefault() } }, inline(node.children));
    default: return '';
  }
});

export function render(blocks) {
  return blocks.map(block => {
    switch (block.t) {
      case 'heading': return h(`h${Math.min(block.level, 6)}`, {}, inline(block.children));
      case 'paragraph': return h('p', {}, block.lines.map((line, index) => [index ? h('br') : '', inline(line)]));
      case 'fence': return h('pre', {}, h('code', { text: block.text }));
      case 'rule': return h('hr');
      case 'quote': return h('blockquote', {}, render(block.children));
      case 'list': return h(block.ordered ? 'ol' : 'ul', {}, block.items.map(item => h('li', { class: item.checked === null ? '' : 'task-list-item' },
        item.checked === null ? '' : h('input', { type: 'checkbox', disabled: true, checked: item.checked ? true : undefined, class: 'task-list-item-checkbox' }), inline(item.children))));
      case 'table': return h('table', {}, h('thead', {}, h('tr', {}, block.head.map(cell => h('th', {}, inline(cell))))),
        h('tbody', {}, block.rows.map(row => h('tr', {}, row.map(cell => h('td', {}, inline(cell)))))));
      default: return '';
    }
  });
}

/** A rendered body: `<div class="markdown-body">`. */
export const markdown = source => h('div.markdown-body.comment-body', {}, render(parse(source)));
