import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import { installAppDrafts } from './browser-app-drafts.mjs';

// MP-08/MP-10/MP-11: a published draft must survive losing the renderer.
function storage() {
  const values = new Map(), commits = [], options = [];
  const db = { createObjectStore() {}, transaction(_store, mode, option) {
    options.push(option);
    const tx = { objectStore: () => ({
      get(key) { const request = {}; queueMicrotask(() => {
        if (store.readFailures > 0) { store.readFailures--; return request.onerror(); }
        request.result = values.get(key); request.onsuccess?.();
      }); return request; },
      put(value, key) { const commit = () => { values.set(key, structuredClone(value)); tx.oncomplete(); }; commit.abort = () => tx.onabort(); commits.push(commit); },
    }) };
    if (mode === 'readonly') queueMicrotask(() => tx.oncomplete?.());
    return tx;
  } };
  const store = { values, commits, options, unavailable: false, readFailures: 0, indexedDB: { open() { const request = {}; queueMicrotask(() => {
    if (store.unavailable) return request.onerror();
    request.result = db; request.onupgradeneeded?.(); request.onsuccess();
  }); return request; } } };
  return store;
}
function page(store, readyState = 'complete', selection = 'normal', titleDefault = '') {
  const listeners = new Map(), form = { id: 'todo', getAttribute: () => null };
  const title = { tagName: 'INPUT', type: 'text', id: 'title', name: 'title', value: titleDefault, defaultValue: titleDefault, form, getAttribute: () => null };
  const secret = { ...title, id: 'password', type: 'password', value: 'private-password' };
  const apiKey = { ...title, id: 'api_key', name: 'api_key', value: 'private-api-key' };
  const protectedField = { ...title, id: 'private-notice', name: 'notice', value: 'private-protected-value', closest: () => ({}) };
  const optOut = { ...title, id: 'optout', value: 'private-opt-out', getAttribute: () => 'OFF' };
  const priority = { ...title, tagName: 'SELECT', type: 'select-one', id: 'priority', name: 'priority', value: selection,
    defaultValue: undefined, options: [{ value: 'normal', defaultSelected: true }, { value: 'high', defaultSelected: false }] };
  const notices = [];
  const context = { document: { readyState, body: { append: node => notices.push(node) }, createElement: () => ({ setAttribute() {}, style: {}, remove() { notices.splice(notices.indexOf(this), 1); } }), querySelectorAll: () => [title, secret, apiKey, protectedField, optOut, priority], addEventListener: (name, fn) => { const previous=listeners.get(name); listeners.set(name, previous ? (...args) => { previous(...args); fn(...args); } : fn); } },
    location: { pathname: '/' }, indexedDB: store.indexedDB, localStorage: { getItem: () => null, removeItem() {} },
    MutationObserver: class { observe() {} }, queueMicrotask };
  vm.runInNewContext(`(${installAppDrafts.toString()})()`, context);
  return { context, title, priority, notices, change(value) { priority.value = value; listeners.get('change')({ target: priority }); }, loaded() { context.document.readyState='interactive'; listeners.get('DOMContentLoaded')?.(); }, input(value) { title.value = value; listeners.get('input')({ target: title }); },
    reset() { title.value = ''; listeners.get('reset')(); } };
}
const turn = () => new Promise(resolve => setImmediate(resolve));

