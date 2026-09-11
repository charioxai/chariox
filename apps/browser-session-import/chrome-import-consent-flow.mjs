import {approveBrowserImportRequest,cancelBrowserImportRequest}
  from '../../packages/kernel-client/src/browser-import-requests.ts';
import {snapshotImportRequest} from './relay-request-metadata.mjs';
import {prepareChromeCookieBatch} from './chrome-cookie-batch.mjs';
import {sourceCall} from './chrome-cookie-reader.mjs';
import {readKernelApprovedChromeCookies} from './kernel-source-reader.mjs';

// Trusted extension UI only. request must belong to an authenticated, paired
// connector. This creates no page-message endpoint and does not apply cookies.
export async function prepareChromeCookieImport({chrome,selection,sourceTabId,request,
  signal,timeoutMs = 120000}) {
  let selected;
  let permission;
  try {
    if (typeof request !== 'function' || !Number.isSafeInteger(sourceTabId) || sourceTabId < 0
        || !Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 120000
        || (signal != null && !(signal instanceof AbortSignal))) throw new Error();
    selected = snapshotImportRequest({PrepareBrowserImport:{selection}}).request.PrepareBrowserImport.selection;
    prepareChromeCookieBatch([],{sourceStoreId:selected.source_store_id,
      approvedDomains:selected.domains,approvedPartitionSites:selected.partition_sites});
    Object.freeze(selected.domains);
    Object.freeze(selected.partition_sites);
    Object.freeze(selected);
    permission = {permissions:['cookies'],origins:[...new Set(selected.domains.map(host => `*://${host.toLowerCase()}/*`))]};
    if (typeof chrome?.permissions?.request !== 'function') throw new Error();
  } catch { throw error('cookie_source_denied'); }
  if (signal?.aborted) throw error('cookie_source_cancelled');

  const deadline = performance.now() + timeoutMs;
  const active = new AbortController();
  let state = 'preparing';
  let requestId;
  let cleanup;
  let timer;
  let ended = false;
  const onAbort = () => { void cancel(); };
  signal?.addEventListener('abort',onAbort,{once:true});
  const send = payload => sourceCall(() => request(payload,{signal:active.signal,
    timeoutMs:Math.min(5000,Math.max(1,Math.ceil(deadline - performance.now())))}),active.signal,
  Math.min(deadline,performance.now() + 5000));
  const detach = () => {
    clearTimeout(timer);
    signal?.removeEventListener('abort',onAbort);
  };
  const live = () => {
    if (ended || active.signal.aborted || signal?.aborted) throw error('cookie_source_cancelled');
    if (performance.now() >= deadline) throw error('cookie_source_timeout');
  };

  // At most one best-effort cancellation. Its fresh signal does not revive the
  // aborted source read, close the shared relay or abort the caller's signal.
  function cancel() {
    if (cleanup) return cleanup;
    ended = true;
    state = 'cancelled';
    active.abort();
    detach();
    cleanup = (async () => {
      if (!requestId) return {kernelCancellationConfirmed:false};
      const cancellation = new AbortController();
      try {
        const response = await sourceCall(() => request(cancelBrowserImportRequest(
          selected.session_id,selected.attachment_id,requestId),
        {signal:cancellation.signal,timeoutMs:5000}),cancellation.signal,performance.now() + 5000);
        return {kernelCancellationConfirmed:matches(response,requestId,'cancelled')};
      } catch { return {kernelCancellationConfirmed:false}; }
      finally { cancellation.abort(); }
    })();
    return cleanup;
  }

  try {
    live();
    const response = await send({PrepareBrowserImport:{selection:selected}});
    const consent = response?.BrowserImportConsent;
    if (typeof consent?.request_id !== 'string' || !/^[a-fA-F0-9]{32}$/.test(consent.request_id)
        || !matches(response,consent.request_id,'prepared')) throw error('cookie_source_denied');
    requestId = consent.request_id;
    live();
    state = 'prepared';
    timer = setTimeout(() => { void cancel(); },Math.max(1,deadline - performance.now()));
  } catch (failure) {
    await cancel();
    throw safeError(failure);
  }

  // Call directly in the extension's confirmation click handler. No await,
  // permission preflight or kernel round trip may precede permissions.request:
  // Chrome enforces that optional permission requests originate in a gesture.
  async function confirmAndRead() {
    if (state !== 'prepared') throw error('cookie_source_denied');
    state = 'requesting_permission';
    try {
      live();
      const granted = chrome.permissions.request(structuredClone(permission));
      if (await sourceCall(() => granted,active.signal,deadline) !== true) throw error('cookie_source_denied');
      live();
      state = 'approving';
      if (!matches(await send(approveBrowserImportRequest(requestId,selected)),requestId,'approved')) {
        throw error('cookie_source_denied');
      }
      live();
      state = 'reading';
      const result = await readKernelApprovedChromeCookies({chrome,requestId,selection:selected,
        sourceTabId,request,signal:active.signal,
        timeoutMs:Math.min(30000,deadline - performance.now())});
      live();
      state = 'source_read';
      detach();
      // Leave the one-use kernel source claim for the destination executor.
      // No cookie payload is retained in this flow's state or request metadata.
      return result;
    } catch (failure) {
      await cancel();
      throw safeError(failure);
    }
  }
  return Object.freeze({selection:selected,sourceTabId,
    get requestId() { return requestId; },
    get state() { return state; },confirmAndRead,cancel});
}

function matches(response,id,status) {
  const consent = response?.BrowserImportConsent;
  return consent?.request_id === id && consent.status === status
    && Object.keys(response).length === 1 && Object.keys(consent).length === 2;
}

function error(code) { return Object.assign(new Error(code),{code}); }
function safeError(failure) {
  const codes = ['cookie_source_denied','cookie_source_cancelled','cookie_source_timeout',
    'cookie_source_unavailable','cookie_source_too_large'];
  return error(codes.includes(failure?.code) ? failure.code : 'cookie_source_unavailable');
}
