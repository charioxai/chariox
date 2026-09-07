import {prepareChromeCookieBatch} from './chrome-cookie-batch.mjs';
import {applyCookieImport,createCdpCookieStore} from './cookie-import-transaction.mjs';

// Internal bridge only. The kernel caller must supply live consent and exclusive
// Environment ownership; no wire-supplied approval flag is accepted here.
export async function applyControllerCookieImport({controller,browserGeneration,targetId,documentId,
  source,scope,overwrite=false}, {authorize,runExclusive,signal} = {}) {
  if (typeof authorize !== 'function' || typeof runExclusive !== 'function') fail('cookie_import_denied');
  if (!Number.isSafeInteger(browserGeneration) || browserGeneration < 1
      || typeof targetId !== 'string' || !targetId || typeof documentId !== 'string' || !documentId) {
    fail('cookie_import_target_stale');
  }
  const selectedSource = structuredClone(source);
  const selectedScope = structuredClone(scope);
  prepareChromeCookieBatch(selectedSource,selectedScope);
  const binding = Object.freeze({browserGeneration,targetId,documentId,overwrite:overwrite === true,
    sourceStoreId:selectedScope.sourceStoreId,
    approvedDomains:Object.freeze([...selectedScope.approvedDomains]),
    approvedPartitionSites:Object.freeze([...(selectedScope.approvedPartitionSites ?? [])])});
  return runExclusive(async () => {
    const live = () => {
      if (signal?.aborted) fail('cookie_import_cancelled');
      if (controller.browserGeneration !== browserGeneration) fail('cookie_import_target_stale');
    };
    live();
    if (await authorize(binding) !== true) fail('cookie_import_denied');
    live();
    const {connection,sessionId} = await controller.resolvePageTarget(targetId);
    const check = async () => {
      live();
      if (!connection.isOpen()) fail('cookie_import_target_stale');
      const {frameTree} = await connection.send('Page.getFrameTree',{},sessionId);
      if (frameTree?.frame?.loaderId !== documentId) fail('cookie_import_target_stale');
      if (await authorize(binding) !== true) fail('cookie_import_denied');
      live();
      return true;
    };
    await check();
    const store = await createCdpCookieStore({
      browserCdp:{send:(method,params) => connection.send(method,params)},
      pageCdp:{send:(method,params) => connection.send(method,params,sessionId)},
    });
    return applyCookieImport({source:selectedSource,scope:selectedScope,store,overwrite,signal,
      authorize:check,runExclusive:operation => operation()});
  });
}

function fail(code) {
  throw Object.assign(new Error(code),{code,recoveryRequired:false});
}
