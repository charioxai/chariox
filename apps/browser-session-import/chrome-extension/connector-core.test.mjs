import assert from 'node:assert/strict';
import test from 'node:test';
import {acceptAuthenticatedBootstrap, acceptPairingEnvelope, createPairingChallenge, exactHostPermission,
  metadataOnlyDiscovery, publicFailure, publicProgress, publicResult}
  from './connector-core.mjs';

const senderPublicKey = await validP256PublicKey();
const kernelPublicKey = await validP256PublicKey();
const otherKernelPublicKey = await validP256PublicKey();
const now = 2_000_000;

async function validP256PublicKey() {
  const pair = await crypto.subtle.generateKey({name:'ECDH',namedCurve:'P-256'},false,['deriveBits']);
  return Buffer.from(await crypto.subtle.exportKey('raw',pair.publicKey)).toString('base64');
}

const discovered = {currentProfile:true,sourceTabId:12,hostname:'example.com'};
function bootstrap(overrides = {}) {
  return {
    version:1,
    bootstrap_id:'c'.repeat(32),
    connector_sender_public_key:senderPublicKey,
    expires_at_ms:now + 90_000,
    relay_url:'wss://relay.chariox.example/ws',
    daemon_id:'kernel-1',
    kernel_public_key:kernelPublicKey,
    protocol_version:321,
    delivery_capability:{name:'browser_import_final_delivery',version:1},
    source:{current_profile:true,hostname:'example.com',store_id:'0'},
    selection:{session_id:'room-1',attachment_id:'attachment-1',environment_id:'environment-1',
      runtime_generation:2,tab_id:'tab-1',document_revision:3,source_store_id:'0',
      domains:['Example.COM','login.example.com'],partition_sites:[],overwrite:false},
    ...overrides,
  };
}
function pairing(challenge,anchor,overrides = {}) {
  return {
    version:1,
    bootstrap_id:anchor.bootstrapId,
    enrollment_nonce:challenge.enrollment_nonce,
    connector_sender_public_key:senderPublicKey,
    expires_at_ms:now + 60_000,
    relay_url:'wss://relay.chariox.example/ws',
    relay_auth_token:'fixture-token',
    daemon_id:'kernel-1',
    kernel_public_key:kernelPublicKey,
    protocol_version:321,
    delivery_capability:{name:'browser_import_final_delivery',version:1},
    source:{current_profile:true,store_id:'0'},
    selection:{session_id:'room-1',attachment_id:'attachment-1',environment_id:'environment-1',
      runtime_generation:2,tab_id:'tab-1',document_revision:3,source_store_id:'0',
      domains:['Example.COM','login.example.com'],partition_sites:[],overwrite:false},
    ...overrides,
  };
}

test('pairing binds one enrollment nonce, sender identity and selected kernel', () => {
  const anchor = acceptAuthenticatedBootstrap(bootstrap(),{senderPublicKey,discovered,now});
  const challenge = createPairingChallenge({senderPublicKey,nonceBytes:new Uint8Array(16).fill(7),
    discovered,bootstrap:anchor});
  const accepted = acceptPairingEnvelope(pairing(challenge,anchor),challenge,{now});
  assert.equal(accepted.kernelPublicKey,kernelPublicKey);
  assert.equal(accepted.senderPublicKey,senderPublicKey);
  assert.equal(accepted.selection.session_id,'room-1');
  assert.equal(accepted.sourceTabId,12);
  assert.equal(accepted.selection.domains[0],'example.com');
  assert.throws(() => acceptPairingEnvelope(pairing(challenge,anchor),challenge,{now}),
    {code:'connector_pairing_denied'});
});

test('pairing rejects a valid attacker kernel, relay and destination substituted after trusted bootstrap', () => {
  const anchor = acceptAuthenticatedBootstrap(bootstrap(),{senderPublicKey,discovered,now});
  const challenge = createPairingChallenge({senderPublicKey,nonceBytes:new Uint8Array(16).fill(6),
    discovered,bootstrap:anchor});
  const substituted = pairing(challenge,anchor,{kernel_public_key:otherKernelPublicKey,
    relay_url:'wss://attacker.example/ws',daemon_id:'attacker-kernel',
    selection:{...bootstrap().selection,session_id:'attacker-room',environment_id:'attacker-environment'}});
  assert.throws(() => acceptPairingEnvelope(substituted,challenge,{now}),
    {code:'connector_pairing_denied'});
});

test('pairing rejects wrong sender, changed source store, expiry and unsafe relay targets', () => {
  for (const changes of [
    {connector_sender_public_key:otherKernelPublicKey},
    {kernel_public_key:otherKernelPublicKey},
    {expires_at_ms:now - 1},
    {relay_url:'ws://relay.example.test'},
    {relay_url:'wss://relay.example.test/?token=secret'},
    {protocol_version:320},
    {delivery_capability:null},
    {source:{current_profile:true,store_id:'1'}},
  ]) {
    const anchor = acceptAuthenticatedBootstrap(bootstrap(),{senderPublicKey,discovered,now});
    const challenge = createPairingChallenge({senderPublicKey,nonceBytes:new Uint8Array(16).fill(8),
      discovered,bootstrap:anchor});
    assert.throws(() => acceptPairingEnvelope(pairing(challenge,anchor,changes),challenge,{now}),
      {code:'connector_pairing_denied'});
  }
});

test('authenticated bootstrap itself is sender, source and expiry bound', () => {
  assert.equal(acceptAuthenticatedBootstrap(bootstrap(),{senderPublicKey,discovered,now}).kernelPublicKey,
    kernelPublicKey);
  for (const value of [bootstrap({connector_sender_public_key:otherKernelPublicKey}),
    bootstrap({expires_at_ms:now - 1}),bootstrap({protocol_version:320}),
    bootstrap({source:{current_profile:true,hostname:'other.example',store_id:'0'}})]) {
    assert.throws(() => acceptAuthenticatedBootstrap(value,{senderPublicKey,discovered,now}),
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
  assert.deepEqual(publicResult({requestId:'a'.repeat(32),status:'completed',
    domains:[{domain:'example.com',status:'imported'},{domain:'login.example.com',status:'sign_in_required'}]},
  {requestId:'a'.repeat(32),confirmedDomains:['example.com','login.example.com']}),
  {type:'result',requestId:'a'.repeat(32),status:'completed',
    domains:[{domain:'example.com',status:'imported'},{domain:'login.example.com',status:'sign_in_required'}]});
  const failure = publicFailure(Object.assign(new Error('cookie=private-marker'),{code:'unexpected'}));
  assert.deepEqual(failure,{type:'result',status:'failed',code:'connector_unavailable'});
  assert.equal(JSON.stringify(failure).includes('private-marker'),false);
});

test('authoritative results require exact keys and exactly the frozen confirmed domain set', () => {
  const options = {requestId:'a'.repeat(32),confirmedDomains:Object.freeze(['example.com','login.example.com'])};
  const valid = {requestId:'a'.repeat(32),status:'completed',domains:[
    {domain:'login.example.com',status:'unsupported'},{domain:'example.com',status:'imported'}]};
  assert.equal(publicResult(valid,options).status,'completed');
  for (const value of [
    {...valid,extra:true},
    {...valid,domains:[valid.domains[0]]},
    {...valid,domains:[valid.domains[0],valid.domains[0]]},
    {...valid,domains:[valid.domains[0],{domain:'other.example',status:'imported'}]},
    {...valid,domains:[valid.domains[0],{domain:'example.com',status:'success'}]},
  ]) assert.deepEqual(publicResult(value,options),
    {type:'result',status:'failed',code:'connector_unavailable'});
});
