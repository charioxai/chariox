// MP-08/MP-10/MP-11: browser-local editing state, never App calls or authority.
// Runs inside the verified App's own origin, beside its existing web storage.
export function installAppDrafts() {
  if (typeof document === 'undefined' || typeof location === 'undefined') return;
  const key = 'chariox.form-draft.v1:' + location.pathname;
  const eligible = field => field.form && !field.disabled && !field.readOnly
    && (field.id || field.name)
    && /^(?:INPUT|TEXTAREA|SELECT)$/.test(field.tagName)
    && (field.tagName !== 'INPUT' || /^(?:text|search|url|email|tel|number|date|datetime-local|time)$/.test(field.type))
    && !/off|password|one-time-code|cc-/.test(field.getAttribute('autocomplete') ?? '')
    && !/off/.test(field.form.getAttribute?.('autocomplete') ?? '')
    && !/password|passwd|token|secret|credential|authorization/i.test(field.id + ' ' + field.name);
  const identity = field => JSON.stringify([field.form.id || field.form.name || '', field.tagName, field.id || field.name]);
  const fields = () => [...document.querySelectorAll('input,textarea,select')].filter(eligible).slice(0, 128);
  let saved;
  try {
    const text = localStorage.getItem(key);
    saved = text && text.length <= 65536 ? JSON.parse(text) : {};
    if (!saved || typeof saved !== 'object' || Array.isArray(saved)) saved = {};
  } catch { saved = {}; }
  const restored = new WeakSet();
  const restore = () => {
    for (const field of fields()) {
      if (restored.has(field)) continue;
      restored.add(field);
      const value = saved[identity(field)];
      if (Object.hasOwn(saved, identity(field)) && typeof value === 'string' && value.length <= 16384
        && field.value === (field.defaultValue ?? '')) field.value = value;
    }
  };
  const save = () => {
    const next = {};
    for (const field of fields()) {
      if (typeof field.value === 'string' && field.value && field.value.length <= 16384) next[identity(field)] = field.value;
    }
    try {
      const text = JSON.stringify(next);
      if (text.length <= 65536 && Object.keys(next).length) localStorage.setItem(key, text);
      else localStorage.removeItem(key);
      saved = next;
    } catch {}
  };
  for (const event of ['input', 'change']) document.addEventListener(event, e => { if (eligible(e.target)) save(); }, true);
  // reset fires before the browser applies default values, including App-driven resets.
  document.addEventListener('reset', () => queueMicrotask(save), true);
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', restore, {once:true});
  else restore();
  new MutationObserver(restore).observe(document, {childList:true, subtree:true});
}
