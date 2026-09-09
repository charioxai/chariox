import {prepareChromeCookieBatch} from './chrome-cookie-batch.mjs';
import {applyCookieImport,createCdpCookieStore,recoverCookieImport} from './cookie-import-transaction.mjs';

// Internal bridge only. The kernel caller must supply live consent and exclusive
// Environment ownership; no wire-supplied approval flag is accepted here.
export async function applyControllerCookieImport({controller,browserGeneration,targetId,documentId,
  source,scope,overwrite=false}, {authorize,runExclusive,signal,journal,complete} = {}) {
  if (typeof authorize !== 'function' || typeof runExclusive !== 'function'
      || typeof complete !== 'function' || typeof controller?.withCookieWritersQuiesced !== 'function') {
    fail('cookie_import_denied');
  }
  if (!journal || ['read','prepare','discard'].some(method => typeof journal[method] !== 'function')) {
    fail('cookie_import_journal_required');
  }
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
  return runExclusive(async () => controller.withCookieWritersQuiesced(async fence => {
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
    try {
      const result = await applyCookieImport({source:selectedSource,scope:selectedScope,store,overwrite,signal,journal,
        authorize:check,runExclusive:operation => operation()});
      if (result.cookieCount > 0) {
        let receipt;
        try {
          const pending = await journal.read();
          if (!pending) fail('cookie_import_completion_failed',true);
          receipt = pending.receipt;
          pending.bytes.fill(0);
        } catch { fail('cookie_import_completion_failed',true); }
        await completeAndVerify({journal,complete,payload:{receipt,result,binding}});
      }
      return result;
    } catch (error) {
      if (error?.recoveryRequired === true) fence.retain();
      throw error;
    }
  }));
}

// Restart recovery uses the same controller fence. A retained in-process fence
// is reused; a new executor acquires a fresh fence before reading the journal.
export async function recoverControllerCookieImport({controller,targetId},
  {authorize,runExclusive,journal,complete} = {}) {
  if (typeof targetId !== 'string' || !targetId || typeof authorize !== 'function'
      || typeof runExclusive !== 'function' || typeof complete !== 'function'
      || typeof controller?.withCookieWritersQuiesced !== 'function'
      || !journal || ['read','discard'].some(method => typeof journal[method] !== 'function')) {
    fail('cookie_import_denied',true);
  }
  return runExclusive(async () => controller.withCookieWritersQuiesced(async fence => {
    try {
      const {connection,sessionId} = await controller.resolvePageTarget(targetId);
      const store = await createCdpCookieStore({
        browserCdp:{send:(method,params) => connection.send(method,params)},
        pageCdp:{send:(method,params) => connection.send(method,params,sessionId)},
      });
      const result = await recoverCookieImport({journal,store,runExclusive:operation => operation(),authorize});
      await completeAndVerify({journal,complete,payload:result});
      return result;
    } catch (error) {
      if (error?.recoveryRequired === true) fence.retain();
      throw error;
    }
  }));
}

async function completeAndVerify({journal,complete,payload}) {
  try {
    await complete(payload);
    const retained = await journal.read();
    if (retained) {
      retained.bytes.fill(0);
      fail('cookie_import_completion_failed',true);
    }
  } catch { fail('cookie_import_completion_failed',true); }
}

function fail(code,recoveryRequired=false) {
  throw Object.assign(new Error(code),{code,recoveryRequired});
}
