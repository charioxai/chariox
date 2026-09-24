// Worker-global Fetch adaptation. Native Web value objects remain available;
// every network effect is routed through the one authenticated SDK peer.
import { FETCH_LIFETIME, requestValue, redirect, unsupported } from './fetch-request.js';
import { responseValue } from './fetch-response.js';
import { Transfer } from './fetch-transfer.js';

export function createFetch(http) {
  return async function fetch(input, init) {
    let value = requestValue(input, init);
    const deadline = performance.now() + FETCH_LIFETIME;
    let owner;
    try {
      for (let count = 0;; count++) {
        owner = new Transfer(http, value.request.signal, deadline);
        await owner.open(value);
        owner.startUpload(value.request.body, value.expectedBytes);
        const head = await owner.headers();
        const location = new Headers(head.headers).get('location');
        if (![301, 302, 303, 307, 308].includes(head.status) || location === null
          || value.request.redirect === 'manual') {
          return await responseValue(owner, head, value.method, count > 0);
        }
        if (value.request.redirect === 'error') throw unsupported('redirect mode is error');
        const next = redirect(value, head.status, location, count);
        await owner.stop();
        value = next;
      }
    } catch (error) {
      const failure = owner?.failure ?? error;
      await owner?.stop(failure);
      if (value.request.signal.aborted) throw value.request.signal.reason;
      if (failure instanceof TypeError || failure?.name === 'AbortError' || failure?.name === 'TimeoutError') throw failure;
      throw new TypeError('Chariox Fetch network request failed', { cause: failure });
    }
  };
}
