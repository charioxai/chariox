// Serialized before any publisher code. CSP supplies the resource restrictions;
// these constructors need an additional pre-script fence (notably WebRTC).
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