test('App draft publication waits for strict commit and reset persists an empty draft', async () => {
  const store = storage(), first = page(store);
  assert.equal(typeof first.context.__charioxAppDraftFlushed, 'function');
  await first.context.__charioxAppDraftFlushed();
  first.input('new unsent title');
  let flushed = false;
  const flush = first.context.__charioxAppDraftFlushed().then(() => { flushed = true; });
  await turn();
  assert.equal(flushed, false);
  assert.equal(store.values.size, 0, 'uncommitted values cannot survive a renderer loss');
  assert.equal(store.options.at(-1).durability, 'strict');
  store.commits.shift()(); await flush;
  const recovered = page(store); await recovered.context.__charioxAppDraftFlushed();
  assert.equal(recovered.title.value, 'new unsent title');
  assert(!JSON.stringify([...store.values]).includes('private-password'));
  assert(!JSON.stringify([...store.values]).includes('private-api-key'));
  assert(!JSON.stringify([...store.values]).includes('private-protected-value'));
  assert(!JSON.stringify([...store.values]).includes('private-opt-out'));
  first.reset(); await turn(); store.commits.shift()(); await first.context.__charioxAppDraftFlushed();
  const cleared = page(store); await cleared.context.__charioxAppDraftFlushed();
  assert.equal(cleared.title.value, '');
});

test('a user edit before asynchronous draft loading takes precedence', async () => {
  const store = storage(); store.values.set('chariox.form-draft.v1:/', { '["todo","INPUT","title"]': 'old draft' });
  const current = page(store); current.input('new edit');
  await turn(); store.commits.shift()(); await current.context.__charioxAppDraftFlushed();
  assert.equal(current.title.value, 'new edit');
  const recovered = page(store); await recovered.context.__charioxAppDraftFlushed();
  assert.equal(recovered.title.value, 'new edit');
});

// MP-08/MP-10/MP-11: a replacement App must not publish its opening document.
test('App publication waits for document readiness and restored fields', async () => {
 const store=storage();store.values.set('chariox.form-draft.v1:/', { '["todo","INPUT","title"]':'recovered draft' });
 const current=page(store,'loading');let flushed=false;
 const ready=current.context.__charioxAppDraftFlushed().then(()=>{flushed=true});
 await turn();assert.equal(flushed,false,'a loading document must not replace the retained view');
 current.loaded();await ready;assert.equal(current.title.value,'recovered draft');
});

// MP-08/MP-10/MP-11: mirror polls keep publishing while storage retries without another edit.
test('failed draft writes do not freeze mirror polls and recover without another edit', async () => {
  const store = storage(), first = page(store);
  await first.context.__charioxAppDraftFlushed();
  first.input('kept after storage recovers');
  const failed = first.context.__charioxAppDraftFlushed();
  await turn(); store.commits.shift().abort();
  assert.equal(await failed, true, 'failed storage must not block mirror publication');
  assert.match(first.notices[0]?.textContent ?? '', /not saved/i);
  for (let n = 0; n < 3; n++) {
    assert.equal(await first.context.__charioxAppDraftFlushed(), true);
    await turn();
    assert.equal(await first.context.__charioxAppDraftFlushed(), true, 'a pending retry must not block a poll');
    assert.equal(store.commits.length, 1, 'only one retry at a time');
    store.commits.shift().abort(); await turn();
  }
  assert.equal(first.title.value, 'kept after storage recovers');
  assert.equal(await first.context.__charioxAppDraftFlushed(), true);
  await turn(); store.commits.shift()(); await turn();
  assert.equal(first.notices.length, 0, 'successful recovery clears the unsaved notice');
  const recovered = page(store); await recovered.context.__charioxAppDraftFlushed();
  assert.equal(recovered.title.value, 'kept after storage recovers');
});

// MP-08/MP-10/MP-11: selects have option defaults, not an input defaultValue.
test('dropdown drafts restore a nonempty default and preserve later user edits', async () => {
  const store = storage(), first = page(store);
  await first.context.__charioxAppDraftFlushed();
  first.change('high');
  const flush = first.context.__charioxAppDraftFlushed();
  await turn(); store.commits.shift()(); await flush;
  const restored = page(store); await restored.context.__charioxAppDraftFlushed();
  assert.equal(restored.priority.value, 'high');
  const appSelected = page(store, 'complete', 'low');
  await appSelected.context.__charioxAppDraftFlushed();
  assert.equal(appSelected.priority.value, 'low', 'a value already changed by the App is retained');
  const edited = page(store); edited.change('normal');
  await turn(); store.commits.shift()(); await edited.context.__charioxAppDraftFlushed();
  assert.equal(edited.priority.value, 'normal');
});

