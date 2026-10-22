// Today's runtime: the same tools as a plain JS App on the real SDK.
import { TOOLS, transform, compute, spin, hog } from './tools.js';

export default function register(chariox) {
  const handlers = {
    echo: input => input,
    async counter() {
      const current = await chariox.state.get('count');
      const count = current ? current.value : 0;
      const { revision } = await chariox.state.transaction({
        schemaVersion: 0,
        checks: [{ key: 'count', version: current ? current.version : null }],
        writes: [{ key: 'count', value: count + 1 }],
      });
      return { count: count + 1, revision };
    },
    async fetch({ url }) {
      const response = await chariox.http.request({ url, method: 'GET', headers: [['accept', 'text/plain']] });
      const body = Buffer.from(response.bodyBase64, 'base64');
      return { status: response.status, bytes: body.length, head: body.subarray(0, 16).toString() };
    },
    transform: ({ n, rounds }) => transform(n, rounds),
    compute: ({ n }) => compute(n),
    spin: () => spin(),
    hog: ({ mb, kind }) => hog(mb, kind),
    // Shared by every concurrent caller, as in the async wasm Apps.
    async shared({ url }) {
      const n = ++shared.counter;
      shared.connection ??= { id: `conn-${n}`, requests: 0 };
      const connection = shared.connection;
      const sequence = ++connection.requests;
      const response = await chariox.http.request({ url, method: 'GET', headers: [['accept', 'text/plain']] });
      shared.cache.set(n, Buffer.from(response.bodyBase64, 'base64').length);
      return { n, sequence, connection: connection.id, cached: shared.cache.size };
    },
  };
  const shared = { counter: 0, cache: new Map(), connection: null };
  for (const name of [...TOOLS, 'shared']) chariox.tools.register(name, handlers[name]);
}
