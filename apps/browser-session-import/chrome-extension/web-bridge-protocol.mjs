import {decryptRelayPayload} from '../../../packages/kernel-client/src/browser-relay-crypto.js';
import {acceptAuthenticatedBootstrap,acceptPairingEnvelope} from './connector-core.mjs';

const denied = () => Object.assign(new Error('connector_pairing_denied'),{code:'connector_pairing_denied'});
const bindingKeys = ['request_id','operation_nonce','connector_session_id','target_binding',
  'session_id','daemon_id','kernel_public_key','expires_at_ms'];

export async function acceptEncryptedBootstrap(message,{connector,discovered,connectorSessionId,now=Date.now()}) {
  try {
    const value = bridgeEnvelope(message,'browser_import.bootstrap_envelope.v1');
    if (value.connector_session_id !== connectorSessionId || !validConnector(connector)) throw denied();
    const decoded = JSON.parse(await decryptRelayPayload(connector.privateKey,value.envelope));
    if (!plain(decoded) || !exact(decoded,['binding','bootstrap'])
        || !plain(decoded.binding) || !exact(decoded.binding,bindingKeys)
        || !sameBinding(decoded.binding,value)) throw denied();
    const bootstrap = acceptAuthenticatedBootstrap(decoded.bootstrap,{senderPublicKey:connector.publicKeyBase64,
      discovered,now});
    if (bootstrap.bootstrapId !== value.request_id || bootstrap.expiresAtMs !== value.expires_at_ms
        || bootstrap.daemonId !== value.daemon_id || bootstrap.kernelPublicKey !== value.kernel_public_key
        || bootstrap.selection.session_id !== value.session_id) throw denied();
    return Object.freeze({bootstrap,webSenderPublicKey:value.envelope.sender_public_key,
      binding:freezeBinding(value)});
  } catch { throw denied(); }
}

export async function acceptEncryptedPairing(message,{connector,challenge,binding,webSenderPublicKey,now=Date.now()}) {
  try {
    const value = bridgeEnvelope(message,'browser_import.pairing_envelope.v1');
    if (!sameBinding(value,binding) || !validConnector(connector)
        || value.envelope.sender_public_key !== webSenderPublicKey) throw denied();
    const decoded = JSON.parse(await decryptRelayPayload(connector.privateKey,value.envelope,webSenderPublicKey));
    if (!plain(decoded) || !exact(decoded,['binding','pairing'])
        || !plain(decoded.binding) || !exact(decoded.binding,bindingKeys)
        || !sameBinding(decoded.binding,value)) throw denied();
    const pairing = acceptPairingEnvelope(decoded.pairing,challenge,{now});
    if (pairing.daemonId !== binding.daemon_id || pairing.kernelPublicKey !== binding.kernel_public_key
        || pairing.selection.session_id !== binding.session_id) throw denied();
    return pairing;
  } catch { throw denied(); }
}

export function publicBridgeProgress(progress,context) {
  const binding = bridgeContext(context);
  if (!plain(progress) || !exact(progress,['type','phase','completedDomains','totalDomains'])
      || progress.type !== 'progress') throw denied();
  return Object.freeze({type:'browser_import.progress.v1',...binding,phase:progress.phase,
    completed_domains:progress.completedDomains,total_domains:progress.totalDomains});
}

export function publicBridgeResult(result,context) {
  const binding = bridgeContext(context);
  const confirmed = context.confirmedDomains ?? result?.domains?.map(item => item?.domain);
  if (!plain(result) || !exact(result,['type','requestId','status','domains'])
      || result.type !== 'result' || result.requestId !== context.requestId || result.status !== 'completed'
      || !Array.isArray(result.domains) || !Array.isArray(confirmed)
      || result.domains.length !== confirmed.length || result.domains.length < 1 || result.domains.length > 32) {
    throw denied();
  }
  const domains = result.domains.map((item,index) => {
    if (!plain(item) || !exact(item,['domain','status']) || item.domain !== confirmed[index]
        || !hostname(item.domain) || !['imported','sign_in_required','unsupported'].includes(item.status)) throw denied();
    return Object.freeze({domain:item.domain,status:item.status});
  });
  return Object.freeze({type:'browser_import.result.v1',...binding,
    result:Object.freeze({requestId:result.requestId,status:'completed',domains:Object.freeze(domains)})});
}

