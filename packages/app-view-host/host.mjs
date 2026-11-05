import { verifyView } from './bundle.mjs';
import { PERMISSIONS, SANDBOX, renderAppDocument } from './document.mjs';

// The host has no conversation data or network connection. Only its embedding
// client may establish the single private port. App window messages are ignored.
let attached = false;
addEventListener('message', async event => {
  if (attached || event.source !== parent || event.source === window || event.origin !== location.origin
      || event.data?.type !== 'chariox.app-host.attach' || event.ports.length !== 1) return;
  attached = true;
  const port = event.ports[0];
  try {
    const source = await verifyView(event.data.bundle, event.data.trust);
    const frame = document.createElement('iframe');
    frame.title = 'App'; frame.sandbox = SANDBOX; frame.allow = PERMISSIONS; frame.referrerPolicy = 'no-referrer';
    frame.srcdoc = await renderAppDocument(source);
    document.body.append(frame);
    port.postMessage({ type: 'chariox.app-host.mounted' });
  } catch (error) {
    port.postMessage({ type: 'chariox.app-host.failed', reason: error instanceof Error ? error.message : 'App view verification failed' });
  } finally { port.close(); }
});
parent.postMessage({ type: 'chariox.app-host.ready' }, location.origin);
