import assert from 'node:assert/strict';
import test from 'node:test';
import {readKernelApprovedChromeCookies} from './kernel-source-reader.mjs';

const requestId = 'a'.repeat(32);
function fixture() {
  const selection = {session_id:'room-1', attachment_id:'attachment-1', environment_id:'environment-1',
    runtime_generation:1, tab_id:'tab-1', document_revision:1, source_store_id:'normal',
    domains:['example.test'], partition_sites:[], overwrite:false};
  const requests = [];
  const reads = [];
  const chrome = {
    tabs:{get:async () => ({id:7,incognito:false})},
    permissions:{contains:async () => true},
    cookies:{getAllCookieStores:async () => [{id:'normal',tabIds:[7]}],
      getAll:async details => {
        reads.push(details);
        return [{name:'session',value:'fixture-secret',domain:'example.test',path:'/',
          secure:true,httpOnly:true,hostOnly:true,session:true,sameSite:'lax',storeId:'normal'}];
      }},
  };
  const request = async payload => {
    requests.push(structuredClone(payload));
    return {BrowserImportConsent:{request_id:requestId,
      status:payload.ClaimBrowserImportSource ? 'source_claimed' : 'source_authorized'}};
  };
  return {requests,reads,options:{chrome,requestId,selection,sourceTabId:7,request}};
}

test('kernel claim and current authorization guard the real cookie reader', async () => {
  const {requests,reads,options} = fixture();
  options.selection.cookies = [{value:'must-not-travel'}];
  const result = await readKernelApprovedChromeCookies(options);
  assert.equal(result.cookies[0].value,'fixture-secret');
  assert.deepEqual(reads,[{domain:'example.test',storeId:'normal'}]);
  assert.equal(requests.filter(r => r.ClaimBrowserImportSource).length,1);
  assert.ok(requests.slice(1).every(r => r.AuthorizeBrowserImportSource));
  assert.equal(JSON.stringify(requests).includes('fixture-secret'),false);
  assert.equal(JSON.stringify(requests).includes('must-not-travel'),false);
  assert.equal(requests.at(-1).AuthorizeBrowserImportSource.request_id,requestId);
});

test('the reader owns one transport signal and releases it on success or failure', async () => {
  for (const fail of [false,true]) {
    const {options} = fixture();
    const caller = new AbortController();
    options.signal = caller.signal;
    const send = options.request;
    const signals = [];
    options.request = (payload,transport) => {
      signals.push(transport.signal);
      assert.equal(transport.signal.aborted,false);
      if (fail) throw new Error('private-marker');
      return send(payload);
    };
    const pending = readKernelApprovedChromeCookies(options);
    if (fail) await assert.rejects(pending,{code:'cookie_source_unavailable'});
    else await pending;
    assert.ok(signals.length > 0);
    assert.equal(new Set(signals).size,1);
    assert.equal(signals[0].aborted,true);
    assert.equal(caller.signal.aborted,false);
  }
});

test('late claim replies cannot issue more requests after timeout or cancellation', async () => {
  for (const mode of ['timeout','cancel']) {
    const {options,reads} = fixture();
    const controller = new AbortController();
    options.signal = controller.signal;
    options.timeoutMs = mode === 'timeout' ? 20 : 30000;
    const calls = [];
    let reply;
    let began;
    const started = new Promise(resolve => {began = resolve;});
    options.request = payload => {
      calls.push(payload);
      began();
      return new Promise(resolve => {reply = resolve;});
    };
    const pending = readKernelApprovedChromeCookies(options);
    await started;
    if (mode === 'cancel') controller.abort('private-marker');
    await assert.rejects(pending,{code:mode === 'timeout' ? 'cookie_source_timeout' : 'cookie_source_cancelled'});
    reply({BrowserImportConsent:{request_id:requestId,status:'source_claimed'}});
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(calls.length,1,mode);
    assert.equal(reads.length,0);
  }
});

test('invalid request identifiers or metadata never reach the transport or Chrome', async () => {
  for (const badId of [undefined,'','request-1','g'.repeat(32)]) {
    const {options,requests,reads} = fixture();
    options.requestId = badId;
    await assert.rejects(readKernelApprovedChromeCookies(options),{code:'cookie_source_denied'});
    assert.equal(requests.length,0);
    assert.equal(reads.length,0);
  }
  const {options,requests,reads} = fixture();
  options.selection = undefined;
  await assert.rejects(readKernelApprovedChromeCookies(options),{code:'cookie_source_denied'});
  assert.equal(requests.length,0);
  assert.equal(reads.length,0);
});

test('booleans, wrong ids, wrong phases and transport failures never authorize source access', async () => {
  for (const stage of ['claim','authorize']) {
    for (const mode of ['boolean','wrong-id','wrong-status','empty','error']) {
      const {options,reads} = fixture();
      const send = options.request;
      options.request = async payload => {
        const target = stage === 'claim' ? payload.ClaimBrowserImportSource : payload.AuthorizeBrowserImportSource;
        if (!target) return send(payload);
        if (mode === 'error') throw new Error('private-marker');
        if (mode === 'boolean') return true;
        if (mode === 'empty') return {};
        return {BrowserImportConsent:{request_id:mode === 'wrong-id' ? 'b'.repeat(32) : requestId,
          status:mode === 'wrong-id' ? (stage === 'claim' ? 'source_claimed' : 'source_authorized') : 'approved'}};
      };
      await assert.rejects(readKernelApprovedChromeCookies(options),error => {
        assert.equal(String(error).includes('private-marker'),false);
        return ['cookie_source_denied','cookie_source_unavailable'].includes(error.code);
      });
      assert.equal(reads.length,0,`${stage}/${mode}`);
    }
  }
});

test('kernel denial during a Chrome query discards the cookie batch', async () => {
  const {options} = fixture();
  const send = options.request;
  const read = options.chrome.cookies.getAll;
  let revoked = false;
  options.chrome.cookies.getAll = async details => {revoked = true; return read(details);};
  options.request = payload => revoked
    ? Promise.resolve({BrowserImportConsent:{request_id:requestId,status:'cancelled'}})
    : send(payload);
  await assert.rejects(readKernelApprovedChromeCookies(options),{code:'cookie_source_denied'});
});

test('source selection is snapshotted before the first kernel round trip', async () => {
  const {options,requests,reads} = fixture();
  const send = options.request;
  options.request = payload => {
    options.selection.domains.push('unapproved.test');
    options.selection.source_store_id = 'other-store';
    options.selection.overwrite = true;
    return send(payload);
  };
  await readKernelApprovedChromeCookies(options);
  assert.deepEqual(reads,[{domain:'example.test',storeId:'normal'}]);
  for (const request of requests) {
    const scope = Object.values(request)[0].selection;
    assert.deepEqual(scope.domains,['example.test']);
    assert.equal(scope.source_store_id,'normal');
    assert.equal(scope.overwrite,false);
  }
});
