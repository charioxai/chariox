import {prepareChromeCookieBatch} from './chrome-cookie-batch.mjs';

export async function createCdpCookieStore({browserCdp, pageCdp, timeoutMs = 5000}) {
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 5000) fail('cookie_import_invalid_timeout');
  let unavailable = false;
  let mutationPossible = false;
  const execute = async operation => {
    if (unavailable) fail('cookie_import_cdp_unavailable', mutationPossible);
    const deadline = performance.now() + timeoutMs;
    const send = async (session, method, params, mutates = false) => {
      if (unavailable) fail('cookie_import_cdp_unavailable', mutationPossible);
      const remaining = deadline - performance.now();
      if (remaining <= 0) {
        unavailable = true;
        fail('cookie_import_cdp_timeout', mutationPossible);
      }
      let timer;
      try {
        mutationPossible ||= mutates;
        return await Promise.race([
          Promise.resolve().then(() => session.send(method, params)),
          new Promise((_, reject) => {
            timer = setTimeout(() => reject(new CookieImportError('cookie_import_cdp_timeout', mutationPossible)), remaining);
          }),
        ]);
      } catch (error) {
        unavailable = true;
        fail(error instanceof CookieImportError ? error.code : 'cookie_import_cdp_failed', mutationPossible);
      } finally { clearTimeout(timer); }
    };
    return operation(send);
  };
  let target;
  let context = {};
  const check = async send => {
    const {targetInfo:current} = await send(browserCdp, 'Target.getTargetInfo', {targetId:target.targetId});
    if (current.targetId !== target.targetId || current.browserContextId !== target.browserContextId) {
      fail('cookie_import_target_changed');
    }
  };
  await execute(async send => {
    ({targetInfo:target} = await send(pageCdp, 'Target.getTargetInfo'));
    if (target.browserContextId) {
      const {browserContextIds,defaultBrowserContextId} = await send(browserCdp,'Target.getBrowserContexts');
      if (!Array.isArray(browserContextIds)) fail('cookie_import_context_unsupported');
      if (browserContextIds.includes(target.browserContextId)) context = {browserContextId:target.browserContextId};
      else if (target.browserContextId !== defaultBrowserContextId) fail('cookie_import_context_unsupported');
    }
    await check(send);
  });
  return {
    read:() => execute(async send => {await check(send); return (await send(browserCdp, 'Storage.getCookies',context)).cookies;}),
    write:cookies => execute(async send => {await check(send); await send(browserCdp, 'Storage.setCookies',{...context,cookies},true);}),
    remove:cookies => execute(async send => {
      await check(send);
      for (const {name,domain,path,partitionKey} of cookies) {
        await send(pageCdp, 'Network.deleteCookies',{name,domain,path,
          ...(partitionKey === undefined ? {} : {partitionKey})},true);
      }
    }),
  };
}

// Internal destination operation. No transport endpoint or authority is granted here.
export async function applyCookieImport({source, scope, store, runExclusive, authorize, signal, overwrite = false, journal}) {
  const batch = prepareChromeCookieBatch(source, scope);
  if (typeof runExclusive !== 'function' || typeof authorize !== 'function') fail('cookie_import_denied');
  const check = async () => {
    if (signal?.aborted) fail('cookie_import_cancelled');
    if (await authorize() !== true) fail('cookie_import_denied');
    if (signal?.aborted) fail('cookie_import_cancelled');
  };
  let mutationStarted = false;
  let receipt;
  try { return await runExclusive(async () => {
    await check();
    if (journal) {
      const pending = await journal.read();
      if (pending) {
        pending.bytes.fill(0);
        fail('cookie_import_recovery_required',true);
      }
    }
    if (!batch.cookies.length) return batch.summary;
    const before = await store.read();
    validateSnapshot(before);
    const keys = new Set(batch.cookies.map(identity));
    if (overwrite !== true && before.some(c => keys.has(identity(c)))) fail('cookie_import_conflict');
    const replaced = before.filter(c => keys.has(identity(c)));
    await check();
    if (journal) {
      const bytes = Buffer.from(JSON.stringify({schema:1,before,imported:batch.cookies}));
      try { receipt = await journal.prepare(bytes); }
      finally { bytes.fill(0); }
    }
    try {
      await check();
      mutationStarted = true;
      await store.write(batch.cookies);
      await check();
      const after = await store.read();
      if (batch.cookies.some(c => !after.some(actual => semantic(actual) === semantic(c)))
          || fingerprint(after.filter(c => !keys.has(identity(c)))) !== fingerprint(before.filter(c => !keys.has(identity(c))))) {
        fail('cookie_import_verification_failed');
      }
      await check();
    } catch (error) {
      let recoveryRequired = !(error instanceof CookieImportError) || error.recoveryRequired;
      if (mutationStarted) try {
        await store.remove(batch.cookies.map(c => ({...c, domain:c.domain ?? new URL(c.url).hostname})));
        if (replaced.length) await store.write(replaced.map(restoreParams));
        const restored = await store.read();
        recoveryRequired ||= fingerprint(restored) !== fingerprint(before);
      } catch { recoveryRequired = true; }
      if (receipt && !recoveryRequired) {
        try { await journal.discard(receipt); }
        catch { recoveryRequired = true; }
      }
      fail(error instanceof CookieImportError ? error.code : 'cookie_import_failed', recoveryRequired);
    }
    // Browser readback is not durable completion. The kernel must record the
    // outcome before clearing the retained journal and releasing quarantine.
    return batch.summary;
  }); } catch (error) {
    if (error instanceof CookieImportError) throw error;
    fail('cookie_import_failed', mutationStarted || error?.recoveryRequired === true);
  }
}

