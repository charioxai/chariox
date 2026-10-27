import assert from 'node:assert/strict';
import test from 'node:test';
import {createRelayKeypair, encryptRelayPayload} from '../../packages/kernel-client/src/browser-relay-crypto.ts';
import {decryptBrowserImportResponse} from './relay-response.mjs';
import {readKernelApprovedChromeCookies} from './kernel-source-reader.mjs';

test('an import reply is bound to one encrypted request, including retries of a command', async () => {
  const sender = await createRelayKeypair();
  const kernel = await createRelayKeypair();
  const first = await encryptRelayPayload(kernel.publicKeyBase64, 'same-command', sender);
  const retry = await encryptRelayPayload(kernel.publicKeyBase64, 'same-command', sender);
  assert.notEqual(first.payload.nonce, retry.payload.nonce);
  const response = {BrowserImportConsent:{request_id:'a'.repeat(32),status:'source_authorized'}};
  const reply = await encryptRelayPayload(sender.publicKeyBase64,
    JSON.stringify({request_nonce:first.payload.nonce,response}), kernel);
  await assert.rejects(decryptBrowserImportResponse(sender.privateKey, reply.payload,
    kernel.publicKeyBase64, retry.payload.nonce), {message:'browser import response denied'});
  assert.deepEqual(await decryptBrowserImportResponse(sender.privateKey, reply.payload,
    kernel.publicKeyBase64, first.payload.nonce), response);
});

test('replaying a pre-read authorization cannot release cookies after a query', async () => {
  const sender = await createRelayKeypair();
  const kernel = await createRelayKeypair();
  const requestId = 'b'.repeat(32);
  let oldAuthorization;
  let queried = false;
  const request = async metadata => {
    const outgoing = await encryptRelayPayload(kernel.publicKeyBase64, JSON.stringify(metadata), sender);
    const response = {BrowserImportConsent:{request_id:requestId,
      status:metadata.ClaimBrowserImportSource ? 'source_claimed' : 'source_authorized'}};
    const current = await encryptRelayPayload(sender.publicKeyBase64,
      JSON.stringify({request_nonce:outgoing.payload.nonce,response}), kernel);
    if (metadata.AuthorizeBrowserImportSource && !queried) oldAuthorization = current.payload;
    return decryptBrowserImportResponse(sender.privateKey,
      queried ? oldAuthorization : current.payload, kernel.publicKeyBase64, outgoing.payload.nonce);
  };
  const chrome = {
    tabs:{get:async () => ({id:7,incognito:false})},
    permissions:{contains:async () => true},
    cookies:{getAllCookieStores:async () => [{id:'normal',tabIds:[7]}],getAll:async () => {
      queried = true;
      return [{name:'session',value:'fixture-private-cookie',domain:'example.test',path:'/',
        secure:true,httpOnly:true,hostOnly:true,session:true,sameSite:'lax',storeId:'normal'}];
    }},
  };
  await assert.rejects(readKernelApprovedChromeCookies({chrome,requestId,request,sourceTabId:7,
    selection:{session_id:'room-1',attachment_id:'attachment-1',environment_id:'environment-1',
      runtime_generation:1,tab_id:'tab-1',document_revision:1,source_store_id:'normal',
      domains:['example.test'],partition_sites:[],overwrite:false}}), error => {
    assert.ok(['cookie_source_denied','cookie_source_unavailable'].includes(error.code));
    assert.equal(String(error).includes('fixture-private-cookie'),false);
    return true;
  });
  assert.equal(queried,true);
  assert.ok(oldAuthorization);
});

test('unbound legacy replies, wrong kernels and invalid context fail without leaking contents', async () => {
  const sender = await createRelayKeypair();
  const kernel = await createRelayKeypair();
  const other = await createRelayKeypair();
  const requestNonce = 'AAAAAAAAAAAAAAAA';
  const privateMarker = 'not-a-real-private-fixture';
  for (const [body, source, expectedKey, nonce] of [
    [{BrowserImportConsent:{status:privateMarker}}, kernel, kernel.publicKeyBase64, requestNonce],
    [{request_nonce:requestNonce,response:privateMarker}, other, kernel.publicKeyBase64, requestNonce],
    [{request_nonce:requestNonce}, kernel, kernel.publicKeyBase64, requestNonce],
    [{request_nonce:requestNonce,response:privateMarker}, kernel, undefined, requestNonce],
    [{request_nonce:requestNonce,response:privateMarker}, kernel, kernel.publicKeyBase64, ''],
    [null, kernel, kernel.publicKeyBase64, requestNonce],
  ]) {
    const reply = await encryptRelayPayload(sender.publicKeyBase64, JSON.stringify(body), source);
    await assert.rejects(decryptBrowserImportResponse(sender.privateKey, reply.payload, expectedKey, nonce), error => {
      assert.equal(error.message, 'browser import response denied');
      assert.equal(error.cause, undefined);
      assert.equal(String(error).includes(privateMarker), false);
      return true;
    });
  }
});
