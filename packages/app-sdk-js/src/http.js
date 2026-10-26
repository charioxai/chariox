import { ResourceScope } from './core.js';

export class Headers {
  #entries = [];
  constructor(init = []) {
    const entries = init instanceof Headers ? init.entries()
      : Array.isArray(init) || init?.[Symbol.iterator] ? init : Object.entries(init);
    for (const [name, value] of entries) this.append(name, value);
  }
  #name(name) {
    const normalized = String(name).toLowerCase();
    if (!/^[!#$%&'*+.^_`|~0-9a-z-]+$/.test(normalized)) throw new TypeError('invalid header name');
    return normalized;
  }
  append(name, value) {
    name = this.#name(name); value = String(value).trim();
    if (/[\r\n\0]/.test(value)) throw new TypeError('invalid header value');
    this.#entries.push([name, value]);
  }
  get(name) { const values = this.#entries.filter(([key]) => key === this.#name(name)); return values.length ? values.map(([, value]) => value).join(', ') : null; }
  has(name) { return this.get(name) !== null; }
  delete(name) { name = this.#name(name); this.#entries = this.#entries.filter(([key]) => key !== name); }
  set(name, value) { this.delete(name); this.append(name, value); }
  *entries() { yield* this.#entries; }
  [Symbol.iterator]() { return this.entries(); }
}

/** A bounded fetch response. Streaming consumers use body's async iterator. */
export class AppResponse {
  #tuple;
  #scope;
  bodyUsed = false;
  constructor(tuple, scope) {
    this.#tuple = tuple; this.#scope = scope;
    const [head] = tuple;
    this.status = head.status; this.url = head.url;
    this.ok = head.status >= 200 && head.status < 300;
    this.headers = new Headers(head.headers);
    this.body = {
      [Symbol.asyncIterator]: () => this.#consume(),
      cancel: () => scope.release(tuple),
    };
  }
  async *#consume() {
    if (this.bodyUsed) throw new TypeError('response body has already been consumed');
    this.bodyUsed = true;
    try {
      for await (const chunk of this.#tuple[1]) {
        this.#scope.signal.throwIfAborted();
        yield chunk;
      }
    } finally { await this.#scope.release(this.#tuple); }
  }
  async bytes({ limit = 1024 * 1024 } = {}) {
    if (!Number.isSafeInteger(limit) || limit < 0) throw new RangeError('invalid response byte limit');
    const chunks = []; let size = 0;
    for await (const chunk of this.body) {
      size += chunk.length;
      if (size > limit) throw { code: 'APP_HTTP_LIMIT', message: `response exceeds ${limit} bytes; consume body as a stream` };
      chunks.push(chunk);
    }
    const result = new Uint8Array(size); let offset = 0;
    for (const chunk of chunks) { result.set(chunk, offset); offset += chunk.length; }
    return result;
  }
  async arrayBuffer(options) { return (await this.bytes(options)).buffer; }
  async text(options) { return new TextDecoder().decode(await this.bytes(options)); }
  async json(options) { return JSON.parse(await this.text(options)); }
}

export function createFetch(http, invocationScope) {
  return async (url, init = {}) => {
    url = String(url);
    if (!/^https?:\/\//i.test(url)) throw new TypeError('fetch requires an absolute HTTP(S) URL');
    const scope = new ResourceScope();
    if (invocationScope) await invocationScope.own(Promise.resolve(scope), child => child.close());
    scope.signal.throwIfAborted();
    const abort = () => { scope.close(init.signal.reason).catch(() => {}); };
    init.signal?.throwIfAborted();
    init.signal?.addEventListener('abort', abort, { once: true });
    const body = init.body == null ? null : typeof init.body === 'string'
      ? new TextEncoder().encode(init.body) : init.body;
    const stream = body == null ? null : body instanceof Uint8Array
      ? (async function* () { yield body; })() : body;
    try {
      const tuple = await scope.own(http.fetch({
        url, method: init.method ?? 'GET', headers: [...new Headers(init.headers)],
        connectionId: init.connectionId ?? null, operationId: init.operationId ?? null,
      }, stream), async ([, responseBody]) => {
        try { await responseBody.return(); }
        finally { init.signal?.removeEventListener('abort', abort); }
      });
      return new AppResponse(tuple, scope);
    } catch (error) {
      init.signal?.removeEventListener('abort', abort);
      throw error;
    }
  };
}
