import { serveCalls } from './call-bridge.mjs';

// Shared by Web and the future TUI default-browser host. The verified frontend bytes attach to the static host;
// `call(method, input)` is the client's own authority over the App's backend, served to the page over a private port.
export function mountAppView(container, { hostUrl, bundle, trust, call, timeoutMs = 10000 }) {
  if (typeof call !== 'function') throw new Error('An App view needs the call that serves its backend');
  // The container may sit in a window the page popped out; its messages arrive there, not here.
  const view = container.ownerDocument.defaultView;
  const url = new URL(hostUrl, view.location.href);
  if (url.origin !== view.location.origin || url.search || url.hash) throw new Error('App host must be a same-origin static document');
  const host = container.ownerDocument.createElement('iframe'); host.title = 'App host'; host.src = url.href;
  host.style.cssText = 'display:block;width:100%;height:100%;border:0';
  const channel = new MessageChannel();
  let finish, stopServing = () => {};
  const ready = new Promise((resolve, reject) => {
    let settled = false;
    const timer = setTimeout(() => finish(new Error('App host did not mount')), timeoutMs);
    finish = error => {
      if (settled) return; settled = true;
      clearTimeout(timer); view.removeEventListener('message', attached);
      if (error) { stopServing(); channel.port1.close(); channel.port2.close(); host.remove(); reject(error); } else resolve();
    };
    const attached = event => {
      if (event.source !== host.contentWindow || event.origin !== url.origin || event.data?.type !== 'chariox.app-host.ready') return;
      view.removeEventListener('message', attached);
      host.contentWindow.postMessage({ type: 'chariox.app-host.attach', bundle, trust }, url.origin, [channel.port2]);
    };
    view.addEventListener('message', attached);
    stopServing = serveCalls(channel.port1, call);
    channel.port1.addEventListener('message', event => {
      if (event.data?.type === 'chariox.app-host.mounted') finish();
      else if (event.data?.type === 'chariox.app-host.failed') finish(new Error(event.data.reason));
    });
    container.append(host);
  });
  return { ready, close: () => { finish(new Error('App view closed')); stopServing(); channel.port1.close(); host.remove(); } };
}
