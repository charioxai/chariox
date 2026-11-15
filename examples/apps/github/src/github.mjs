// Calls to the GitHub REST API as one of the owner's accounts. The App never holds a credential: it names
// `account:<label>` and the kernel adds the token and removes it from the answer. This file adds what the
// kernel does not: conditional requests (an unchanged answer costs no rate limit), pagination and the
// App's own view of each account's rate limit.
export const ORIGIN = 'https://api.github.com';
/** Calls stop before the limit is used up, so the owner's other tools keep working. */
export const RESERVE = { core: 100, search: 2 };
const CACHE_SLOTS = 48;
/** An answer larger than this is not kept for conditional requests. */
const CACHE_VALUE_LIMIT = 120 * 1024;
const API_VERSION = '2022-11-28';

const fail = (code, message, extra) => { throw { code, message, retryable: false, ...extra }; };
const text = value => String(value ?? '');

const fnv = input => {
  let hash = 0x811c9dc5;
  for (let i = 0; i < input.length; i++) { hash ^= input.charCodeAt(i); hash = Math.imul(hash, 0x01000193) >>> 0; }
  return hash;
};

export const expand = (template, params = {}) => template.replace(/\{(\w+)\}/g, (_, name) => {
  if (params[name] === undefined || params[name] === '') fail('INVALID_ARGUMENT', `missing ${name}`);
  return encodeURIComponent(text(params[name]));
});

export const queryString = query => {
  const pairs = Object.entries(query ?? {}).filter(([, value]) => value !== undefined && value !== null && value !== '');
  return pairs.length ? '?' + pairs.map(([key, value]) => `${encodeURIComponent(key)}=${encodeURIComponent(text(value))}`).join('&') : '';
};

export function parseLink(header) {
  const links = {};
  for (const part of text(header).split(',')) {
    const match = /<([^>]+)>\s*;\s*rel="([^"]+)"/.exec(part);
    if (match) links[match[2]] = match[1];
  }
  return links;
}

/** What a kernel refusal or GitHub answer means for the person, in the App's own codes. */
const kernelRefusals = {
  APP_HTTP_ACCOUNT_UNKNOWN: ['ACCOUNT_NOT_BOUND', account => `No GitHub account named "${account}" is bound to this App. The owner binds one in the kernel.`],
  APP_HTTP_ACCOUNT_NOT_ALLOWED: ['ACCOUNT_NOT_ALLOWED', account => `The account "${account}" is not allowed to do this. The owner can raise what it may do in the kernel.`],
  APP_HTTP_ACCOUNT_UNAVAILABLE: ['ACCOUNT_UNAVAILABLE', account => `The credential of the account "${account}" cannot be read: unlock the kernel's Vault.`],
  APP_HTTP_ROUTE_NOT_DECLARED: ['ROUTE_NOT_DECLARED', () => 'This App has not declared that GitHub call.'],
};

