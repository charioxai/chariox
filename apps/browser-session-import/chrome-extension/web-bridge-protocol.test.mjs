import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import test from 'node:test';

import {packageChromeExtension} from './package-extension.mjs';

let packaged,createRelayKeypair,encryptRelayPayload,createPairingChallenge,metadataOnlyDiscovery;
let acceptEncryptedBootstrap,acceptEncryptedPairing,publicBridgeResult;
test.before(async () => {
  assert.ok(process.env.TYPESCRIPT_MODULE,'TYPESCRIPT_MODULE must name an existing TypeScript module');
  const typescript=(await import(process.env.TYPESCRIPT_MODULE)).default;
  packaged=await mkdtemp(path.join(tmpdir(),'chariox-web-bridge-'));
  await packageChromeExtension(packaged,typescript);
  ({createRelayKeypair,encryptRelayPayload}=await load('packages/kernel-client/src/browser-relay-crypto.js'));
  ({createPairingChallenge,metadataOnlyDiscovery}=await load('apps/browser-session-import/chrome-extension/connector-core.mjs',false));
  ({acceptEncryptedBootstrap,acceptEncryptedPairing,publicBridgeResult}=await load(
    'apps/browser-session-import/chrome-extension/web-bridge-protocol.mjs'));
});
test.after(async () => { if (packaged) await rm(packaged,{recursive:true,force:true}); });

const now = 10_000;
const requestId = 'a'.repeat(32);
const operationNonce = 'b'.repeat(32);
const connectorSessionId = 'c'.repeat(32);
const selected = Object.freeze({session_id:'room-1',attachment_id:'attachment-1',
  environment_id:'environment-1',runtime_generation:7,tab_id:'tab-1',document_revision:9,
  source_store_id:'0',domains:Object.freeze(['example.test','login.example.test']),
  partition_sites:Object.freeze([]),overwrite:false});

test('encrypted bridge pins sender, request, nonce, Room, daemon, kernel and selection',async () => {
  const connector = await createRelayKeypair();
  const web = await createRelayKeypair();
  const kernel = await createRelayKeypair();
  const discovered = metadataOnlyDiscovery({sourceTabId:7,url:'https://example.test/private',incognito:false});
  const bootstrap = bootstrapValue(connector.publicKeyBase64,kernel.publicKeyBase64);
  const bootstrapMessage=bridgeMessage(null,kernel.publicKeyBase64);
  const encryptedBootstrap = await encryptRelayPayload(connector.publicKeyBase64,JSON.stringify({
    binding:messageBinding(bootstrapMessage),bootstrap}),web);
  const accepted = await acceptEncryptedBootstrap({...bootstrapMessage,envelope:encryptedBootstrap.payload},{
    connector,discovered,connectorSessionId,now});
  assert.equal(accepted.webSenderPublicKey,web.publicKeyBase64);
  assert.equal(accepted.bootstrap.selection.session_id,'room-1');

  const challenge = createPairingChallenge({senderPublicKey:connector.publicKeyBase64,
    nonceBytes:new Uint8Array(16).fill(7),discovered,bootstrap:accepted.bootstrap});
  const pairingMessage=bridgeMessage(null,kernel.publicKeyBase64,'browser_import.pairing_envelope.v1');
  const encryptedPairing = await encryptRelayPayload(connector.publicKeyBase64,JSON.stringify({
    binding:messageBinding(pairingMessage),
    pairing:pairingValue(challenge,connector.publicKeyBase64,kernel.publicKeyBase64)}),web);
  assert.deepEqual(messageBinding(pairingMessage),accepted.binding);
  assert.equal(encryptedPairing.payload.sender_public_key,accepted.webSenderPublicKey);
  const pairing = await acceptEncryptedPairing({...pairingMessage,envelope:encryptedPairing.payload},{
    connector,challenge,binding:accepted.binding,webSenderPublicKey:accepted.webSenderPublicKey,now});
  assert.equal(pairing.relayAuthToken,'relay-secret');
  assert.equal(JSON.stringify(bridgeMessage(encryptedPairing.payload)).includes('relay-secret'),false);
});

