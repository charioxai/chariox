import {createRelayKeypair,relayPublicKeyThumbprint}
  from '../../../packages/kernel-client/src/browser-relay-crypto.js';
import {prepareChromeCookieImport} from '../chrome-import-consent-flow.mjs';
import {connectBrowserImportRelay} from '../relay-connector.mjs';
import {createPairingChallenge,metadataOnlyDiscovery,
  publicFailure,publicProgress,publicResult} from './connector-core.mjs';
import {deliverBrowserImport} from './delivery-adapter.mjs';
import {createChromePermissionLifecycle} from './permission-coordinator.mjs';
import {acceptEncryptedBootstrap,acceptEncryptedPairing,publicBridgeFailure,
  publicBridgeProgress,publicBridgeResult} from './web-bridge-protocol.mjs';

const element = id => document.getElementById(id);
const sourceText = element('source');
const pairingSection = element('pairing');
const confirmSection = element('confirm');
const statusText = element('message');
const lifetime = new AbortController();
let flow;
let relay;
let pairing;
let busy = false;
let sender;
let discovered;
let challenge;
let webPort;
let webSession;

void initialize();

async function initialize() {
  try {
    const sourceTabId = Number(new URL(location.href).searchParams.get('sourceTabId'));
    const tab = await chrome.tabs.get(sourceTabId);
    discovered = metadataOnlyDiscovery({sourceTabId,url:tab.url,incognito:tab.incognito});
    sender = await createRelayKeypair();
    sourceText.textContent = `Current Chrome profile · ${discovered.hostname}`;
    connectWebBridge();
  } catch (error) { show(publicFailure(error)); }
}

async function establishPairing() {
    if (pairing.senderPublicKey !== sender.publicKeyBase64) throw denied();
    let relayToken = pairing.relayAuthToken;
    relay = await connectBrowserImportRelay({relayUrl:pairing.relayUrl,authToken:relayToken,
      daemonId:pairing.daemonId,kernelPublicKey:pairing.kernelPublicKey,sender,
      protocolVersion:pairing.protocolVersion,signal:lifetime.signal});
    relayToken = '';
    const retainedPairing = {...pairing};
    delete retainedPairing.relayAuthToken;
    pairing = Object.freeze(retainedPairing);
    const permissionLifecycle = createChromePermissionLifecycle(chrome);
    flow = await prepareChromeCookieImport({chrome,selection:pairing.selection,
      sourceTabId:pairing.sourceTabId,request:relay.request,signal:lifetime.signal,permissionLifecycle});
    const list = element('domains');
    for (const domain of pairing.selection.domains) {
      const item = document.createElement('li');
      item.textContent = domain;
      list.append(item);
    }
    pairingSection.hidden = true;
    confirmSection.hidden = false;
    statusText.textContent = 'Review the destination and exact hosts before continuing.';
    if (webSession?.binding) sendWeb({type:'browser_import.ready.v1',...webBinding()});
}

async function pairingFailed(error) {
  await flow?.cancel();
  flow = undefined;
  pairing = undefined;
  relay?.close();
  relay = undefined;
  const failure=publicFailure(error);
  show(failure);
  sendWebFailure(failure.code);
}

// This handler must remain a direct click handler. confirmAndDeliver synchronously
// invokes chrome.permissions.request before its first await so Chrome retains
// the user's gesture for the exact host grant.
element('start').addEventListener('click',() => {
  if (busy || !flow) return;
  busy = true;
  const progress=publicProgress('requesting_permission',0,pairing.selection.domains.length);
  show(progress);
  sendWebProgress(progress);
  const delivery = flow.confirmAndDeliver((value,options) =>
    deliverBrowserImport({deliver:relay.deliver,...value},options));
  void finishImport(delivery);
});

async function finishImport(delivery) {
  try {
    const progress=publicProgress('verifying_consent',0,pairing.selection.domains.length);
    show(progress);
    sendWebProgress(progress);
    const result = await delivery;
    const published=publicResult(result,{requestId:flow.requestId,confirmedDomains:flow.selection.domains});
    show(published);
    sendWebResult(published,flow.selection.domains);
  } catch (error) {
    await flow?.cancel();
    const failure=publicFailure(error);
    show(failure);
    sendWebFailure(failure.code);
  } finally {
    await flow?.releasePermissions();
    busy = false;
  }
}

element('cancel').addEventListener('click',() => void cancelImport());
addEventListener('pagehide',() => { lifetime.abort(); void flow?.cancel(); relay?.close(); webPort?.disconnect(); },{once:true});

async function cancelImport() {
  if (busy && !flow) return;
  lifetime.abort();
  const cancellation = await flow?.cancel();
  relay?.close();
  const code=cancellation?.kernelCancellationConfirmed === true
    ? 'cookie_source_cancelled' : 'browser_import_cancellation_unconfirmed';
  show({type:'result',status:'failed',code});
  element('start').disabled = true;
  element('cancel').disabled = true;
  sendWebFailure(code);
}

