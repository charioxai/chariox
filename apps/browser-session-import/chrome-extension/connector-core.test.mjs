import assert from 'node:assert/strict';
import test from 'node:test';
import {acceptPairingEnvelope, createPairingChallenge, exactHostPermission,
  metadataOnlyDiscovery, publicFailure, publicProgress, publicResult}
  from './connector-core.mjs';

const senderPublicKey = 'B'.repeat(88);
const kernelPublicKey = 'K'.repeat(88);
const now = 2_000_000;

function pairing(challenge, overrides = {}) {
  return {
    version:1,
    enrollment_nonce:challenge.enrollment_nonce,
    connector_sender_public_key:senderPublicKey,
    expires_at_ms:now + 60_000,
    relay_url:'wss://relay.chariox.example/ws',
    relay_auth_token:'fixture-token',
    daemon_id:'kernel-1',
    kernel_public_key:kernelPublicKey,
    protocol_version:320,
    delivery_capability:{name:'browser_import_final_delivery',version:1},
    source:{current_profile:true,store_id:'0'},
    selection:{session_id:'room-1',attachment_id:'attachment-1',environment_id:'environment-1',
      runtime_generation:2,tab_id:'tab-1',document_revision:3,source_store_id:'0',
      domains:['Example.COM','login.example.com'],partition_sites:[],overwrite:false},
    ...overrides,
  };
}

test('pairing binds one enrollment nonce, sender identity and selected kernel', () => {
  const challenge = createPairingChallenge({senderPublicKey, nonceBytes:new Uint8Array(16).fill(7),
    discovered:{sourceTabId:12,hostname:'example.com'}});
  const accepted = acceptPairingEnvelope(pairing(challenge),challenge,{now});
  assert.equal(accepted.kernelPublicKey,kernelPublicKey);
  assert.equal(accepted.senderPublicKey,senderPublicKey);
  assert.equal(accepted.selection.session_id,'room-1');
  assert.equal(accepted.sourceTabId,12);
  assert.equal(accepted.selection.domains[0],'example.com');
  assert.throws(() => acceptPairingEnvelope(pairing(challenge),challenge,{now}),
    {code:'connector_pairing_denied'});
});

test('pairing rejects wrong sender, changed source store, expiry and unsafe relay targets', () => {
  for (const changes of [
    {connector_sender_public_key:'other'},
    {kernel_public_key:'other'},
    {expires_at_ms:now - 1},
    {relay_url:'ws://relay.example.test'},
    {relay_url:'wss://relay.example.test/?token=secret'},
    {protocol_version:319},
    {delivery_capability:null},
    {source:{current_profile:true,store_id:'1'}},
  ]) {
    const challenge = createPairingChallenge({senderPublicKey,nonceBytes:new Uint8Array(16).fill(8),
      discovered:{sourceTabId:12,hostname:'example.com'},expectedKernelPublicKey:kernelPublicKey});
    assert.throws(() => acceptPairingEnvelope(pairing(challenge,changes),challenge,{now}),
      {code:'connector_pairing_denied'});
  }
});

test('permissions are narrowed to exact confirmed hosts', () => {
  assert.deepEqual(exactHostPermission(['Example.COM','login.example.com','example.com']),{
    permissions:['cookies'],origins:['*://example.com/*','*://login.example.com/*'],
  });
  for (const domains of [['*.example.com'],['https://example.com'],['example.com/path'],[]]) {
    assert.throws(() => exactHostPermission(domains),{code:'connector_pairing_denied'});
  }
});

test('discovery and public messages are metadata-only and redact arbitrary failures', () => {
  const discovery = metadataOnlyDiscovery({sourceTabId:9,url:'https://accounts.example.com/private/path?token=x',
    incognito:false});
  assert.deepEqual(discovery,{currentProfile:true,sourceTabId:9,hostname:'accounts.example.com'});
  assert.equal(JSON.stringify(discovery).includes('/private/path'),false);
  assert.deepEqual(publicProgress('transferring',1,2),{type:'progress',phase:'transferring',completedDomains:1,totalDomains:2});
  assert.deepEqual(publicResult({domains:['example.com'],imported:1,signInRequired:0}),
    {type:'result',status:'completed',domains:[{domain:'example.com',status:'imported'}]});
  const failure = publicFailure(Object.assign(new Error('cookie=private-marker'),{code:'unexpected'}));
  assert.deepEqual(failure,{type:'result',status:'failed',code:'connector_unavailable'});
  assert.equal(JSON.stringify(failure).includes('private-marker'),false);
});
