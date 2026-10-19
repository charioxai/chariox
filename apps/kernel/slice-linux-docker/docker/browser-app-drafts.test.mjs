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
      get(key) { const request = {}; queueMicrotask(() => { request.result = values.get(key); request.onsuccess?.(); }); return request; },
      put(value, key) { const commit = () => { values.set(key, structuredClone(value)); tx.oncomplete(); }; commit.abort = () => tx.onabort(); commits.push(commit); },
    }) };
    if (mode === 'readonly') queueMicrotask(() => tx.oncomplete?.());
    return tx;
  } };
  const store = { values, commits, options, unavailable: false, indexedDB: { open() { const request = {}; queueMicrotask(() => {
    if (store.unavailable) return request.onerror();
    request.result = db; request.onupgradeneeded?.(); request.onsuccess();
  }); return request; } } };
  return store;
}
function page(store, readyState = 'complete') {
  const listeners = new Map(), form = { id: 'todo', getAttribute: () => null };
  const title = { tagName: 'INPUT', type: 'text', id: 'title', name: 'title', value: '', defaultValue: '', form, getAttribute: () => null };
  const secret = { ...title, id: 'password', type: 'password', value: 'private-password' };
  const apiKey = { ...title, id: 'api_key', name: 'api_key', value: 'private-api-key' };
  const protectedField = { ...title, id: 'private-notice', name: 'notice', value: 'private-protected-value', closest: () => ({}) };
  const optOut = { ...title, id: 'optout', value: 'private-opt-out', getAttribute: () => 'OFF' };
  const context = { document: { readyState, querySelectorAll: () => [title, secret, apiKey, protectedField, optOut], addEventListener: (name, fn) => { const previous=listeners.get(name); listeners.set(name, previous ? (...args) => { previous(...args); fn(...args); } : fn); } },
    location: { pathname: '/' }, indexedDB: store.indexedDB, localStorage: { getItem: () => null, removeItem() {} },
    MutationObserver: class { observe() {} }, queueMicrotask };
  vm.runInNewContext(`(${installAppDrafts.toString()})()`, context);
  return { context, title, loaded() { context.document.readyState='interactive'; listeners.get('DOMContentLoaded')?.(); }, input(value) { title.value = value; listeners.get('input')({ target: title }); },
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

// MP-08/MP-10/MP-11: one aborted write (for example quota) must not wedge later drafts.
test('a draft write that aborts is retried by the next edit', async () => {
  const store = storage(), first = page(store);
  await first.context.__charioxAppDraftFlushed();
  first.input('lost write');
  const failed = first.context.__charioxAppDraftFlushed();
  await turn(); store.commits.shift().abort();
  await assert.rejects(failed, /commit failed/);
  first.input('kept after storage recovers');
  const recovering = first.context.__charioxAppDraftFlushed();
  await turn(); store.commits.shift()();
  assert.equal(await recovering, true);
  const recovered = page(store); await recovered.context.__charioxAppDraftFlushed();
  assert.equal(recovered.title.value, 'kept after storage recovers');
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
