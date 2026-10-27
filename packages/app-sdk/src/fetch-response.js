import { DOWNLOAD_LIMIT, unsupported } from './fetch-request.js';

export function limitBytes(limit = DOWNLOAD_LIMIT) {
  let total = 0;
  return new TransformStream({
    transform(chunk, controller) {
      if (!(chunk instanceof Uint8Array)) throw unsupported('response stream must contain bytes');
      total += chunk.byteLength;
      if (total > limit) throw unsupported('decoded response exceeds its byte limit');
      controller.enqueue(chunk);
    },
  }, { highWaterMark: 1 }, { highWaterMark: 0 });
}

function decoded(raw, headers) {
  const encoding = headers.get('content-encoding');
  if (!encoding) return raw.pipeThrough(limitBytes());
  const formats = { gzip: 'gzip', 'x-gzip': 'gzip', deflate: 'deflate', br: 'brotli', identity: null };
  const encodings = encoding.split(',').map(value => value.trim().toLowerCase());
  if (encodings.length > 3 || encodings.some(value => !Object.hasOwn(formats, value))) throw unsupported('unsupported content encoding');
  let stream = raw;
  for (const encoding of encodings.reverse()) {
    if (formats[encoding]) stream = stream.pipeThrough(new DecompressionStream(formats[encoding]));
    stream = stream.pipeThrough(limitBytes());
  }
  // These fields describe the encoded representation, as in native fetch;
  // callers must not use content-length to allocate the decoded response.
  return stream;
}

function decorate(response, url, redirected) {
  const clone = response.clone.bind(response);
  Object.defineProperties(response, {
    url: { value: url }, redirected: { value: redirected }, type: { value: 'basic' },
    clone: { value: () => decorate(clone(), url, redirected) },
  });
  for (const name of ['append', 'set', 'delete']) {
    Object.defineProperty(response.headers, name, { value: () => { throw new TypeError('Headers are immutable'); } });
  }
  return response;
}

export async function responseValue(owner, head, method, redirected) {
  const headers = new Headers(head.headers);
  if (method === 'HEAD' || [204, 205, 304].includes(head.status)) {
    await owner.stop();
    return decorate(new Response(null, { status: head.status, headers }), head.url, redirected);
  }
  const stream = decoded(owner.rawBody(), headers);
  const reader = stream.getReader();
  const body = new ReadableStream({
    start(controller) { owner.output = controller; },
    async pull(controller) {
      try {
        owner.options();
        const part = await reader.read();
        owner.options();
        if (part.done) { controller.close(); await owner.stop(); }
        else controller.enqueue(part.value);
      } catch (error) {
        try { controller.error(error); } catch { /* earlier abort already settled it */ }
        void reader.cancel(error).catch(() => {});
        await owner.stop(error);
      }
    },
    async cancel(reason) {
      void reader.cancel(reason).catch(() => {});
      await owner.stop(reason);
    },
  }, { highWaterMark: 0 });
  return decorate(new Response(body, { status: head.status, headers }), head.url, redirected);
}
