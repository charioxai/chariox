// Request value normalization and replay sources. Never inspect Undici private
// slots or tee an arbitrary stream into an unbounded redirect replay buffer.
export const UPLOAD_LIMIT = 64 * 1024 * 1024;
export const DOWNLOAD_LIMIT = 256 * 1024 * 1024;
export const FETCH_LIFETIME = 60 * 60 * 1000;
export const CHUNK_BYTES = 64 * 1024;
export const unsupported = message => new TypeError(`Chariox Fetch: ${message}`);

function snapshot(body) {
  if (body === undefined || body === null) return { source: null, replayable: true };
  if (typeof body === 'string') {
    if (Buffer.byteLength(body) > UPLOAD_LIMIT) throw unsupported('request body exceeds 64 MiB');
    return { source: body, replayable: true };
  }
  if (body instanceof URLSearchParams) {
    if (Buffer.byteLength(body.toString()) > UPLOAD_LIMIT) throw unsupported('request body exceeds 64 MiB');
    return { source: new URLSearchParams(body), replayable: true };
  }
  if (body instanceof Blob) {
    if (body.size > UPLOAD_LIMIT) throw unsupported('request body exceeds 64 MiB');
    return { source: body, replayable: true };
  }
  if (ArrayBuffer.isView(body) || body instanceof ArrayBuffer) {
    const bytes = ArrayBuffer.isView(body)
      ? new Uint8Array(body.buffer, body.byteOffset, body.byteLength) : new Uint8Array(body);
    if (bytes.length > UPLOAD_LIMIT) throw unsupported('request body exceeds 64 MiB');
    return { source: bytes.slice(), replayable: true };
  }
  if (body instanceof FormData) {
    let bytes = 0;
    let fields = 0;
    const copy = new FormData();
    for (const [name, value] of body) {
      bytes += Buffer.byteLength(name) + (typeof value === 'string' ? Buffer.byteLength(value) : value.size);
      if (++fields > 1024 || bytes > UPLOAD_LIMIT) throw unsupported('multipart body exceeds its bounds');
      copy.append(name, value);
    }
    return { source: copy, replayable: true, multipart: true };
  }
  return { source: body, replayable: false };
}

export function requestValue(input, init = {}) {
  for (const name of ['dispatcher', 'agent', 'connectionId', 'operationId']) {
    if (Object.hasOwn(init, name)) throw unsupported(`${name} cannot select network authority`);
  }
  const hasBody = Object.hasOwn(init, 'body') && init.body !== undefined && init.body !== null;
  const retained = hasBody ? snapshot(init.body) : { source: null, replayable: !(input instanceof Request && input.body) };
  const options = hasBody ? { ...init, body: retained.source } : init;
  const request = new Request(input, options);
  if (request.credentials === 'include') throw unsupported('credential inclusion requires an opaque App connection');
  if (request.integrity) throw unsupported('subresource integrity is not supported');
  if (request.keepalive) throw unsupported('keepalive cannot outlive the App worker');
  if (!['default', 'no-store', 'no-cache', 'reload'].includes(request.cache)) throw unsupported('HTTP cache storage is not supported');
  const url = new URL(request.url);
  if (url.protocol !== 'https:' || url.username || url.password) throw unsupported('only anonymous HTTPS URLs are supported');
  url.hash = '';
  const headers = new Headers(request.headers);
  const length = headers.get('content-length');
  let expectedBytes;
  if (length !== null) {
    if (!/^(0|[1-9][0-9]*)$/u.test(length) || Number(length) > UPLOAD_LIMIT) throw unsupported('invalid content-length');
    expectedBytes = Number(length);
    if (!request.body && expectedBytes !== 0) throw unsupported('content-length does not match the empty body');
    // Framing remains kernel-owned. Verify the caller's length while streaming,
    // never send a caller-selected framing header through the broker.
    headers.delete('content-length');
  }
  if (!headers.has('accept')) headers.set('accept', '*/*');
  if (!headers.has('accept-encoding')) headers.set('accept-encoding', 'gzip, deflate, br');
  return { request, url, method: request.method, headers, retained, expectedBytes,
    // A new multipart Request chooses a new boundary on replay. Only remove an
    // automatically generated content type; preserve an explicit caller value.
    generatedMultipart: retained.multipart && !new Headers(init.headers).has('content-type'),
  };
}

export function redirect(value, status, location, count) {
  if (count >= 20) throw unsupported('redirect limit exceeded');
  const url = new URL(location, value.url);
  if (url.protocol !== 'https:' || url.username || url.password) throw unsupported('redirect destination is not anonymous HTTPS');
  url.hash = '';
  // The Fetch redirect algorithm rejects non-replayable bodies except for303.
  if (status !== 303 && value.request.body && !value.retained.replayable) throw unsupported('redirect requires a replayable request body');
  const dropBody = ((status === 301 || status === 302) && value.method === 'POST')
    || (status === 303 && !['GET', 'HEAD'].includes(value.method));
  const headers = new Headers(value.headers);
  if (dropBody) {
    for (const name of ['content-encoding', 'content-language', 'content-location', 'content-type']) headers.delete(name);
  } else if (value.generatedMultipart) headers.delete('content-type');
  if (url.origin !== value.url.origin) headers.delete('authorization');
  const body = dropBody ? null : value.retained.source;
  const method = dropBody ? 'GET' : value.method;
  const request = new Request(url, { method, headers, body, signal: value.request.signal,
    redirect: value.request.redirect, duplex: 'half' });
  return { ...value, request, url, method, headers: new Headers(request.headers),
    expectedBytes: dropBody ? undefined : value.expectedBytes,
    retained: dropBody ? { source: null, replayable: true } : value.retained };
}
