const acceptedChallenges = new WeakSet();
const trustedBootstraps = new WeakSet();
const challengeBootstraps = new WeakMap();
const pairingCode = 'connector_pairing_denied';

export function metadataOnlyDiscovery({sourceTabId,url,incognito}) {
  try {
    const parsed = new URL(url);
    if (!Number.isSafeInteger(sourceTabId) || sourceTabId < 0 || incognito !== false
        || !['http:','https:'].includes(parsed.protocol) || !hostname(parsed.hostname)) throw new Error();
    return Object.freeze({currentProfile:true,sourceTabId,hostname:parsed.hostname.toLowerCase()});
  } catch { throw failure(pairingCode); }
}

export function acceptAuthenticatedBootstrap(value,{senderPublicKey,discovered,now = Date.now()}) {
  try {
    if (!plain(value) || !relayKey(senderPublicKey) || !validDiscovery(discovered)
        || !Number.isSafeInteger(now)) throw new Error();
    exact(value,['version','bootstrap_id','connector_sender_public_key','expires_at_ms','relay_url',
      'daemon_id','kernel_public_key','protocol_version','delivery_capability','source','selection']);
    const relayUrl = trustedRelayUrl(value.relay_url);
    const selection = snapshotSelection(value.selection);
    exact(value.source,['current_profile','hostname','store_id']);
    validateDeliveryCapability(value.delivery_capability);
    if (value.version !== 1 || !identifier(value.bootstrap_id)
        || value.connector_sender_public_key !== senderPublicKey || !relayKey(value.kernel_public_key)
        || !Number.isSafeInteger(value.expires_at_ms) || value.expires_at_ms <= now
        || value.expires_at_ms > now + 120_000 || !text(value.daemon_id,512)
        || value.protocol_version < 321 || !Number.isInteger(value.protocol_version)
        || value.source.current_profile !== true || value.source.hostname !== discovered.hostname
        || value.source.store_id !== '0' || selection.source_store_id !== value.source.store_id) throw new Error();
    const bootstrap = Object.freeze({bootstrapId:value.bootstrap_id,expiresAtMs:value.expires_at_ms,
      relayUrl,daemonId:value.daemon_id,kernelPublicKey:value.kernel_public_key,
      senderPublicKey,protocolVersion:value.protocol_version,sourceTabId:discovered.sourceTabId,selection});
    trustedBootstraps.add(bootstrap);
    return bootstrap;
  } catch { throw failure(pairingCode); }
}

export function createPairingChallenge({senderPublicKey,nonceBytes,discovered,bootstrap}) {
  try {
    if (!relayKey(senderPublicKey) || !(nonceBytes instanceof Uint8Array) || nonceBytes.byteLength !== 16
        || !validDiscovery(discovered) || !trustedBootstraps.has(bootstrap)
        || bootstrap.senderPublicKey !== senderPublicKey || bootstrap.sourceTabId !== discovered.sourceTabId) {
      throw new Error();
    }
    const challenge = {version:1,bootstrap_id:bootstrap.bootstrapId,enrollment_nonce:base64(nonceBytes),
      connector_sender_public_key:senderPublicKey,
      source:{current_profile:true,hostname:discovered.hostname},sourceTabId:discovered.sourceTabId};
    challengeBootstraps.set(challenge,bootstrap);
    return challenge;
  } catch { throw failure(pairingCode); }
}