function identity(cookie) {
  return JSON.stringify([cookie.name, cookie.domain ?? new URL(cookie.url).hostname,
    cookie.path, cookie.partitionKey?.topLevelSite ?? null,
    cookie.partitionKey?.hasCrossSiteAncestor ?? null]);
}

function validateSnapshot(cookies) {
  if (!Array.isArray(cookies) || cookies.length > 10000
      || new TextEncoder().encode(JSON.stringify(cookies)).byteLength > 4 * 1024 * 1024) fail('cookie_import_snapshot_too_large');
  const keys = new Set();
  for (const cookie of cookies) {
    if (typeof cookie.session !== 'boolean' || (cookie.session ? cookie.expires !== -1
        : !Number.isFinite(cookie.expires) || cookie.expires <= Date.now() / 1000)) {
      fail('cookie_import_snapshot_unsupported');
    }
    if (cookie.partitionKeyOpaque || Object.keys(cookie).some(key => ![
      'name','value','domain','path','expires','size','httpOnly','secure','session','sameSite',
      'priority','sourceScheme','sourcePort','partitionKey','partitionKeyOpaque',
    ].includes(key))) fail('cookie_import_snapshot_unsupported');
    const key = identity(cookie);
    if (keys.has(key)) fail('cookie_import_snapshot_unsupported');
    keys.add(key);
  }
}

function semantic(cookie) {
  return JSON.stringify([identity(cookie), cookie.value, cookie.secure, cookie.httpOnly,
    cookie.sameSite ?? null, cookie.session ?? cookie.expires === undefined,
    Math.round((cookie.expires ?? -1) * 1000)]);
}

function fingerprint(cookies) {
  return JSON.stringify(cookies.map(c => JSON.stringify([semantic(c),
    c.priority ?? null, c.sourceScheme ?? null, c.sourcePort ?? null])).sort());
}

function restoreParams(cookie) {
  const {name,value,path,secure,httpOnly,sameSite,partitionKey} = cookie;
  return {name,value,path,secure,httpOnly,
    url:`${secure ? 'https' : 'http'}://${cookie.domain.replace(/^\./, '')}/`,
    ...(cookie.domain.startsWith('.') ? {domain:cookie.domain} : {}),
    ...(sameSite === undefined ? {} : {sameSite}),
    ...(cookie.session ? {} : {expires:cookie.expires}),
    ...(partitionKey === undefined ? {} : {partitionKey}),
    ...Object.fromEntries(['priority','sourceScheme','sourcePort'].filter(k => cookie[k] !== undefined).map(k => [k,cookie[k]]))};
}

class CookieImportError extends Error {
  constructor(code, recoveryRequired) {
    super(code);
    this.code = code;
    this.recoveryRequired = recoveryRequired;
  }
}

function fail(code, recoveryRequired = false) { throw new CookieImportError(code, recoveryRequired); }
