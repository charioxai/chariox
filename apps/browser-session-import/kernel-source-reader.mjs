import {claimBrowserImportSourceRequest, authorizeBrowserImportSourceRequest}
  from '../../packages/kernel-client/src/browser-import-requests.ts';
import {readApprovedChromeCookies} from './chrome-cookie-reader.mjs';

// Internal connector entry point. request must use the initiating client's
// authenticated kernel transport, never a page-supplied approval callback.
export async function readKernelApprovedChromeCookies({chrome, requestId, selection,
  sourceTabId, request, signal, timeoutMs = 30000}) {
  let claim;
  try {
    if (typeof request !== 'function' || typeof requestId !== 'string'
        || !/^[a-fA-F0-9]{32}$/.test(requestId)) throw new Error();
    claim = claimBrowserImportSourceRequest(requestId, selection);
  } catch {
    const error = new Error('cookie_source_denied');
    error.code = 'cookie_source_denied';
    throw error;
  }
  const approved = claim.ClaimBrowserImportSource.selection;
  const authorize = authorizeBrowserImportSourceRequest(requestId, approved);
  let claimed = false;
  let finished = false;
  const deadline = performance.now() + timeoutMs;
  const active = () => !finished && !signal?.aborted && performance.now() < deadline;
  try {
    return await readApprovedChromeCookies({chrome, sourceTabId, signal, timeoutMs,
      scope:{sourceStoreId:approved.source_store_id, approvedDomains:approved.domains,
        approvedPartitionSites:approved.partition_sites},
      authorize:async () => {
        if (!active()) return false;
        if (!claimed) {
          const response = await request(claim);
          if (!active() || !matches(response, requestId, 'source_claimed')) return false;
          claimed = true;
        }
        const response = await request(authorize);
        return active() && matches(response, requestId, 'source_authorized');
      },
    });
  } finally { finished = true; }
}

function matches(response, requestId, status) {
  return response?.BrowserImportConsent?.request_id === requestId
    && response.BrowserImportConsent.status === status;
}