function connectWebBridge() {
  try {
    const connectorSessionId=crypto.randomUUID().replaceAll('-','');
    webPort=chrome.runtime.connect({name:'browser-import-web-session-v1'});
    webPort.onMessage.addListener(value=>void receiveWeb(value));
    webPort.onDisconnect.addListener(()=>void webBridgeDisconnected());
    webPort.postMessage({type:'browser_import.connector_ready.v1',connector_session_id:connectorSessionId,
      connector_sender_public_key:sender.publicKeyBase64,
      source:{current_profile:true,hostname:discovered.hostname,store_id:'0'}});
    webSession={connectorSessionId,binding:null,webSenderPublicKey:null};
  } catch { webPort=undefined; webSession=undefined; }
}

async function webBridgeDisconnected() {
  if (!webSession) return;
  webSession=undefined;
  lifetime.abort();
  await flow?.cancel();
  await flow?.releasePermissions();
  relay?.close();
  element('start').disabled=true;
  show({type:'result',status:'failed',code:'connector_unavailable'});
}

async function receiveWeb(value) {
  if (!webSession || busy) return;
  if (value?.type === 'browser_import.cancel.v1') {
    if (webSession.binding && value.request_id===webSession.binding.request_id
        && value.operation_nonce===webSession.binding.operation_nonce
        && value.connector_session_id===webSession.connectorSessionId) await cancelImport();
    return;
  }
  busy=true;
  try {
    if (value?.type === 'browser_import.bootstrap_envelope.v1' && !webSession.binding) {
      const accepted=await acceptEncryptedBootstrap(value,{connector:sender,discovered,
        connectorSessionId:webSession.connectorSessionId});
      webSession.binding=accepted.binding;
      webSession.webSenderPublicKey=accepted.webSenderPublicKey;
      challenge=createPairingChallenge({senderPublicKey:sender.publicKeyBase64,
        nonceBytes:crypto.getRandomValues(new Uint8Array(16)),discovered,bootstrap:accepted.bootstrap});
      const kernelThumbprint=await relayPublicKeyThumbprint(accepted.bootstrap.kernelPublicKey);
      element('destination').textContent=`Pinned session ${accepted.bootstrap.selection.session_id}, environment ${accepted.bootstrap.selection.environment_id}, kernel ${kernelThumbprint}`;
      pairingSection.hidden=false;
      sendWeb({type:'browser_import.challenge.v1',...webBinding(),challenge:{version:1,
        bootstrap_id:challenge.bootstrap_id,enrollment_nonce:challenge.enrollment_nonce,
        connector_sender_public_key:challenge.connector_sender_public_key}});
    } else if (value?.type === 'browser_import.pairing_envelope.v1' && webSession.binding && !pairing) {
      pairing=await acceptEncryptedPairing(value,{connector:sender,challenge,binding:webSession.binding,
        webSenderPublicKey:webSession.webSenderPublicKey});
      await establishPairing();
    } else throw denied();
  } catch (error) { await pairingFailed(error); }
  finally { busy=false; }
}

function webBinding() {
  if (!webSession?.binding) throw denied();
  return webSession.binding;
}
function webContext() {
  const binding=webSession?.binding;
  if (!binding) throw denied();
  return {requestId:binding.request_id,operationNonce:binding.operation_nonce,
    connectorSessionId:binding.connector_session_id,binding};
}
function sendWeb(value) { try { if (webPort && webSession?.binding) webPort.postMessage(value); } catch { /* disconnected */ } }
function sendWebProgress(value) { if (webSession?.binding) sendWeb(publicBridgeProgress(value,webContext())); }
function sendWebResult(value,confirmedDomains) { if (webSession?.binding) sendWeb(publicBridgeResult(value,
  {...webContext(),confirmedDomains})); }
function sendWebFailure(code) { if (webSession?.binding) sendWeb(publicBridgeFailure(code,webContext())); }

function show(message) {
  if (message.type === 'progress') {
    statusText.textContent = `${message.phase.replaceAll('_',' ')} (${message.completedDomains}/${message.totalDomains})`;
    return;
  }
  statusText.textContent = message.status === 'completed' ? 'Import completed.' : failureLabel(message.code);
  const result = element('result');
  result.replaceChildren();
  for (const entry of message.domains ?? []) {
    const item = document.createElement('li');
    item.textContent = `${entry.domain}: ${entry.status.replaceAll('_',' ')}`;
    result.append(item);
  }
}

function failureLabel(code) {
  if (code === 'browser_import_delivery_unavailable') return 'This connector needs the production browser-import runtime delivery command.';
  if (code === 'cookie_source_cancelled') return 'Import cancelled.';
  if (code === 'browser_import_cancellation_unconfirmed') return 'Cancellation could not be confirmed. The Environment remains quarantined pending recovery.';
  if (code === 'cookie_source_timeout') return 'Import expired. Start again.';
  if (code === 'connector_pairing_denied') return 'Pairing was rejected. Create a new pairing response in Chariox.';
  return 'Import could not continue. No cookies were delivered.';
}

function denied() { return Object.assign(new Error('connector_pairing_denied'),{code:'connector_pairing_denied'}); }
