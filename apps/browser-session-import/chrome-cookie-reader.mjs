import { prepareChromeCookieBatch } from './chrome-cookie-batch.mjs';

// Internal connector adapter, not a web-accessible export endpoint.
export async function readApprovedChromeCookies({chrome, scope, sourceTabId, authorize, signal, timeoutMs = 30000}) {
  if (signal?.aborted) fail('cookie_source_cancelled');
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0 || timeoutMs > 30000) fail('cookie_source_denied');
  const deadline = performance.now() + timeoutMs;
  const safe = call => sourceCall(call, signal, deadline);
  prepareChromeCookieBatch([], scope);
  scope = Object.freeze({...scope, approvedDomains:Object.freeze([...scope.approvedDomains]),
    approvedPartitionSites:Object.freeze([...(scope.approvedPartitionSites ?? [])])});
  const selection = Object.freeze({sourceTabId, scope});
  if (!Number.isSafeInteger(sourceTabId) || sourceTabId < 0 || typeof authorize !== 'function'
      || await safe(() => authorize(selection)) !== true) fail('cookie_source_denied');
  const tab = await safe(() => chrome.tabs.get(sourceTabId));
  const stores = await safe(() => chrome.cookies.getAllCookieStores());
  const matches = stores.filter(store => store.tabIds.includes(sourceTabId));
  if (tab.id !== sourceTabId || tab.incognito !== false || matches.length !== 1
      || matches[0].id !== scope.sourceStoreId) fail('cookie_source_denied');
  const origins = [...new Set(scope.approvedDomains.map(domain => `*://${domain.toLowerCase()}/*`))];
  const check = async () => {
    if (signal?.aborted) fail('cookie_source_cancelled');
    if (!origins.length || await safe(() => authorize(selection)) !== true
        || await safe(() => chrome.permissions.contains({permissions:['cookies'], origins})) !== true) fail('cookie_source_denied');
    if (signal?.aborted) fail('cookie_source_cancelled');
  };
  await check();
  const cookies = [];
  for (const domain of new Set(scope.approvedDomains.map(value => value.toLowerCase()))) {
    for (const site of [undefined, ...new Set(scope.approvedPartitionSites ?? [])]) {
      await check();
      const found = await safe(() => chrome.cookies.getAll({domain, storeId:scope.sourceStoreId,
        ...(site ? {partitionKey:{topLevelSite:site}} : {})}));
      await check();
      if (!Array.isArray(found)) fail('cookie_source_unavailable');
      if (found.length > 512) fail('cookie_source_too_large');
      for (const cookie of found) {
        if (typeof cookie?.domain !== 'string') fail('cookie_source_unavailable');
        if (cookie.domain.replace(/^\./, '').toLowerCase() !== domain) continue;
        if (cookies.length >= 512) fail('cookie_source_too_large');
        cookies.push(cookie);
      }
      prepareChromeCookieBatch(cookies, scope);
    }
  }
  await check();
  const {summary} = prepareChromeCookieBatch(cookies, scope);
  // Keep the source format for independent validation by the destination.
  // CDP parameters are created only where cookies are applied.
  return {cookies:structuredClone(cookies), summary};
}

async function sourceCall(call, signal, deadline) {
  if (signal?.aborted) fail('cookie_source_cancelled');
  const remaining = deadline - performance.now();
  if (remaining <= 0) fail('cookie_source_timeout');
  let onAbort;
  let timer;
  try {
    return await Promise.race([
      Promise.resolve().then(call).catch(() => fail('cookie_source_unavailable')),
      new Promise((_, reject) => {
        onAbort = () => reject(sourceError('cookie_source_cancelled'));
        signal?.addEventListener('abort', onAbort, {once:true});
        timer = setTimeout(() => reject(sourceError('cookie_source_timeout')), remaining);
      }),
    ]);
  } finally {
    signal?.removeEventListener('abort', onAbort);
    clearTimeout(timer);
  }
}

function sourceError(code) {
  const error = new Error(code);
  error.code = code;
  return error;
}

function fail(code) { throw sourceError(code); }
