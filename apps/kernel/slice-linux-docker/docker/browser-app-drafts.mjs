// MP-08/MP-10/MP-11: browser-local editing state, never App calls or authority.
// Runs inside the verified App's own origin, beside its existing web storage.
export function installAppDrafts() {
  if (typeof document === 'undefined' || typeof location === 'undefined') return;
  const key = 'chariox.form-draft.v1:' + location.pathname;
  const eligible = field => field.form && !field.disabled && !field.readOnly
    && (field.id || field.name)
    && /^(?:INPUT|TEXTAREA|SELECT)$/.test(field.tagName)
    && (field.tagName !== 'INPUT' || /^(?:text|search|url|email|tel|number|date|datetime-local|time)$/.test(field.type))
    && !field.closest?.('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected]')
    && !/off|password|one-time-code|cc-/i.test(field.getAttribute('autocomplete') ?? '')
    && !/off/i.test(field.form.getAttribute?.('autocomplete') ?? '')
    && !/password|passwd|passcode|token|secret|credential|authorization|(?:api|access|private|signing|auth)[_ -]?key|\botp\b|\bcvv\b|\bcvc\b/i.test(field.id + ' ' + field.name);
  const identity = field => JSON.stringify([field.form.id || field.form.name || '', field.tagName, field.id || field.name]);
  const fields = () => [...document.querySelectorAll('input,textarea,select')].filter(eligible).slice(0, 128);
  let saved = {}, loaded = false;
  const restored = new WeakSet(), edited = new WeakSet();
  const restore = () => {
    for (const field of fields()) {
      if (!loaded || restored.has(field) || edited.has(field)) continue;
      restored.add(field);
      const value = saved[identity(field)];
      if (Object.hasOwn(saved, identity(field)) && typeof value === 'string' && value.length <= 16384
        && field.value === (field.defaultValue ?? '')) field.value = value;
    }
  };
  const snapshot = () => {
    const next = {};
    for (const field of fields()) {
      if (typeof field.value === 'string' && field.value && field.value.length <= 16384) next[identity(field)] = field.value;
    }
    return JSON.stringify(next).length <= 65536 ? next : {};
  };
  const opened = new Promise((resolve, reject) => {
    const request = indexedDB.open('chariox-form-drafts', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('drafts');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(new Error('App draft storage unavailable'));
  });
  const commit = async value => {
    const db = await opened;
    await new Promise((resolve, reject) => {
      // Strict completion includes flushing to disk, unlike localStorage or
      // IndexedDB's default relaxed transaction. Empty drafts are tombstones.
      const tx = db.transaction('drafts', 'readwrite', { durability: 'strict' });
      tx.oncomplete = resolve;
      tx.onabort = tx.onerror = () => reject(new Error('App draft commit failed'));
      tx.objectStore('drafts').put(value, key);
    });
  };
  let pending = opened.then(db => new Promise((resolve, reject) => {
    const request = db.transaction('drafts', 'readonly').objectStore('drafts').get(key);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(new Error('App draft read failed'));
  })).then(value => {
    saved = value && typeof value === 'object' && !Array.isArray(value) ? value : {};
    loaded = true; restore();
  });
  const save = () => {
    const captured = loaded ? snapshot() : null;
    pending = pending.then(async () => { saved = captured ?? snapshot(); await commit(saved); });
    pending.catch(() => {}); // The publication fence still observes this failure.
  };
  pending.catch(() => {});
  // Private renderer fence, not an App SDK method or serialized protocol.
  Object.defineProperty(globalThis, '__charioxAppDraftFlushed', { value: async () => { await pending; return true; } });
  for (const event of ['input', 'change']) document.addEventListener(event, e => { if (eligible(e.target)) { edited.add(e.target); save(); } }, true);
  // reset fires before the browser applies default values, including App-driven resets.
  document.addEventListener('reset', () => queueMicrotask(save), true);
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', restore, {once:true});
  else restore();
  new MutationObserver(restore).observe(document, {childList:true, subtree:true});
}
