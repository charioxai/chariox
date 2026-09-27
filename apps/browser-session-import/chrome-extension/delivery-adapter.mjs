const deliveryError = () => Object.assign(new Error('browser_import_delivery_unavailable'),
  {code:'browser_import_delivery_unavailable'});

// The supplied method belongs to the independently pinned relay connection.
// Cookie values cross only that method and are cleared on every terminal path.
export async function deliverBrowserImport({deliver,requestId,selection,cookies},options = {}) {
  try {
    if (typeof deliver !== 'function' || typeof requestId !== 'string'
        || !/^[a-fA-F0-9]{32}$/.test(requestId) || !Object.isFrozen(selection)
        || !Object.isFrozen(selection?.domains) || !Array.isArray(selection.domains)
        || selection.domains.length < 1 || selection.domains.length > 32
        || !Array.isArray(cookies) || cookies.length > 512) throw deliveryError();
    const response = await deliver({requestId,selection,cookies},options);
    const results = validResults(response,selection.domains);
    return Object.freeze({requestId,status:'completed',domains:Object.freeze(results.map(result =>
      Object.freeze({domain:result.domain,status:result.status === 'imported'
        ? 'imported' : 'sign_in_required'})))});
  } catch { throw deliveryError(); }
  finally { scrub(cookies); }
}

function validResults(response,domains) {
  if (!exact(response,['BrowserImportDelivered'])
      || !exact(response.BrowserImportDelivered,['results'])
      || !Array.isArray(response.BrowserImportDelivered.results)
      || response.BrowserImportDelivered.results.length !== domains.length) throw deliveryError();
  let total = 0;
  for (let index = 0; index < domains.length; index++) {
    const result = response.BrowserImportDelivered.results[index];
    if (!exact(result,['domain','status','cookie_count']) || result.domain !== domains[index]
        || !['imported','no_cookies'].includes(result.status)
        || !Number.isSafeInteger(result.cookie_count) || result.cookie_count < 0
        || result.cookie_count > 512
        || (result.status === 'imported') !== (result.cookie_count > 0)) throw deliveryError();
    total += result.cookie_count;
    if (total > 512) throw deliveryError();
  }
  return response.BrowserImportDelivered.results;
}

function scrub(cookies) {
  if (!Array.isArray(cookies)) return;
  for (const cookie of cookies) if (cookie && typeof cookie === 'object') {
    try { cookie.value = ''; } catch { /* immutable source is discarded by the caller */ }
  }
}

function exact(value,keys) {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value,key));
}