export function createGitHub({ state, fetch, now = () => Date.now() }) {
  const rateKey = (account, resource) => `rate:${account}:${resource}`;
  const slotOf = (account, url, accept) => fnv(`${account}\n${accept}\n${url}`) % CACHE_SLOTS;

  async function readJson(key) {
    const record = await state.get(key);
    return record ? { value: JSON.parse(record.value), version: record.version } : null;
  }

  async function put(key, value, previous) {
    try {
      await state.transact({
        schemaVersion: 0, checks: [{ key, version: previous?.version ?? null }],
        writes: [{ tag: 'put', val: [key, JSON.stringify(value)] }], occurrences: [], wakes: [],
      });
    } catch { /* a cache and a rate counter: a concurrent writer's value is as good */ }
  }

  async function readCache(account, url, accept) {
    const entry = await readJson(`cache:${slotOf(account, url, accept)}`);
    return entry && entry.value.url === url && entry.value.account === account && entry.value.accept === accept ? entry : null;
  }

  async function rate(account, resource) {
    return (await readJson(rateKey(account, resource)))?.value ?? null;
  }

  async function noteRate(account, headers) {
    const remaining = headers.get('x-ratelimit-remaining');
    if (remaining === null) return;
    const resource = headers.get('x-ratelimit-resource') === 'search' ? 'search' : 'core';
    const key = rateKey(account, resource);
    await put(key, {
      limit: Number(headers.get('x-ratelimit-limit')), remaining: Number(remaining),
      reset: Number(headers.get('x-ratelimit-reset')), at: now(),
    }, await readJson(key));
  }

  const limited = (resource, limit) => fail('RATE_LIMITED',
    `The GitHub ${resource} rate limit is used up until ${new Date(limit.reset * 1000).toISOString()}.`,
    { resetAt: limit.reset * 1000, resource });

  /** One GET. Answers `{data, headers, link, stale, cached}`; `data` is the parsed JSON or, for a diff, text. */
  async function get(account, template, params, { query, accept = 'application/vnd.github+json', fresh = false } = {}) {
    const url = ORIGIN + expand(template, params) + queryString(query);
    const resource = template.startsWith('/search/') ? 'search' : 'core';
    const cached = fresh ? null : await readCache(account, url, accept);
    const limit = await rate(account, resource);
    if (template !== '/rate_limit' && limit && limit.remaining < RESERVE[resource] && limit.reset * 1000 > now()) {
      if (cached) return answer(cached.value, { stale: true });
      limited(resource, limit);
    }
    const headers = { accept, 'x-github-api-version': API_VERSION };
    if (cached?.value.etag) headers['if-none-match'] = cached.value.etag;
    let response;
    try {
      response = await fetch(url, { method: 'GET', headers, connectionId: `account:${account}` });
    } catch (error) {
      const code = (error?.payload ?? error)?.code;
      const refusal = kernelRefusals[code];
      if (refusal) fail(refusal[0], refusal[1](account));
      throw error;
    }
    if (response.status === 304 && cached) return answer(cached.value, { cached: true });
    const body = await response.text();
    if (response.status === 200 || response.status === 201) {
      await noteRate(account, response.headers);
      const etag = response.headers.get('etag');
      const entry = { url, account, accept, etag, body, link: response.headers.get('link') };
      if (etag && body.length <= CACHE_VALUE_LIMIT) await put(`cache:${slotOf(account, url, accept)}`, entry, cached);
      return answer(entry);
    }
    await noteRate(account, response.headers);
    return refuse(account, resource, response, body);
  }

  function answer(entry, { stale = false, cached = false } = {}) {
    return {
      data: entry.accept.includes('json') ? (entry.body ? JSON.parse(entry.body) : null) : entry.body,
      link: parseLink(entry.link), stale, cached,
    };
  }

  function refuse(account, resource, response, body) {
    let message = '';
    try { message = JSON.parse(body).message ?? ''; } catch { /* not JSON */ }
    const status = response.status;
    const remaining = response.headers.get('x-ratelimit-remaining');
    const retryAfter = response.headers.get('retry-after');
    if ((status === 403 || status === 429) && (remaining === '0' || retryAfter || /rate limit/i.test(message))) {
      const reset = retryAfter ? Math.floor(now() / 1000) + Number(retryAfter) : Number(response.headers.get('x-ratelimit-reset') ?? 0);
      limited(resource, { reset });
    }
    if (status === 401) fail('ACCOUNT_UNAUTHORIZED', `GitHub did not accept the account "${account}". Its token may have expired or been revoked.`, { status });
    if (status === 404) fail('NOT_FOUND', `Not found, or the account "${account}" cannot see it.`, { status, github: message });
    if (status === 403) fail('FORBIDDEN', message || `The account "${account}" may not do this.`, { status });
    if (status === 406 || status === 422) fail('UNPROCESSABLE', message || 'GitHub could not produce this answer.', { status });
    fail('GITHUB_ERROR', `GitHub answered ${status}${message ? `: ${message}` : ''}.`, { status });
  }

  /** A list: the page asked for, and whether another follows. */
  async function list(account, template, params, { query = {}, page = 1, perPage = 30, ...options } = {}) {
    const result = await get(account, template, params, { query: { ...query, per_page: perPage, page }, ...options });
    return { items: result.data, page, hasNext: Boolean(result.link.next), stale: result.stale };
  }

  return { get, list, rate };
}
