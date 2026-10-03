import { createAppSdk } from '../src/index.js';
import { fakeTransport, generation, envelope, response, deferred } from './helpers.js';

// Real SDK peer envelopes, with a fixed trusted broker fixture. No DNS/socket
// or production destination grant is introduced by this client conformance test.
export function fixture(route) {
  const transport = fakeTransport();
  const streams = new Map();
  let sequence = 0;
  const send = transport.send.bind(transport);
  async function dispatch(message) {
    const params = message.params;
    if (message.method === 'http.open') {
      const id = `00000000-0000-4000-8000-${String(++sequence).padStart(12, '0')}`;
      const config = await route(params, sequence);
      streams.set(id, { id, params, config, ended: deferred(), writes: [], read: 0, cancelled: 0 });
      return { streamId: id };
    }
    const stream = streams.get(params.streamId);
    if (!stream) throw new Error('unknown fixture stream');
    const { config } = stream;
    switch (message.method) {
      case 'http.write': {
        const bytes = Buffer.from(params.bodyBase64, 'base64');
        stream.writes.push(bytes);
        if (config.write) await config.write(stream, bytes, params.end);
        if (params.end) stream.ended.resolve();
        return { bytesWritten: bytes.length };
      }
      case 'http.headers':
        if (config.waitUpload) await stream.ended.promise;
        if (config.headersWait) await config.headersWait;
        return { pending: false, status: config.status ?? 200, headers: config.headers ?? [], url: stream.params.url };
      case 'http.read': {
        if (config.read) return config.read(stream);
        const part = (config.chunks ?? [])[stream.read++];
        return part === undefined ? { pending: false, done: true, chunkBase64: '' }
          : { pending: false, done: false, chunkBase64: Buffer.from(part).toString('base64') };
      }
      case 'http.cancel': stream.cancelled++; return null;
      default: throw new Error(`unexpected method ${message.method}`);
    }
  }
  transport.send = message => {
    send(message);
    if (message.kind !== 'request') return;
    Promise.resolve().then(() => dispatch(message)).then(
      value => { if (!transport.closed) transport.receive(response(message.id, value)); },
      error => { if (!transport.closed) transport.receive(envelope({ kind: 'response', id: message.id,
        error: { code: 'FIXTURE_DENIED', message: 'bounded broker rejection', retryable: false } })); },
    );
  };
  const sdk = createAppSdk({ transport, generation, paths: { package: '/package', data: '/data', temporary: '/temporary' } });
  return { sdk, transport, streams, fetch: sdk.http.fetch, close: () => sdk.close() };
}