test('encrypted bridge rejects valid but wrong Room, daemon, kernel, nonce and web sender',async () => {
  const connector = await createRelayKeypair();
  const web = await createRelayKeypair();
  const other = await createRelayKeypair();
  const kernel = await createRelayKeypair();
  const discovered = metadataOnlyDiscovery({sourceTabId:7,url:'https://example.test/',incognito:false});
  const baseMessage=bridgeMessage(null,kernel.publicKeyBase64);
  const encrypted = await encryptRelayPayload(connector.publicKeyBase64,JSON.stringify({
    binding:messageBinding(baseMessage),
    bootstrap:bootstrapValue(connector.publicKeyBase64,kernel.publicKeyBase64)}),web);
  for (const changed of [
    {session_id:'room-2'},{daemon_id:'daemon-2'},{kernel_public_key:other.publicKeyBase64},
    {operation_nonce:'d'.repeat(32)},{request_id:'e'.repeat(32)},
  ]) {
    await assert.rejects(acceptEncryptedBootstrap({...baseMessage,envelope:encrypted.payload,...changed},{
      connector,discovered,connectorSessionId,now}),{code:'connector_pairing_denied'});
  }
  const accepted = await acceptEncryptedBootstrap({...baseMessage,envelope:encrypted.payload},{
    connector,discovered,connectorSessionId,now});
  const challenge = createPairingChallenge({senderPublicKey:connector.publicKeyBase64,
    nonceBytes:new Uint8Array(16).fill(3),discovered,bootstrap:accepted.bootstrap});
  const pairingMessage=bridgeMessage(null,kernel.publicKeyBase64,'browser_import.pairing_envelope.v1');
  const fromOther = await encryptRelayPayload(connector.publicKeyBase64,JSON.stringify({
    binding:messageBinding(pairingMessage),
    pairing:pairingValue(challenge,connector.publicKeyBase64,kernel.publicKeyBase64)}),other);
  await assert.rejects(acceptEncryptedPairing({...pairingMessage,envelope:fromOther.payload},{
    connector,challenge,binding:accepted.binding,webSenderPublicKey:web.publicKeyBase64,now}),
  {code:'connector_pairing_denied'});
});

test('bridge result preserves finalized authoritative ordered public domains',() => {
  const result = publicBridgeResult({type:'result',requestId,status:'completed',domains:[
    {domain:'example.test',status:'imported'},
    {domain:'login.example.test',status:'sign_in_required'},
  ]},{requestId,operationNonce,connectorSessionId,binding:binding()});
  assert.deepEqual(result,{type:'browser_import.result.v1',request_id:requestId,
    operation_nonce:operationNonce,connector_session_id:connectorSessionId,...binding(),result:{
      requestId,status:'completed',domains:[
        {domain:'example.test',status:'imported'},
        {domain:'login.example.test',status:'sign_in_required'},
      ],
    }});
  assert.throws(() => publicBridgeResult({type:'result',requestId,status:'completed',domains:[
    {domain:'login.example.test',status:'sign_in_required'},
    {domain:'example.test',status:'imported'},
  ]},{requestId,operationNonce,connectorSessionId,binding:binding(),confirmedDomains:selected.domains}));
});

function bootstrapValue(senderPublicKey,kernelPublicKey) { return {version:1,bootstrap_id:requestId,
  connector_sender_public_key:senderPublicKey,expires_at_ms:now + 60_000,
  relay_url:'wss://relay.example/ws',daemon_id:'daemon-1',kernel_public_key:kernelPublicKey,
  protocol_version:321,delivery_capability:{name:'browser_import_final_delivery',version:1},
  source:{current_profile:true,hostname:'example.test',store_id:'0'},selection:selected}; }
function pairingValue(challenge,senderPublicKey,kernelPublicKey) { return {version:1,bootstrap_id:requestId,
  enrollment_nonce:challenge.enrollment_nonce,connector_sender_public_key:senderPublicKey,
  expires_at_ms:now + 50_000,relay_url:'wss://relay.example/ws',relay_auth_token:'relay-secret',
  daemon_id:'daemon-1',kernel_public_key:kernelPublicKey,protocol_version:321,
  delivery_capability:{name:'browser_import_final_delivery',version:1},
  source:{current_profile:true,store_id:'0'},selection:selected}; }
function binding() { return {target_binding:'binding-1',session_id:'room-1',daemon_id:'daemon-1',
  kernel_public_key:bootstrapKey(),expires_at_ms:now + 60_000}; }
function bootstrapKey() { return 'BBDjFsEd2B4znY9eLUPJWBfJLwJ31gNDNoJ2ofW5ZTscmHFcHjQKkSSCpTDKaXUBf0Q1JvGgwG3PtOfdFWspfxA='; }
function bridgeMessage(envelope,kernelPublicKey,type='browser_import.bootstrap_envelope.v1') { return {type,
  request_id:requestId,operation_nonce:operationNonce,connector_session_id:connectorSessionId,
  target_binding:'binding-1',session_id:'room-1',daemon_id:'daemon-1',
  kernel_public_key:kernelPublicKey,expires_at_ms:now + 60_000,envelope}; }
function messageBinding(value) { const {type,envelope,...binding}=value; return binding; }
function load(relative,bust=true) { const url=pathToFileURL(path.join(packaged,relative)).href;
  return import(bust ? `${url}?v=${crypto.randomUUID()}` : url); }
