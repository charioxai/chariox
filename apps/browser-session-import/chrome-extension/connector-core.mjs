const acceptedChallenges = new WeakSet();
const pairingCode = 'connector_pairing_denied';

export function metadataOnlyDiscovery({sourceTabId,url,incognito}) {
  try {
    const parsed = new URL(url);
    if (!Number.isSafeInteger(sourceTabId) || sourceTabId < 0 || incognito !== false
        || !['http:','https:'].includes(parsed.protocol) || !hostname(parsed.hostname)) throw new Error();
    return Object.freeze({currentProfile:true,sourceTabId,hostname:parsed.hostname.toLowerCase()});
  } catch { throw failure(pairingCode); }
}

export function createPairingChallenge({senderPublicKey,nonceBytes,discovered,expectedKernelPublicKey}) {
  try {
    if (!key(senderPublicKey) || !(nonceBytes instanceof Uint8Array) || nonceBytes.byteLength !== 16
        || discovered?.currentProfile !== true && !validDiscoveryInput(discovered)
        || (expectedKernelPublicKey !== undefined && !key(expectedKernelPublicKey))) throw new Error();
    const source = discovered.currentProfile === true ? discovered
      : {currentProfile:true,sourceTabId:discovered.sourceTabId,hostname:discovered.hostname};
    return {
      version:1,
      enrollment_nonce:base64(nonceBytes),
      connector_sender_public_key:senderPublicKey,
      source:{current_profile:true,hostname:source.hostname},
      sourceTabId:source.sourceTabId,
      expectedKernelPublicKey,
    };
  } catch { throw failure(pairingCode); }
}

export function acceptPairingEnvelope(value,challenge,{now = Date.now()} = {}) {
  try {
    if (acceptedChallenges.has(challenge) || !plain(value) || !plain(challenge)) throw new Error();
    exact(value,['version','enrollment_nonce','connector_sender_public_key','expires_at_ms','relay_url',
      'relay_auth_token','daemon_id','kernel_public_key','protocol_version','delivery_capability',
      'source','selection']);
    if (value.version !== 1 || value.enrollment_nonce !== challenge.enrollment_nonce
        || value.connector_sender_public_key !== challenge.connector_sender_public_key
        || !key(value.connector_sender_public_key) || !key(value.kernel_public_key)
        || challenge.expectedKernelPublicKey !== undefined
          && value.kernel_public_key !== challenge.expectedKernelPublicKey
        || !Number.isSafeInteger(now) || !Number.isSafeInteger(value.expires_at_ms)
        || value.expires_at_ms <= now || value.expires_at_ms > now + 120_000
        || !text(value.relay_auth_token,16384) || !text(value.daemon_id,512)
        || !Number.isInteger(value.protocol_version) || value.protocol_version < 320
        || !plain(value.delivery_capability) || Object.keys(value.delivery_capability).length !== 2
        || value.delivery_capability.name !== 'browser_import_final_delivery'
        || value.delivery_capability.version !== 1) throw new Error();
    const relay = new URL(value.relay_url);
    if (relay.protocol !== 'wss:' || relay.username || relay.password || relay.search || relay.hash) throw new Error();
    exact(value.source,['current_profile','store_id']);
    if (value.source.current_profile !== true || value.source.store_id !== '0') throw new Error();
    const selection = snapshotSelection(value.selection);
    if (selection.source_store_id !== value.source.store_id) throw new Error();
    exactHostPermission(selection.domains);
    acceptedChallenges.add(challenge);
    return Object.freeze({relayUrl:relay.href,relayAuthToken:value.relay_auth_token,
      daemonId:value.daemon_id,kernelPublicKey:value.kernel_public_key,
      senderPublicKey:value.connector_sender_public_key,protocolVersion:value.protocol_version,
      sourceTabId:challenge.sourceTabId,selection});
  } catch { throw failure(pairingCode); }
}