export function publicBridgeFailure(code,context) {
  const allowed = new Set(['connector_pairing_denied','cookie_source_denied','cookie_source_cancelled',
    'cookie_source_timeout','cookie_source_unavailable','cookie_source_too_large',
    'browser_import_delivery_unavailable','browser_import_cancellation_unconfirmed','connector_unavailable']);
  return Object.freeze({type:'browser_import.failure.v1',...bridgeContext(context),
    code:allowed.has(code) ? code : 'connector_unavailable'});
}

export function bridgeContext({requestId,operationNonce,connectorSessionId,binding}) {
  const value = {request_id:requestId,operation_nonce:operationNonce,
    connector_session_id:connectorSessionId,...binding};
  if (!identifier(value.request_id) || !identifier(value.operation_nonce)
      || !identifier(value.connector_session_id) || !text(value.target_binding,512)
      || !text(value.session_id,512) || !text(value.daemon_id,512)
      || !relayKey(value.kernel_public_key) || !Number.isSafeInteger(value.expires_at_ms)) throw denied();
  return value;
}

function bridgeEnvelope(value,type) {
  if (!plain(value) || !exact(value,['type',...bindingKeys,'envelope']) || value.type !== type
      || !identifier(value.request_id) || !identifier(value.operation_nonce)
      || !identifier(value.connector_session_id) || !text(value.target_binding,512)
      || !text(value.session_id,512) || !text(value.daemon_id,512)
      || !relayKey(value.kernel_public_key) || !Number.isSafeInteger(value.expires_at_ms)
      || !encrypted(value.envelope)) throw denied();
  return value;
}
function freezeBinding(value) { return Object.freeze({request_id:value.request_id,
  operation_nonce:value.operation_nonce,connector_session_id:value.connector_session_id,
  target_binding:value.target_binding,session_id:value.session_id,daemon_id:value.daemon_id,
  kernel_public_key:value.kernel_public_key,expires_at_ms:value.expires_at_ms}); }
function sameBinding(left,right) { return plain(left) && plain(right)
  && bindingKeys.every(key => left[key] === right[key]); }
function validConnector(value) { return value?.privateKey?.type === 'private'
  && value.privateKey.extractable === false && value.privateKey.algorithm?.name === 'ECDH'
  && value.privateKey.algorithm?.namedCurve === 'P-256' && relayKey(value.publicKeyBase64); }
function encrypted(value) { return plain(value) && exact(value,['sender_public_key','nonce','ciphertext'])
  && relayKey(value.sender_public_key) && text(value.nonce,64) && text(value.ciphertext,131072); }
function relayKey(value) { try { const bytes=BufferLike(value); return bytes.length===65 && bytes[0]===4; } catch { return false; } }
function BufferLike(value) { if (typeof value !== 'string' || value.length > 256) throw new Error();
  const binary=atob(value); return Uint8Array.from(binary,character=>character.charCodeAt(0)); }
function identifier(value) { return typeof value === 'string' && /^[a-fA-F0-9]{32}$/.test(value); }
function hostname(value) { return typeof value === 'string' && value.length > 0 && value.length <= 253
  && /^(?=.{1,253}$)(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)*[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?$/.test(value); }
function text(value,max) { return typeof value === 'string' && value.length > 0 && value.length <= max
  && !/[\x00-\x1f\x7f]/.test(value); }
function plain(value) { return typeof value === 'object' && value !== null && !Array.isArray(value); }
function exact(value,keys) { const actual=Object.keys(value); return actual.length===keys.length
  && keys.every(key=>Object.hasOwn(value,key)); }