// MP-08/MP-10/MP-11: a failed first open of draft storage must not freeze publication or later drafts.
test('draft storage that fails to open lets the App publish and recovers on the next edit', async () => {
  const store = storage(); store.unavailable = true;
  const first = page(store);
  assert.equal(await first.context.__charioxAppDraftFlushed(), true);
  store.unavailable = false;
  first.input('kept after storage returns');
  const flush = first.context.__charioxAppDraftFlushed();
  await turn(); store.commits.shift()();
  assert.equal(await flush, true);
  const recovered = page(store); await recovered.context.__charioxAppDraftFlushed();
  assert.equal(recovered.title.value, 'kept after storage returns');
});

// MP-08/MP-10/MP-11: retrying an old draft never displaces a newer edit.
test('the newest edit wins while an unsaved draft retry is in flight', async () => {
  const store = storage(), first = page(store);
  await first.context.__charioxAppDraftFlushed(); first.input('old edit');
  const initial = first.context.__charioxAppDraftFlushed();
  await turn(); store.commits.shift().abort(); await initial;
  await first.context.__charioxAppDraftFlushed(); await turn();
  first.input('new edit');
  store.commits.shift()(); await turn();
  assert.equal(first.notices.length, 1, 'old success cannot clear the newer unsaved edit');
  store.commits.shift()(); await turn();
  assert.equal(first.notices.length, 0);
  const restored = page(store); await restored.context.__charioxAppDraftFlushed();
  assert.equal(restored.title.value, 'new edit');
});

// MP-08/MP-10/MP-11: emptying a field is an edit; it must come back empty, not as the field default.
test('a field the person emptied is restored empty', async () => {
  const store = storage(), first = page(store, 'complete', 'normal', 'Untitled');
  await first.context.__charioxAppDraftFlushed();
  first.input('');
  const flush = first.context.__charioxAppDraftFlushed();
  await turn(); store.commits.shift()(); await flush;
  const recovered = page(store, 'complete', 'normal', 'Untitled'); await recovered.context.__charioxAppDraftFlushed();
  assert.equal(recovered.title.value, '');
});

// MP-08/MP-10/MP-11: a transient read failure must never replace the retained draft.
test('a failed draft read is retried before the retained draft is replaced', async () => {
  const store = storage(); store.values.set('chariox.form-draft.v1:/', { '["todo","INPUT","title"]': 'retained draft' });
  store.readFailures = 1;
  const current = page(store);
  await current.context.__charioxAppDraftFlushed();
  assert.equal(current.title.value, '', 'the failed read restored nothing yet');
  current.change('high');
  const flush = current.context.__charioxAppDraftFlushed();
  await turn(); store.commits.shift()(); await flush;
  assert.equal(current.title.value, 'retained draft', 'the retry restores the retained draft');
  assert.equal(store.values.get('chariox.form-draft.v1:/')['["todo","INPUT","title"]'], 'retained draft');
});

test('a draft whose read keeps failing is not written over the retained one', async () => {
  const store = storage(); store.values.set('chariox.form-draft.v1:/', { '["todo","INPUT","title"]': 'retained draft' });
  store.readFailures = 3;
  const current = page(store);
  await current.context.__charioxAppDraftFlushed();
  current.change('high');
  assert.equal(await current.context.__charioxAppDraftFlushed(), true, 'publication is never blocked');
  await turn();
  assert.equal(store.commits.length, 0, 'nothing is written while the retained draft cannot be read');
  assert.match(current.notices[0]?.textContent ?? '', /not saved/i);
  store.readFailures = 0;
  assert.equal(await current.context.__charioxAppDraftFlushed(), true);
  await turn(); store.commits.shift()(); await turn();
  assert.equal(current.notices.length, 0);
  assert.equal(store.values.get('chariox.form-draft.v1:/')['["todo","INPUT","title"]'], 'retained draft');
});