export function exactHostPermission(domains) {
  try {
    if (!Array.isArray(domains) || domains.length < 1 || domains.length > 32) throw new Error();
    const normalized = [...new Set(domains.map(domain => {
      if (!hostname(domain)) throw new Error();
      return domain.toLowerCase();
    }))];
    return Object.freeze({permissions:Object.freeze(['cookies']),
      origins:Object.freeze(normalized.map(domain => `*://${domain}/*`))});
  } catch { throw failure(pairingCode); }
}

export function publicProgress(phase,completedDomains,totalDomains) {
  if (!['requesting_permission','verifying_consent','transferring','applying','verifying'].includes(phase)
      || !count(completedDomains) || !count(totalDomains) || completedDomains > totalDomains) {
    return Object.freeze({type:'progress',phase:'verifying_consent',completedDomains:0,totalDomains:0});
  }
  return Object.freeze({type:'progress',phase,completedDomains,totalDomains});
}

export function publicResult(summary) {
  try {
    if (!plain(summary) || !Array.isArray(summary.domains) || !count(summary.imported)
        || !count(summary.signInRequired) || summary.imported + summary.signInRequired > summary.domains.length) {
      throw new Error();
    }
    const domains = summary.domains.map((domain,index) => ({domain:checkedHostname(domain),
      status:index < summary.imported ? 'imported' : index < summary.imported + summary.signInRequired
        ? 'sign_in_required' : 'unsupported'}));
    return Object.freeze({type:'result',status:'completed',domains:Object.freeze(domains)});
  } catch { return publicFailure(); }
}

export function publicFailure(error) {
  const allowed = new Set(['connector_pairing_denied','cookie_source_denied','cookie_source_cancelled',
    'cookie_source_timeout','cookie_source_unavailable','cookie_source_too_large',
    'browser_import_delivery_unavailable']);
  return Object.freeze({type:'result',status:'failed',code:allowed.has(error?.code)
    ? error.code : 'connector_unavailable'});
}

function snapshotSelection(source) {
  if (!plain(source)) throw new Error();
  exact(source,['session_id','attachment_id','environment_id','runtime_generation','tab_id',
    'document_revision','source_store_id','domains','partition_sites','overwrite']);
  const result = {};
  for (const field of ['session_id','attachment_id','environment_id','tab_id','source_store_id']) {
    result[field] = checkedText(source[field],512);
  }
  for (const field of ['runtime_generation','document_revision']) {
    if (!count(source[field])) throw new Error();
    result[field] = source[field];
  }
  result.domains = Object.freeze(source.domains.map(checkedHostname));
  if (!Array.isArray(source.partition_sites) || source.partition_sites.length > 32) throw new Error();
  result.partition_sites = Object.freeze(source.partition_sites.map(site => {
    const url = new URL(site);
    if (url.origin !== site || !['http:','https:'].includes(url.protocol)) throw new Error();
    return site;
  }));
  if (typeof source.overwrite !== 'boolean') throw new Error();
  result.overwrite = source.overwrite;
  return Object.freeze(result);
}

function validDiscoveryInput(value) {
  return plain(value) && Number.isSafeInteger(value.sourceTabId) && value.sourceTabId >= 0
    && hostname(value.hostname);
}
function checkedHostname(value) { if (!hostname(value)) throw new Error(); return value.toLowerCase(); }
function hostname(value) { return typeof value === 'string' && value.length > 0 && value.length <= 253
  && /^(?=.{1,253}$)(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)*[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?$/.test(value); }
function checkedText(value,max) { if (!text(value,max)) throw new Error(); return value; }
function text(value,max) { return typeof value === 'string' && value.length > 0 && value.length <= max
  && !/[\x00-\x1f\x7f]/.test(value); }
function key(value) { return text(value,256); }
function count(value) { return Number.isSafeInteger(value) && value >= 0; }
function plain(value) { return typeof value === 'object' && value !== null && !Array.isArray(value); }
function exact(value,fields) { if (!plain(value) || Object.keys(value).length !== fields.length
  || Object.keys(value).some(field => !fields.includes(field))) throw new Error(); }
function base64(bytes) { let binary=''; for (const byte of bytes) binary += String.fromCharCode(byte); return btoa(binary); }
function failure(code) { return Object.assign(new Error(code),{code}); }
