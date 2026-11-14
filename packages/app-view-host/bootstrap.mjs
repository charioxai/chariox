// Serialized before any publisher code, so it cannot import anything. CSP supplies the resource
// restrictions; these constructors need an additional pre-script fence (notably WebRTC), and
// window.chariox is the page's only service surface.
export function bootstrap() {
  for (const name of ['RTCPeerConnection', 'webkitRTCPeerConnection', 'WebTransport', 'indexedDB', 'webkitIndexedDB']) {
    Object.defineProperty(globalThis, name, { value: undefined, writable: false, configurable: false });
  }
  // Browser libraries use Web Storage for preferences. An opaque document has
  // no storage origin; keep these preferences only for this document's lifetime.
  for (const name of ['localStorage', 'sessionStorage']) {
    const data = new Map();
    const storage = Object.freeze({
      get length() { return data.size; },
      key: index => [...data.keys()][index] ?? null,
      getItem: key => data.get(String(key)) ?? null,
      removeItem: key => { data.delete(String(key)); },
      clear: () => { data.clear(); },
      setItem: (key, value) => {
        key = String(key); value = String(value);
        let size = key.length + value.length;
        for (const [storedKey, storedValue] of data) if (storedKey !== key) size += storedKey.length + storedValue.length;
        if (size > 512 * 1024) throw new DOMException('App preferences exceed the document memory limit', 'QuotaExceededError');
        data.set(key, value);
      },
    });
    Object.defineProperty(globalThis, name, { value: storage, writable: false, configurable: false });
  }
  // Calls travel over one private port that the trusted host transfers after the document loads; the page
  // never holds the client's connection. Calls made before the port arrives wait for it.
  const calls = new Map(), waiting = [];
  let port = null, next = 0;
  addEventListener('message', event => {
    if (port || event.source !== parent || event.data?.type !== 'chariox.app-view.port' || event.ports.length !== 1) return;
    port = event.ports[0];
    port.onmessage = message => {
      const { data } = message, call = data?.type === 'chariox.app-view.result' ? calls.get(data.id) : undefined;
      if (!call) return;
      calls.delete(data.id);
      if (data.ok === true) call.resolve(data.result);
      else call.reject(Object.assign(new Error(String(data.error?.message ?? 'The call did not complete')), { code: String(data.error?.code ?? 'CALL_FAILED') }));
    };
    for (const send of waiting.splice(0)) send();
  });
  const call = (method, input = {}) => new Promise((resolve, reject) => {
    const id = next++;
    calls.set(id, { resolve, reject });
    const send = () => {
      try { port.postMessage({ type: 'chariox.app-view.call', id, method, input }); }
      catch (error) { calls.delete(id); reject(error); }
    };
    if (port) send(); else waiting.push(send);
  });
  // The App backend's change feed (`view_changes`): the changes after `since`, asked again each second until one
  // arrives or `waitMs` (at most 30 s) is up. A reader that fell behind (`dropped`) is answered at once.
  const changes = async (since, waitMs = 0) => {
    if (!Number.isSafeInteger(since) || since < 0 || !Number.isFinite(waitMs) || waitMs < 0) throw new TypeError('changes takes a feed position and a wait in milliseconds');
    const until = Date.now() + Math.min(waitMs, 30000);
    for (;;) {
      const answer = await call('view_changes', { since });
      const left = until - Date.now();
      if (answer.dropped || answer.changes.length || left <= 0) return answer;
      await new Promise(resolve => setTimeout(resolve, Math.min(1000, left)));
    }
  };
  const chariox = Object.freeze({ call, changes });
  Object.defineProperty(globalThis, 'chariox', { value: chariox, writable: false, configurable: false });
}

export function assemble() {
  const bytes = Uint8Array.from(atob(document.getElementById('app-source').textContent), c => c.charCodeAt(0));
  const source = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
  document.getElementById('app-source').remove();
  const style = document.createElement('link'); style.rel = 'stylesheet';
  const css = 'html,body{margin:0;width:100%;height:100%;overflow:hidden}#' + source.rootId + '{width:100%;height:100%}' + source.style;
  style.href = 'data:text/css;base64,' + btoa(Array.from(new TextEncoder().encode(css), byte => String.fromCharCode(byte)).join(''));
  document.head.append(style);
  const root = document.createElement('div'); root.id = source.rootId; document.body.append(root);
  const script = document.createElement('script'); script.type = 'module'; script.textContent = source.script; document.body.append(script);
}
