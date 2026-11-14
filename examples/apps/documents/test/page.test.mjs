// The page's own logic in Node: the change follower and the safe previews. The window itself
// (layout, editor, DPR) is checked in a real browser against a real kernel.
import assert from 'node:assert/strict';
import test from 'node:test';
import { buildView } from '../scripts/build-client-view.mjs';
import { follower } from '../src/feed.js';
import { markdownToHtml, sanitizeHtmlInto } from '../src/preview.js';

test('the follower lists once at the start, then only when the feed moved or lost changes', async () => {
  const asked = [];
  let feed = { latest: 4, dropped: false, changes: [] };
  const call = async (method, input) => { asked.push([method, input.since]); return feed; };
  let listed = 0;
  const { poll } = follower(call, async () => { listed += 1; });
  await poll();
  assert.equal(listed, 1);
  assert.ok(asked[0][1] > 1e15, 'the first read asks for no changes, only where the feed is');
  await poll();
  assert.equal(listed, 1, 'a quiet feed lists nothing');
  assert.equal(asked[1][1], 4);
  feed = { latest: 6, dropped: false, changes: [{ seq: 5 }, { seq: 6 }] };
  await poll();
  assert.equal(listed, 2, 'an agent saved twice: one list');
  feed = { latest: 6, dropped: true, changes: [] };
  await poll();
  assert.equal(listed, 3, 'a feed that moved past this window is read again from the list');
});

test('a failed read of the feed lists nothing and is tried again', async () => {
  let fail = true;
  let listed = 0;
  const { poll } = follower(async () => { if (fail) throw new Error('offline'); return { latest: 1, dropped: false, changes: [] }; },
    async () => { listed += 1; });
  await assert.rejects(poll());
  fail = false;
  await poll();
  assert.equal(listed, 1);
});

test('Markdown preview escapes document text and emits only its own tags', () => {
  const html = markdownToHtml('# Title <script>x</script>\n- **bold** `a<b>`\n\n```\n<img src=x onerror=1>\n```');
  assert.doesNotMatch(html, /<script|<img/);
  assert.match(html, /<h1>Title &lt;script&gt;x&lt;\/script&gt;<\/h1>/);
  assert.match(html, /<ul>\n<li><strong>bold<\/strong> <code>a&lt;b&gt;<\/code><\/li>\n<\/ul>/);
  assert.match(html, /<pre><code>&lt;img src=x onerror=1&gt;<\/code><\/pre>/);
});

test('HTML preview drops foreign SVG and MathML subtrees whose names are lowercase', () => {
  // A parsed tree as DOMParser shapes it: HTML names upper-case, foreign ones as written.
  const el = (nodeName, ...childNodes) => ({ nodeType: 1, nodeName, tagName: nodeName, childNodes, getAttribute: () => null });
  const text = (textContent) => ({ nodeType: 3, textContent });
  const body = el('BODY', el('P', text('kept')), el('svg', el('a', text('svg link'))), el('math', el('mi', text('x'))), el('FONT', text('unwrapped')));
  const node = (name) => ({ name, children: [], append(...nodes) { this.children.push(...nodes); }, setAttribute() {} });
  const target = { createTextNode: (value) => value, createElement: node, createDocumentFragment: () => node('') };
  const render = (item) => typeof item === 'string' ? item : item.name ? `<${item.name}>${item.children.map(render).join('')}</${item.name}>` : item.children.map(render).join('');
  assert.equal(render(sanitizeHtmlInto('', target, () => ({ body }))), '<p>kept</p>unwrapped');
});

test('the page builds into one self-contained view that reaches the App only through chariox.call', async () => {
  const view = new TextDecoder().decode(await buildView());
  assert.match(view, /^<!doctype html>/);
  const source = JSON.parse(Buffer.from(view.slice(view.indexOf('>', view.indexOf('chariox-app-view')) + 1, -'</script>'.length), 'base64'));
  assert.equal(source.rootId, 'app');
  assert.match(source.script, /view_changes/);
  assert.doesNotMatch(source.script + source.style, /https?:\/\/|sessionStorage|<form/);
});