export function acceptPairingEnvelope(value,challenge,{now = Date.now()} = {}) {
  try {
    const bootstrap = challengeBootstraps.get(challenge);
    if (acceptedChallenges.has(challenge) || !plain(value) || !plain(challenge)
        || !trustedBootstraps.has(bootstrap)) throw new Error();
    exact(value,['version','bootstrap_id','enrollment_nonce','connector_sender_public_key','expires_at_ms','relay_url',
      'relay_auth_token','daemon_id','kernel_public_key','protocol_version','delivery_capability',
      'source','selection']);
    if (value.version !== 1 || value.bootstrap_id !== bootstrap.bootstrapId
        || value.enrollment_nonce !== challenge.enrollment_nonce
        || value.connector_sender_public_key !== challenge.connector_sender_public_key
        || !relayKey(value.connector_sender_public_key) || value.kernel_public_key !== bootstrap.kernelPublicKey
        || !Number.isSafeInteger(now) || !Number.isSafeInteger(value.expires_at_ms)
        || value.expires_at_ms <= now || value.expires_at_ms > bootstrap.expiresAtMs
        || !text(value.relay_auth_token,16384) || value.daemon_id !== bootstrap.daemonId
        || value.protocol_version !== bootstrap.protocolVersion) throw new Error();
    validateDeliveryCapability(value.delivery_capability);
    const relayUrl = trustedRelayUrl(value.relay_url);
    if (relayUrl !== bootstrap.relayUrl) throw new Error();
    exact(value.source,['current_profile','store_id']);
    if (value.source.current_profile !== true || value.source.store_id !== '0') throw new Error();
    const selection = snapshotSelection(value.selection);
    if (selection.source_store_id !== value.source.store_id || !sameSelection(selection,bootstrap.selection)) throw new Error();
    exactHostPermission(selection.domains);
    acceptedChallenges.add(challenge);
    return Object.freeze({relayUrl,relayAuthToken:value.relay_auth_token,
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

export function publicResult(value,{requestId,confirmedDomains} = {}) {
  try {
    if (!identifier(requestId) || !Array.isArray(confirmedDomains) || confirmedDomains.length < 1
        || confirmedDomains.length > 32 || !plain(value)) throw new Error();
    exact(value,['requestId','status','domains']);
    if (value.requestId !== requestId || value.status !== 'completed' || !Array.isArray(value.domains)
        || value.domains.length !== confirmedDomains.length) throw new Error();
    const expected = new Set(confirmedDomains.map(checkedHostname));
    if (expected.size !== confirmedDomains.length) throw new Error();
    const seen = new Set();
    const domains = value.domains.map(item => {
      exact(item,['domain','status']);
      const domain = checkedHostname(item.domain);
      if (!expected.has(domain) || seen.has(domain)
          || !['imported','sign_in_required','unsupported'].includes(item.status)) throw new Error();
      seen.add(domain);
      return Object.freeze({domain,status:item.status});
    });
    if (seen.size !== expected.size) throw new Error();
    return Object.freeze({type:'result',requestId,status:'completed',domains:Object.freeze(domains)});
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

function validDiscovery(value) { return plain(value) && value.currentProfile === true
  && Number.isSafeInteger(value.sourceTabId) && value.sourceTabId >= 0 && hostname(value.hostname); }
function validateDeliveryCapability(value) { exact(value,['name','version']);
  if (value.name !== 'browser_import_final_delivery' || value.version !== 1) throw new Error(); }
function trustedRelayUrl(value) { const relay = new URL(value);
  if (relay.protocol !== 'wss:' || relay.username || relay.password || relay.search || relay.hash) throw new Error();
  return relay.href; }
function sameSelection(left,right) { return ['session_id','attachment_id','environment_id','runtime_generation',
  'tab_id','document_revision','source_store_id','overwrite'].every(field => left[field] === right[field])
  && left.domains.length === right.domains.length && left.domains.every((value,index) => value === right.domains[index])
  && left.partition_sites.length === right.partition_sites.length
  && left.partition_sites.every((value,index) => value === right.partition_sites[index]); }
function checkedHostname(value) { if (!hostname(value)) throw new Error(); return value.toLowerCase(); }
function hostname(value) { return typeof value === 'string' && value.length > 0 && value.length <= 253
  && /^(?=.{1,253}$)(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)*[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?$/.test(value); }
function checkedText(value,max) { if (!text(value,max)) throw new Error(); return value; }
function text(value,max) { return typeof value === 'string' && value.length > 0 && value.length <= max
  && !/[\x00-\x1f\x7f]/.test(value); }
function relayKey(value) { try { const bytes=base64Bytes(value); return bytes.length === 65 && bytes[0] === 4; }
  catch { return false; } }
function identifier(value) { return typeof value === 'string' && /^[a-fA-F0-9]{32}$/.test(value); }
function count(value) { return Number.isSafeInteger(value) && value >= 0; }
function plain(value) { return typeof value === 'object' && value !== null && !Array.isArray(value); }
function exact(value,fields) { if (!plain(value) || Object.keys(value).length !== fields.length
  || Object.keys(value).some(field => !fields.includes(field))) throw new Error(); }
function base64(bytes) { let binary=''; for (const byte of bytes) binary += String.fromCharCode(byte); return btoa(binary); }
function base64Bytes(value) { if (!text(value,256)) throw new Error(); const binary=atob(value);
  return Uint8Array.from(binary,character => character.charCodeAt(0)); }
function failure(code) { return Object.assign(new Error(code),{code}); }
