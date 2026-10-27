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
  const defaultValue = field => field.tagName === 'SELECT'
    ? ([...field.options].filter(option => option.defaultSelected).at(-1)
      ?? [...field.options].find(option => !option.disabled && !option.parentElement?.disabled))?.value ?? ''
    : field.defaultValue ?? '';
  const restore = () => {
    for (const field of fields()) {
      if (!loaded || restored.has(field) || edited.has(field)) continue;
      restored.add(field);
      const value = saved[identity(field)];
      if (Object.hasOwn(saved, identity(field)) && typeof value === 'string' && value.length <= 16384
        && field.value === defaultValue(field)) field.value = value;
    }
  };
  const snapshot = () => {
    const next = {};
    for (const field of fields()) {
      // Emptied fields are kept: an empty value differs from a nonempty default.
      if (typeof field.value === 'string' && field.value !== defaultValue(field) && field.value.length <= 16384) next[identity(field)] = field.value;
    }
    return JSON.stringify(next).length <= 65536 ? next : {};
  };
  // A failed open is not cached: the next edit reopens, so unavailable storage never freezes the App.
  let database;
  const open = () => database ??= new Promise((resolve, reject) => {
    const request = indexedDB.open('chariox-form-drafts', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('drafts');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(new Error('App draft storage unavailable'));
  }).catch(error => { database = undefined; throw error; });
  const commit = async value => {
    const db = await open();
    await new Promise((resolve, reject) => {
      // Strict completion includes flushing to disk, unlike localStorage or
      // IndexedDB's default relaxed transaction. Empty drafts are tombstones.
      const tx = db.transaction('drafts', 'readwrite', { durability: 'strict' });
      tx.oncomplete = resolve;
      tx.onabort = tx.onerror = () => reject(new Error('App draft commit failed'));
      tx.objectStore('drafts').put(value, key);
    });
  };
  const read = () => open().then(db => new Promise((resolve, reject) => {
    const request = db.transaction('drafts', 'readonly').objectStore('drafts').get(key);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(new Error('App draft read failed'));
  })).then(value => {
    saved = value && typeof value === 'object' && !Array.isArray(value) ? value : {};
    loaded = true; restore();
  });
  let pending = read().catch(() => {});
  let latest = null, queued = 0, unsaved = false, notice;
  const setUnsaved = value => {
    unsaved = value;
    if (value && !notice && document.body) {
      notice = document.createElement('div');
      notice.setAttribute('role', 'status');
      notice.setAttribute('data-chariox-draft-status', 'unsaved');
      notice.textContent = 'Draft not saved on this device. Retrying…';
      notice.style.cssText = 'position:fixed;bottom:8px;right:8px;z-index:2147483647;padding:6px 10px;background:#fff3cd;color:#332701;border-radius:4px;font:13px system-ui';
      document.body.append(notice);
    } else if (!value && notice) { notice.remove(); notice = undefined; }
  };
  const enqueue = draft => {
    ++queued;
    pending = pending.then(async () => {
      try {
        // A retained draft that could not be read is never replaced: read it again first.
        if (!loaded) await read();
        saved = draft.value ?? snapshot();
        draft.value = saved;
        await commit(saved);
        if (latest === draft) { latest = null; setUnsaved(false); }
      } catch { setUnsaved(true); }
      finally { --queued; }
    });
  };
  const save = () => {
    latest = {value: loaded ? snapshot() : null};
    enqueue(latest);
  };
  // Keep strict durability before normal publication. After a storage failure,
  // publish with an unsaved notice and retry the latest draft independently.
  const ready = document.readyState === 'loading'
    ? new Promise(resolve => document.addEventListener('DOMContentLoaded', resolve, {once:true})) : Promise.resolve();
  Object.defineProperty(globalThis, '__charioxAppDraftFlushed', { value: async () => {
    await ready;
    if (unsaved) { if (latest && !queued) enqueue(latest); }
    else await pending;
    restore(); return true;
  } });
  for (const event of ['input', 'change']) document.addEventListener(event, e => { if (eligible(e.target)) { edited.add(e.target); save(); } }, true);
  // reset fires before the browser applies default values, including App-driven resets.
  document.addEventListener('reset', () => queueMicrotask(save), true);
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', restore, {once:true});
  else restore();
  new MutationObserver(restore).observe(document, {childList:true, subtree:true});
}
