import { TextEncoder, TextDecoder } from './encoding.js';
import { AbortController, AbortSignal } from './abort.js';
import { createHandlers, ResourceScope } from './core.js';
import { createFetch, Headers, AppResponse } from './http.js';
import { createTimers } from './timers.js';
import registry from 'chariox:app/registry@1.0.0';
import state from 'chariox:app/state@1.0.0';
import schedule from 'chariox:app/schedule@1.0.0';
import events from 'chariox:app/events@1.0.0';
import connections from 'chariox:app/connections@1.0.0';
import files from 'chariox:app/files@1.0.0';
import http from 'chariox:app/http@1.0.0';
import log from 'chariox:app/log@1.0.0';
import host from 'chariox:app/host@1.0.0';
import validation from 'chariox:app/validation@1.0.0';
import monotonic from 'wasi:clocks/monotonic-clock@0.3.0';
import random from 'wasi:random/random@0.3.0';

Object.assign(globalThis, { TextEncoder, TextDecoder, AbortController, AbortSignal, Headers });
globalThis.queueMicrotask ??= callback => { Promise.resolve().then(callback); };
const timers = createTimers(monotonic, error => log.write('error', String(error?.message ?? error), '{}'));
globalThis.setTimeout = timers.setTimeout;
globalThis.clearTimeout = timers.clearTimeout;
globalThis.crypto = {
  getRandomValues(array) {
    if (!ArrayBuffer.isView(array) || array instanceof DataView || array instanceof Float32Array || array instanceof Float64Array) throw new TypeError('expected an integer typed array');
    if (array.byteLength > 65536) throw new RangeError('random request exceeds 65536 bytes');
    new Uint8Array(array.buffer, array.byteOffset, array.byteLength).set(random.getRandomBytes(BigInt(array.byteLength)));
    return array;
  },
};
Math.random = () => {
  const values = crypto.getRandomValues(new Uint32Array(2));
  return ((values[0] >>> 5) * 67108864 + (values[1] >>> 6)) / 9007199254740992;
};
globalThis.console = Object.fromEntries(['debug', 'info', 'log', 'warn', 'error'].map(level => [level, (...values) => {
  log.write(level === 'log' ? 'info' : level, values.map(value => typeof value === 'string' ? value : String(value)).join(' '), '{}');
}]));
export const fetch = createFetch(http);
globalThis.fetch = fetch;

export { registry, state, schedule, events, connections, files, http, log, host, validation, ResourceScope, Headers, AppResponse };

export function defineApp(app) {
  return createHandlers(app, { registry, finishInvocation: timers.finishInvocation, context: resources => ({
    fetch: createFetch(http, resources),
    async subscribe(name) {
      resources.signal.throwIfAborted();
      return resources.own(events.subscribe(name), stream => stream.return());
    },
  }) });
}
