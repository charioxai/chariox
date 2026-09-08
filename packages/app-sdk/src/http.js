import { AppError } from './errors.js';
import { object } from './protocol.js';

const CHUNK = 64 * 1024;
const BUFFERED = 512 * 1024;
const invalid = message => new AppError('INVALID_ARGUMENT', message);
const id = value => {
  if (typeof value !== 'string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u.test(value)) {
    throw invalid('Expected an opaque HTTP stream identity');
  }
  return value;
};
function data(value, limit) {
  if (typeof value !== 'string' && !(value instanceof Uint8Array)) throw invalid('Expected HTTP text or bytes');
  const bytes = typeof value === 'string' ? Buffer.from(value) : Buffer.from(value.buffer, value.byteOffset, value.byteLength);
  if (bytes.length > limit) throw invalid('HTTP body exceeds its byte limit');
  return bytes;
}
function normalize(request, buffered) {
  if (!object(request) || Object.keys(request).some(key => !['url', 'method', 'headers', 'connectionId', 'operationId', buffered ? 'body' : 'hasBody'].includes(key))) {
    throw invalid('Invalid HTTP request fields');
  }
  let url;
  try { url = new URL(request.url); } catch { throw invalid('Invalid HTTP URL'); }
  if (url.protocol !== 'https:' || url.username || url.password || url.hash || Buffer.byteLength(url.href) > 8192) {
    throw invalid('Expected an HTTPS URL without credentials or fragment');
  }
  const method = request.method ?? 'GET';
  if (!['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS'].includes(method)) throw invalid('Unsupported HTTP method');
  const headers = request.headers ?? [];
  if (!Array.isArray(headers) || headers.length > 64) throw invalid('Invalid HTTP headers');
  let total = 0;
  const copied = headers.map(pair => {
    if (!Array.isArray(pair) || pair.length !== 2 || pair.some(value => typeof value !== 'string')) throw invalid('Invalid HTTP header pair');
    total += Buffer.byteLength(pair[0]) + Buffer.byteLength(pair[1]);
    if (total > 16 * 1024) throw invalid('HTTP headers exceed their byte limit');
    return [...pair];
  });
  const hasBody = buffered ? request.body !== undefined : (request.hasBody ?? false);
  if (typeof hasBody !== 'boolean' || (hasBody && ['GET', 'HEAD'].includes(method))) throw invalid('Invalid HTTP body for method');
  const result = { url: url.href, method, headers: copied, hasBody };
  for (const field of ['connectionId', 'operationId']) {
    if (Object.hasOwn(request, field)) {
      const value = request[field];
      if (typeof value !== 'string' || !value || Buffer.byteLength(value) > 128 || /[\s\p{Cc}]/u.test(value)) throw invalid(`Invalid ${field}`);
      result[field] = value;
    }
  }
  return result;
}

/** Streaming transport, not Fetch: redirects, decompression, credentials and
 * protected effects are not implemented here. Body operations never retry. */
export function createHttp(call) {
  const writers = new Set();
  const readers = new Set();
  function exclusive(set, streamId, run) {
    id(streamId);
    if (set.has(streamId)) return Promise.reject(new AppError('APP_BUSY', 'An HTTP stream operation is already in flight'));
    set.add(streamId);
    return Promise.resolve().then(run).finally(() => set.delete(streamId));
  }
  const http = {
    open(request, options) { return call('http.open', normalize(request, false), options); },
    write(streamId, contents, end = false, options) {
      const bytes = data(contents, CHUNK);
      if (typeof end !== 'boolean' || (!bytes.length && !end)) throw invalid('Invalid HTTP body chunk');
      // Encode before yielding so mutation of the caller's buffer cannot alter
      // an already submitted body chunk or reorder concurrent writes.
      const bodyBase64 = bytes.toString('base64');
      return exclusive(writers, streamId, () => call('http.write', { streamId, bodyBase64, end }, options));
    },
    headers(streamId, options) {
      return exclusive(readers, streamId, () => call('http.headers', { streamId }, options));
    },
    read(streamId, options) {
      return exclusive(readers, streamId, () => call('http.read', { streamId }, options));
    },
    cancel(streamId, options) { return call('http.cancel', { streamId: id(streamId) }, options); },
    request(request, options = {}) {
      const parameters = normalize(request, true);
      const body = request.body === undefined ? undefined : Buffer.from(data(request.body, BUFFERED));
      const timeoutMs = options.timeoutMs ?? 30_000;
      if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 30_000) throw invalid('Invalid HTTP request timeout');
      return buffered(parameters, body, options.signal, timeoutMs);
    },
  };
  async function buffered(parameters, body, signal, timeoutMs) {
    const controller = new AbortController();
    const abort = () => controller.abort();
    signal?.addEventListener('abort', abort, { once: true });
    if (signal?.aborted) abort();
    const deadline = performance.now() + timeoutMs;
    const options = () => {
      const remaining = Math.floor(deadline - performance.now());
      if (remaining < 1) throw new AppError('DEADLINE_EXCEEDED', 'HTTP request deadline exceeded');
      return { signal: controller.signal, timeoutMs: remaining };
    };
    let streamId;
    try {
      ({ streamId } = await call('http.open', parameters, options()));
      id(streamId);
      const upload = async () => {
        if (body === undefined) return;
        if (body.length === 0) { await http.write(streamId, body, true, options()); return; }
        for (let offset = 0; offset < body.length; offset += CHUNK) {
          const chunk = body.subarray(offset, offset + CHUNK);
          await http.write(streamId, chunk, offset + chunk.length === body.length, options());
        }
      };
      const download = async () => {
        let head;
        do { head = await http.headers(streamId, options()); } while (head.pending);
        const chunks = [];
        let total = 0;
        for (;;) {
          const part = await http.read(streamId, options());
          if (part.pending) continue;
          if (part.done) break;
          if (typeof part.chunkBase64 !== 'string' || part.chunkBase64.length > ((CHUNK + 2) / 3 | 0) * 4) throw new AppError('PROTOCOL_ERROR', 'Invalid HTTP response chunk');
          const chunk = Buffer.from(part.chunkBase64, 'base64');
          if (chunk.length > CHUNK || chunk.toString('base64') !== part.chunkBase64) throw new AppError('PROTOCOL_ERROR', 'Invalid HTTP response encoding');
          total += chunk.length;
          if (total > BUFFERED) throw new AppError('APP_HTTP_LIMIT', 'Buffered HTTP response exceeds 512 KiB');
          chunks.push(chunk);
        }
        return { status: head.status, headers: head.headers, url: head.url, bodyBase64: Buffer.concat(chunks, total).toString('base64') };
      };
      const outcomes = await Promise.all([upload(), download()]);
      return outcomes[1];
    } finally {
      // Cancellation and cleanup are never sent with the already-aborted signal.
      // This does not retry any body operation, even if its completion was lost.
      controller.abort();
      signal?.removeEventListener('abort', abort);
      if (streamId) await http.cancel(streamId, { timeoutMs: 1000 }).catch(() => {});
    }
  }
  return Object.freeze(http);
}
