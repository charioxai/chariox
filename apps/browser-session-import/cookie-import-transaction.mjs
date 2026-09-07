import {prepareChromeCookieBatch} from './chrome-cookie-batch.mjs';

export async function createCdpCookieStore({browserCdp, pageCdp}) {
  const {targetInfo:target} = await pageCdp.send('Target.getTargetInfo');
  const context = target.browserContextId ? {browserContextId:target.browserContextId} : {};
  const check = async () => {
    const {targetInfo:current} = await browserCdp.send('Target.getTargetInfo', {targetId:target.targetId});
    if (current.targetId !== target.targetId || current.browserContextId !== target.browserContextId) {
      fail('cookie_import_target_changed');
    }
  };
  await check();
  return {
    read:async () => {await check(); return (await browserCdp.send('Storage.getCookies',context)).cookies;},
    write:async cookies => {await check(); await browserCdp.send('Storage.setCookies',{...context,cookies});},
    remove:async cookies => {
      await check();
      for (const {name,domain,path,partitionKey} of cookies) {
        await pageCdp.send('Network.deleteCookies',{name,domain,path,
          ...(partitionKey === undefined ? {} : {partitionKey})});
      }
    },
  };
}

// Internal destination operation. No transport endpoint or authority is granted here.
export async function applyCookieImport({source, scope, store, runExclusive, authorize, signal, overwrite = false}) {
  const batch = prepareChromeCookieBatch(source, scope);
  if (typeof runExclusive !== 'function' || typeof authorize !== 'function') fail('cookie_import_denied');
  const check = async () => {
    if (signal?.aborted) fail('cookie_import_cancelled');
    if (await authorize() !== true) fail('cookie_import_denied');
    if (signal?.aborted) fail('cookie_import_cancelled');
  };
  let mutationStarted = false;
  try { return await runExclusive(async () => {
    await check();
    if (!batch.cookies.length) return batch.summary;
    const before = await store.read();
    validateSnapshot(before);
    const keys = new Set(batch.cookies.map(identity));
    if (overwrite !== true && before.some(c => keys.has(identity(c)))) fail('cookie_import_conflict');
    const replaced = before.filter(c => keys.has(identity(c)));
    await check();
    try {
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
      let recoveryRequired = !(error instanceof CookieImportError);
      try {
        await store.remove(batch.cookies.map(c => ({...c, domain:c.domain ?? new URL(c.url).hostname})));
        if (replaced.length) await store.write(replaced.map(restoreParams));
        const restored = await store.read();
        recoveryRequired ||= fingerprint(restored) !== fingerprint(before);
      } catch { recoveryRequired = true; }
      fail(error instanceof CookieImportError ? error.code : 'cookie_import_failed', recoveryRequired);
    }
    return batch.summary;
  }); } catch (error) {
    if (error instanceof CookieImportError) throw error;
    fail('cookie_import_failed', mutationStarted);
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
    cookie.sameSite ?? null, Math.round((cookie.expires ?? -1) * 1000)]);
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
