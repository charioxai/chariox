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

test('a list that failed after the feed moved is asked for again on the next poll', async () => {
  let failList = true;
  let listed = 0;
  const { poll } = follower(async () => ({ latest: 3, dropped: false, changes: [] }),
    async () => { listed += 1; if (failList) throw new Error('offline'); });
  await assert.rejects(poll());
  failList = false;
  await poll();
  assert.equal(listed, 2, 'the list that failed is retried although the feed head did not move');
  await poll();
  assert.equal(listed, 2, 'a quiet feed lists nothing once the list succeeded');
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

test('chosen files are read as UTF-8 text; a file that is too big or not text is skipped with its reason', async () => {
  const { readUploads, MAX_BYTES } = await import('../src/upload.js');
  const chosen = [
    new File(['# Notes\n'], 'Notes.md'),
    new File([new Uint8Array([0xff, 0xfe, 0x00])], 'photo.txt'),
    new File([new Uint8Array(MAX_BYTES + 1)], 'big.md'),
  ];
  const read = await readUploads(chosen);
  assert.deepEqual(read.files, [{ name: 'Notes.md', content: '# Notes\n' }]);
  assert.deepEqual(read.skipped, [
    { name: 'photo.txt', reason: 'photo.txt is not UTF-8 text' },
    { name: 'big.md', reason: 'big.md is larger than 512 KB' },
  ]);
  const many = await readUploads(Array.from({ length: 10 }, (_, n) => new File(['x'], `${n}.md`)));
  assert.equal(many.files.length, 8);
  assert.deepEqual(many.skipped.map((file) => file.name), ['8.md', '9.md']);
});

test('files that fit one by one are imported in requests that each fit the App call', async () => {
  const { batches, MAX_CALL_BYTES } = await import('../src/upload.js');
  const size = (batch) => new TextEncoder().encode(JSON.stringify({ files: batch })).length;
  const files = [1, 2, 3].map((n) => ({ name: `${n}.md`, content: 'x'.repeat(200 * 1024) }));
  const split = batches(files);
  assert.deepEqual(split.batches.map((batch) => batch.map((file) => file.name)), [['1.md', '2.md'], ['3.md']]);
  for (const batch of split.batches) assert.ok(size(batch) <= MAX_CALL_BYTES);
  assert.deepEqual(split.skipped, []);
  const escaped = batches([{ name: 'q.md', content: '"'.repeat(300 * 1024) }]);
  assert.deepEqual(escaped.batches, [], 'escaping makes a document too big for one call');
  assert.deepEqual(escaped.skipped, [{ name: 'q.md', reason: 'q.md is too big to import here: with its name and escaped characters it is over 512 KB' }]);
});

test('a file is imported whenever its whole request fits the 512 KiB the App accepts', async () => {
  const { batches } = await import('../src/upload.js');
  const LIMIT = 512 * 1024;
  const size = (file) => new TextEncoder().encode(JSON.stringify({ files: [file] })).length;
  const file = (extra) => ({ name: 'big.md', content: 'x'.repeat(LIMIT - size({ name: 'big.md', content: '' }) + extra) });
  const fits = batches([file(0)]);
  assert.deepEqual(fits.batches.map((batch) => batch.length), [1], 'a 500 KB-class document is not skipped');
  assert.deepEqual(fits.skipped, []);
  assert.equal(size(fits.batches[0][0]), LIMIT);
  const over = batches([file(1)]);
  assert.deepEqual(over.batches, []);
  assert.equal(over.skipped.length, 1);
});

// The page itself, run in Node over a small DOM, against a fake backend with the contract of the App's
// tools. A "view" is one page load; replacing the view loads the page again over the same backend.
class Element {
  constructor(id) {
    Object.assign(this, { id, value: '', hidden: false, textContent: '', children: [], readOnly: false, listeners: {} });
    this.classList = { toggle() {} };
  }
  addEventListener(type, listener) { (this.listeners[type] ??= []).push(listener); }
  dispatch(type, event = {}) { for (const listener of this.listeners[type] ?? []) listener(event); }
  click() { this.dispatch('click'); }
  setAttribute() {}
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  set innerHTML(_) {}
}

function fakeBackend() {
  const store = {
    docs: new Map(),
    // One draft per (editor, document), as the App keeps them; `draft` is the latest, for single-window tests.
    drafts: new Map(),
    get draft() { return [...this.drafts.values()].at(-1) ?? null; },
  };
  const summary = ({ content, ...doc }) => doc;
  const calls = [];
  const tools = {
    list_documents: () => ({ documents: [...store.docs.values()].map(summary) }),
    read_document: ({ id }) => ({ ...store.docs.get(id) }),
    update_document: ({ id, expected_revision, content, title, folder }) => {
      const doc = store.docs.get(id);
      if (doc.revision !== expected_revision) throw Error(`revision ${doc.revision} is newer than ${expected_revision}`);
      Object.assign(doc, { content, title, folder, revision: doc.revision + 1, versions: [...doc.versions, doc.revision + 1] });
      return { ...doc };
    },
    view_changes: () => ({ latest: 1, dropped: false, changes: [] }),
    save_draft: (draft) => {
      if (typeof draft.editor !== 'string') throw Error('editor is required');
      const key = `${draft.editor}/${draft.id}`;
      store.drafts.delete(key);
      store.drafts.set(key, draft);
      return { saved: true };
    },
    // The contract of the App's claim: an editor keeps its own latest draft, else is handed a COPY of the
    // latest of another editor, whose draft stays. A draft of a deleted document is not offered.
    claim_draft: ({ editor }) => {
      if (typeof editor !== 'string') throw Error('editor is required');
      const all = [...store.drafts.values()].filter((draft) => store.docs.has(draft.id));
      const own = all.filter((draft) => draft.editor === editor).at(-1);
      const other = all.filter((draft) => draft.editor !== editor).at(-1);
      if (own) return { draft: own };
      if (!other) return { draft: null };
      const copy = { ...other, editor };
      store.drafts.set(`${editor}/${copy.id}`, copy);
      return { draft: copy };
    },
    discard_draft: ({ editor, id }) => {
      if (typeof editor !== 'string' || typeof id !== 'string') throw Error('editor and id are required');
      store.drafts.delete(`${editor}/${id}`);
      return { discarded: true };
    },
  };
  const backend = { store, calls, holdSaves: null };
  const call = async (tool, input) => {
    calls.push(tool);
    if (!tools[tool]) throw Error(`no tool ${tool}`);
    if (tool === 'update_document') await backend.holdSaves;
    return structuredClone(tools[tool](input));
  };
  backend.call = call;
  store.docs.set('d1', { id: 'd1', title: 'Notes', folder: '', kind: 'markdown', revision: 1, versions: [1], content: 'saved text' });
  return backend;
}

let loads = 0;
async function loadView(backend) {
  const elements = new Map();
  const pagehide = [];
  const element = (id) => elements.get(id) ?? elements.set(id, new Element(id)).get(id);
  element('preview').hidden = true;
  Object.assign(globalThis, {
    window: { chariox: { call: backend.call } },
    document: { hidden: false, getElementById: element, createElement: () => new Element(''), body: new Element('body'), addEventListener() {} },
    addEventListener: (type, listener) => { if (type === 'pagehide') pagehide.push(listener); },
    setInterval: () => 0,
    Option: class { constructor(text, value) { Object.assign(this, { text, value }); } },
  });
  await import(`../src/main.js?view=${(loads += 1)}`);
  return { $: element, pagehide: () => pagehide.forEach((listener) => listener()) };
}

async function until(condition, what) {
  for (let waited = 0; waited < 3000; waited += 10) {
    if (condition()) return;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  assert.fail(`never reached: ${what}`);
}

async function openNotes(view) {
  await until(() => view.$('docs').children.length === 1, 'the document list');
  view.$('docs').children[0].children[0].click();
  await until(() => view.$('content').value === 'saved text', 'the document opened');
}

function type(view, fields) {
  for (const [id, value] of Object.entries(fields)) {
    view.$(id).value = value;
    view.$(id).dispatch('input');
  }
}

test('an unsaved edit survives the replacement of the view and keeps the revision it started from', async () => {
  const backend = fakeBackend();
  const first = await loadView(backend);
  await openNotes(first);
  type(first, { content: 'unsaved edit', title: 'Renamed', folder: 'work' });
  await until(() => backend.store.draft?.content === 'unsaved edit', 'the App keeps the draft');

  // The App updates, or the connection returns: a new view replaces this one.
  const second = await loadView(backend);
  await until(() => second.$('content').value === 'unsaved edit', 'the unsaved edit comes back');
  assert.equal(second.$('title').value, 'Renamed');
  assert.equal(second.$('folder').value, 'work');
  assert.match(second.$('status').textContent, /Restored your unsaved changes/);

  // Someone else saved meanwhile: the restored edit is refused, never merged.
  backend.store.docs.get('d1').revision = 2;
  second.$('save').click();
  await until(() => /newer than 1/.test(second.$('status').textContent), 'the stale save is refused');
  assert.equal(backend.store.docs.get('d1').content, 'saved text');
  assert.ok(backend.store.draft, 'the refused edit is still kept');
});

test('a saved edit leaves no draft to restore', async () => {
  const backend = fakeBackend();
  const first = await loadView(backend);
  await openNotes(first);
  type(first, { content: 'saved edit' });
  first.$('save').click();
  await until(() => backend.store.docs.get('d1').content === 'saved edit', 'the save');
  await until(() => backend.store.draft === null, 'the draft is dropped');

  const second = await loadView(backend);
  await until(() => second.$('docs').children.length === 1, 'the list');
  await until(() => backend.calls.filter((tool) => tool === 'claim_draft').length === 2, 'the new view asked for a draft');
  assert.equal(second.$('content').value, '', 'nothing is opened for a view with no draft');
});

test('a window that saves discards its own document\'s draft and leaves another window\'s', async () => {
  const backend = fakeBackend();
  backend.store.docs.set('d2', { id: 'd2', title: 'Other', folder: '', kind: 'markdown', revision: 1, versions: [1], content: 'other text' });
  const first = await loadView(backend);
  await until(() => first.$('docs').children.length === 2, 'the document list');
  first.$('docs').children[0].children[0].click();
  await until(() => first.$('content').value === 'saved text', 'the document opened');
  type(first, { content: 'saved edit' });
  await until(() => [...backend.store.drafts.values()].some((draft) => draft.id === 'd1'), 'the App keeps the draft');
  backend.store.drafts.set('other/d2', { editor: 'other', id: 'd2', base_revision: 1, title: 'Other', folder: '', content: 'edited in another window' });
  first.$('save').click();
  await until(() => ![...backend.store.drafts.values()].some((draft) => draft.id === 'd1'), 'the saved document\'s draft is dropped');
  assert.equal(backend.store.drafts.get('other/d2')?.content, 'edited in another window');
});

// Two windows on one document each have their own draft, so what one saves or gives up never reaches the
// other's edit, and the window that replaces the other's view finds that edit again.
async function otherWindowEdits(backend, content) {
  await backend.call('save_draft', { editor: 'other', id: 'd1', base_revision: 1, title: 'Notes', folder: '', content });
}

test('a window that saves its document leaves another window\'s edit of the same document', async () => {
  const backend = fakeBackend();
  const first = await loadView(backend);
  await openNotes(first);
  type(first, { content: 'edit from this window' });
  await until(() => backend.store.draft?.content === 'edit from this window', 'this window\'s draft is kept');
  await otherWindowEdits(backend, 'edit from the other window');
  first.$('save').click();
  await until(() => backend.store.docs.get('d1').content === 'edit from this window', 'the save');
  await until(() => backend.store.drafts.size === 1, 'this window\'s draft is dropped');
  assert.equal(backend.store.drafts.get('other/d1').content, 'edit from the other window');

  const second = await loadView(backend);
  await until(() => second.$('content').value === 'edit from the other window', 'the other window\'s edit comes back');
});

test('a window that gives up its edit discards only its own draft of the document', async () => {
  const backend = fakeBackend();
  const first = await loadView(backend);
  await openNotes(first);
  type(first, { content: 'edit from this window' });
  await until(() => backend.store.draft?.content === 'edit from this window', 'this window\'s draft is kept');
  await otherWindowEdits(backend, 'edit from the other window');
  const pick = () => first.$('docs').children[0].children[0].click();
  pick();
  await until(() => /Unsaved changes/.test(first.$('status').textContent), 'the window asks before it gives up the edit');
  pick();
  await until(() => first.$('content').value === 'saved text', 'the edit is given up');
  await until(() => backend.store.drafts.size === 1, 'this window\'s draft is discarded');
  assert.deepEqual([...backend.store.drafts.keys()], ['other/d1']);
});

// The review's first repro: a window that is still open keeps its edit through another window starting,
// restoring it and giving it up, and the next window to start finds it.
test('a window that restored another window\'s edit and gave it up leaves that window\'s draft for the next one', async () => {
  const backend = fakeBackend();
  const first = await loadView(backend);
  await openNotes(first);
  type(first, { content: 'edit from the open window' });
  await until(() => backend.store.draft?.content === 'edit from the open window', 'the open window\'s draft is kept');
  const [openKey] = backend.store.drafts.keys();

  const second = await loadView(backend);
  await until(() => second.$('content').value === 'edit from the open window', 'the second window restores the edit');
  const pick = () => second.$('docs').children[0].children[0].click();
  pick();
  await until(() => /Unsaved changes/.test(second.$('status').textContent), 'the second window asks before it gives up the edit');
  pick();
  await until(() => second.$('content').value === 'saved text', 'the second window gave the edit up');
  await until(() => backend.store.drafts.size === 1, 'the second window\'s own copy is discarded');
  assert.deepEqual([...backend.store.drafts.keys()], [openKey], 'the open window\'s draft is untouched');

  const third = await loadView(backend);
  await until(() => third.$('content').value === 'edit from the open window', 'the next window finds the edit');
});

test('a window whose own draft exists is not handed another window\'s edit', async () => {
  const backend = fakeBackend();
  const first = await loadView(backend);
  await openNotes(first);
  type(first, { content: 'edit from this window' });
  await until(() => backend.store.draft?.content === 'edit from this window', 'this window\'s draft is kept');
  const own = backend.store.drafts.values().next().value.editor;
  await otherWindowEdits(backend, 'edit from the other window');
  const claimed = await backend.call('claim_draft', { editor: own });
  assert.equal(claimed.draft.content, 'edit from this window');
  assert.equal(backend.store.drafts.size, 2, 'the other window\'s draft stays where it is');
});

test('a view that is going away hands over its latest keystrokes at once', async () => {
  const backend = fakeBackend();
  const first = await loadView(backend);
  await openNotes(first);
  type(first, { content: 'typed just before the replacement' });
  first.pagehide();
  await until(() => backend.store.draft?.content === 'typed just before the replacement', 'the draft is handed over');
});

test('an edit typed while a save is in flight is kept on the revision that save acknowledged', async () => {
  const backend = fakeBackend();
  let release;
  backend.holdSaves = new Promise((resolve) => { release = resolve; });
  const first = await loadView(backend);
  await openNotes(first);
  type(first, { content: 'sent edit' });
  first.$('save').click();
  await until(() => backend.calls.includes('update_document'), 'the save is in flight');
  type(first, { content: 'later edit' });
  await until(() => backend.store.draft?.content === 'later edit', 'the later edit is kept on revision 1');
  assert.equal(backend.store.draft.base_revision, 1);

  release();
  await until(() => backend.store.docs.get('d1').revision === 2, 'the save lands as revision 2');
  await until(() => backend.store.draft?.base_revision === 2, 'the kept edit moves to the acknowledged revision');

  const second = await loadView(backend);
  await until(() => second.$('content').value === 'later edit', 'the later edit comes back');
  second.$('save').click();
  await until(() => backend.store.docs.get('d1').content === 'later edit', 'the person\'s own revision does not refuse the save');
});
