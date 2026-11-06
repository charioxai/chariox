// Shared by Web and the future TUI default-browser host. This attaches verified
// frontend bytes only; kernel admission and backend port routing belong to AV2/3.
export function mountAppView(container, { hostUrl, bundle, trust, timeoutMs = 10000 }) {
  const url = new URL(hostUrl, location.href);
  if (url.origin !== location.origin || url.search || url.hash) throw new Error('App host must be a same-origin static document');
  const host = document.createElement('iframe'); host.title = 'App host'; host.src = url.href;
  host.style.cssText = 'display:block;width:100%;height:100%;border:0';
  let finish;
  const ready = new Promise((resolve, reject) => {
    const channel = new MessageChannel();
    let settled = false;
    const timer = setTimeout(() => finish(new Error('App host did not mount')), timeoutMs);
    finish = error => {
      if (settled) return; settled = true;
      clearTimeout(timer); removeEventListener('message', attached); channel.port1.close(); channel.port2.close();
      if (error) { host.remove(); reject(error); } else resolve();
    };
    const attached = event => {
      if (event.source !== host.contentWindow || event.origin !== url.origin || event.data?.type !== 'chariox.app-host.ready') return;
      removeEventListener('message', attached);
      host.contentWindow.postMessage({ type: 'chariox.app-host.attach', bundle, trust }, url.origin, [channel.port2]);
    };
    addEventListener('message', attached);
    channel.port1.onmessage = event => {
      if (event.data?.type === 'chariox.app-host.mounted') finish();
      else if (event.data?.type === 'chariox.app-host.failed') finish(new Error(event.data.reason));
    };
    container.append(host);
  });
  return { ready, close: () => { finish(new Error('App view closed')); host.remove(); } };
}
