// Private cookie payload conversion. Never log the returned cookies.
// This module grants no import authority and performs no browser or network I/O.
export function prepareChromeCookieBatch(source, scope) {
  if (!Array.isArray(source) || source.length > 512 || !scope
      || !Array.isArray(scope.approvedDomains) || scope.approvedDomains.length > 32
      || typeof scope.sourceStoreId !== 'string' || !scope.sourceStoreId) fail('invalid_cookie_import');
  const now = scope.nowSeconds ?? Date.now() / 1000;
  if (!Number.isFinite(now) || now < 0) fail('invalid_cookie_import');
  const approved = new Set(scope.approvedDomains.map(hostname));
  const partitionSites = scope.approvedPartitionSites ?? [];
  if (!Array.isArray(partitionSites) || partitionSites.length > 32) fail('invalid_cookie_import');
  const approvedPartitions = new Set(partitionSites.map(partitionSite));
  const keys = new Set();
  let bytes = 0;
  for (const cookie of source) {
    validateCookie(cookie, now);
    if (!approved.has(hostname(cookie.domain.replace(/^\./, '')))) fail('cookie_scope_denied');
    if (cookie.storeId !== scope.sourceStoreId) fail('cookie_store_mismatch');
    if (cookie.partitionKey !== undefined) {
      const key = cookie.partitionKey;
      if (!key || typeof key.hasCrossSiteAncestor !== 'boolean' || !cookie.secure
          || Object.keys(key).some(k => !['topLevelSite', 'hasCrossSiteAncestor'].includes(k))) fail('unsupported_cookie_partition');
      if (!approvedPartitions.has(partitionSite(key.topLevelSite))) fail('cookie_scope_denied');
    }
    const key = JSON.stringify([cookie.name, cookie.hostOnly,
      hostname(cookie.domain.replace(/^\./, '')), cookie.path,
      cookie.partitionKey?.topLevelSite ?? null, cookie.partitionKey?.hasCrossSiteAncestor ?? null]);
    if (keys.has(key)) fail('duplicate_cookie_import');
    keys.add(key);
    bytes += new TextEncoder().encode(JSON.stringify(cookie)).byteLength;
    if (bytes > 524288) fail('cookie_import_too_large');
  }
  const cookies = source.map(cookie => ({
    name: cookie.name,
    value: cookie.value,
    url: `${cookie.secure ? 'https' : 'http'}://${hostname(cookie.domain.replace(/^\./, ''))}/`,
    ...(!cookie.hostOnly ? { domain: `.${hostname(cookie.domain.replace(/^\./, ''))}` } : {}),
    path: cookie.path,
    httpOnly: cookie.httpOnly,
    secure: cookie.secure,
    ...(cookie.sameSite !== 'unspecified' ? {
      sameSite: { lax: 'Lax', strict: 'Strict', no_restriction: 'None' }[cookie.sameSite],
    } : {}),
    ...(!cookie.session ? { expires: cookie.expirationDate } : {}),
    ...(cookie.partitionKey !== undefined ? { partitionKey: {
      topLevelSite: cookie.partitionKey.topLevelSite,
      hasCrossSiteAncestor: cookie.partitionKey.hasCrossSiteAncestor,
    } } : {}),
  }));
  return { cookies, summary: { cookieCount: cookies.length,
    domains: [...new Set(source.map(c => hostname(c.domain.replace(/^\./, ''))))].sort() } };
}

function partitionSite(value) {
  try {
    const url = new URL(value);
    if (!['https:', 'http:'].includes(url.protocol) || url.origin !== value) fail('unsupported_cookie_partition');
    return url.origin;
  } catch { fail('unsupported_cookie_partition'); }
}

function validateCookie(cookie, now) {
  if (!cookie || typeof cookie !== 'object' || Array.isArray(cookie)) fail('invalid_cookie_import');
  if (Object.keys(cookie).some(key => ![
    'name', 'value', 'domain', 'path', 'hostOnly', 'httpOnly', 'secure',
    'session', 'expirationDate', 'sameSite', 'storeId', 'partitionKey',
  ].includes(key))) fail('unsupported_cookie_fields');
  if (typeof cookie.name !== 'string' || /[^!#$%&'*+\-.^_`|~0-9a-z]/i.test(cookie.name)
      || typeof cookie.value !== 'string' || /[\x00-\x1f\x7f;]/.test(cookie.value)
      || cookie.name.length + cookie.value.length > 4096
      || new TextEncoder().encode(cookie.name + cookie.value).byteLength > 4096
      || typeof cookie.domain !== 'string' || cookie.domain.length > 254
      || typeof cookie.path !== 'string' || !cookie.path.startsWith('/')
      || cookie.path.length > 1024 || /[\x00-\x1f\x7f;]/.test(cookie.path)
      || !['hostOnly', 'httpOnly', 'secure', 'session'].every(k => typeof cookie[k] === 'boolean')
      || !['lax', 'strict', 'no_restriction', 'unspecified'].includes(cookie.sameSite)) fail('invalid_cookie_import');
  if (cookie.hostOnly && cookie.domain.startsWith('.')) fail('invalid_cookie_import');
  if (cookie.session ? cookie.expirationDate !== undefined
      : !Number.isFinite(cookie.expirationDate) || cookie.expirationDate <= now) fail('invalid_cookie_expiry');
  if ((cookie.sameSite === 'no_restriction' || cookie.name.startsWith('__Secure-') || cookie.name.startsWith('__Host-'))
      && !cookie.secure) fail('invalid_cookie_import');
  if (cookie.name.startsWith('__Host-') && (!cookie.hostOnly || cookie.path !== '/')) fail('invalid_cookie_import');
}

function hostname(value) {
  if (typeof value !== 'string' || !/^[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/i.test(value)
      || value.length > 253 || value.includes('..')) fail('invalid_cookie_import');
  const host = value.toLowerCase();
  try {
    if (new URL(`https://${host}/`).hostname !== host) fail('invalid_cookie_import');
  } catch { fail('invalid_cookie_import'); }
  return host;
}

function fail(code) {
  const error = new Error(code);
  error.code = code;
  throw error;
}
