import {createRelayKeypair,relayPublicKeyThumbprint}
  from '../../../packages/kernel-client/src/browser-relay-crypto.js';
import {prepareChromeCookieImport} from '../chrome-import-consent-flow.mjs';
import {connectBrowserImportRelay} from '../relay-connector.mjs';
import {acceptPairingEnvelope,createPairingChallenge,metadataOnlyDiscovery,
  publicFailure,publicProgress,publicResult} from './connector-core.mjs';
import {deliverBrowserImport} from './delivery-adapter.mjs';

const element = id => document.getElementById(id);
const sourceText = element('source');
const enrollSection = element('enroll');
const confirmSection = element('confirm');
const requestText = element('request');
const responseText = element('response');
const statusText = element('message');
const lifetime = new AbortController();
let flow;
let relay;
let pairing;
let busy = false;

void initialize();

async function initialize() {
  try {
    const sourceTabId = Number(new URL(location.href).searchParams.get('sourceTabId'));
    const tab = await chrome.tabs.get(sourceTabId);
    const discovered = metadataOnlyDiscovery({sourceTabId,url:tab.url,incognito:tab.incognito});
    const sender = await createRelayKeypair();
    const challenge = createPairingChallenge({senderPublicKey:sender.publicKeyBase64,
      nonceBytes:crypto.getRandomValues(new Uint8Array(16)),discovered});
    const thumbprint = await relayPublicKeyThumbprint(sender.publicKeyBase64);
    const enrollmentRequest = {version:1,enrollment_nonce:challenge.enrollment_nonce,
      connector_sender_public_key:challenge.connector_sender_public_key,
      connector_sender_thumbprint:thumbprint,
      source:{current_profile:true,hostname:discovered.hostname,store_id:'0'}};
    requestText.value = JSON.stringify(enrollmentRequest,null,2);
    sourceText.textContent = `Current Chrome profile · ${discovered.hostname}`;
    enrollSection.hidden = false;
    element('copy').addEventListener('click',() => void navigator.clipboard.writeText(requestText.value));
    element('pair').addEventListener('click',() => void pairDestination(challenge,sender));
  } catch (error) { show(publicFailure(error)); }
}

async function pairDestination(challenge,sender) {
  if (busy || pairing) return;
  busy = true;
  try {
    let envelope;
    try {
      if (responseText.value.length > 65536) throw denied();
      envelope = JSON.parse(responseText.value);
    }
    finally { responseText.value = ''; }
    pairing = acceptPairingEnvelope(envelope,challenge);
    if (pairing.senderPublicKey !== sender.publicKeyBase64) throw denied();
    let relayToken = pairing.relayAuthToken;
    relay = await connectBrowserImportRelay({relayUrl:pairing.relayUrl,authToken:relayToken,
      daemonId:pairing.daemonId,kernelPublicKey:pairing.kernelPublicKey,sender,
      protocolVersion:pairing.protocolVersion,signal:lifetime.signal});
    relayToken = '';
    const retainedPairing = {...pairing};
    delete retainedPairing.relayAuthToken;
    pairing = Object.freeze(retainedPairing);
    const kernelThumbprint = await relayPublicKeyThumbprint(pairing.kernelPublicKey);
    flow = await prepareChromeCookieImport({chrome,selection:pairing.selection,
      sourceTabId:pairing.sourceTabId,request:relay.request,signal:lifetime.signal});
    element('destination').textContent = `Destination session ${pairing.selection.session_id}, environment ${pairing.selection.environment_id}, kernel ${kernelThumbprint}`;
    const list = element('domains');
    for (const domain of pairing.selection.domains) {
      const item = document.createElement('li');
      item.textContent = domain;
      list.append(item);
    }
    enrollSection.hidden = true;
    confirmSection.hidden = false;
    statusText.textContent = 'Review the destination and exact hosts before continuing.';
  } catch (error) {
    pairing = undefined;
    relay?.close();
    relay = undefined;
    show(publicFailure(error));
  } finally { busy = false; }
}

// This handler must remain a direct click handler. confirmAndRead synchronously
// invokes chrome.permissions.request before its first await so Chrome retains
// the user's gesture for the exact host grant.
element('start').addEventListener('click',() => {
  if (busy || !flow) return;
  busy = true;
  show(publicProgress('requesting_permission',0,pairing.selection.domains.length));
  const reading = flow.confirmAndRead();
  void finishImport(reading);
});

async function finishImport(reading) {
  let source;
  try {
    show(publicProgress('verifying_consent',0,pairing.selection.domains.length));
    source = await reading;
    show(publicProgress('transferring',pairing.selection.domains.length,pairing.selection.domains.length));
    const result = await deliverBrowserImport({requestId:flow.requestId,pairing,selection:flow.selection,
      batch:source,onProgress:show,signal:lifetime.signal});
    show(publicResult(result));
  } catch (error) {
    await flow?.cancel();
    show(publicFailure(error));
  } finally {
    source = undefined;
    busy = false;
  }
}

element('cancel').addEventListener('click',() => void cancelImport());
addEventListener('pagehide',() => { lifetime.abort(); void flow?.cancel(); relay?.close(); },{once:true});

async function cancelImport() {
  if (busy && !flow) return;
  lifetime.abort();
  await flow?.cancel();
  relay?.close();
  show({type:'result',status:'failed',code:'cookie_source_cancelled'});
  element('start').disabled = true;
  element('cancel').disabled = true;
}

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
  if (code === 'cookie_source_timeout') return 'Import expired. Start again.';
  if (code === 'connector_pairing_denied') return 'Pairing was rejected. Create a new pairing response in Chariox.';
  return 'Import could not continue. No cookies were delivered.';
}

function denied() { return Object.assign(new Error('connector_pairing_denied'),{code:'connector_pairing_denied'}); }
