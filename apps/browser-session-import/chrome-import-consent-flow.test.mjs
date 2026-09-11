import assert from 'node:assert/strict';
import test from 'node:test';
import {prepareChromeCookieImport} from './chrome-import-consent-flow.mjs';

const id = 'a'.repeat(32);
const reply = status => ({BrowserImportConsent:{request_id:id,status}});
const tick = () => new Promise(resolve => setImmediate(resolve));
function deferred() {
  let resolve;
  const promise = new Promise(done => {resolve = done;});
  return {promise,resolve};
}
function fixture() {
  const calls = [];
  const selection = {session_id:'room',attachment_id:'attachment',environment_id:'environment',
    runtime_generation:1,tab_id:'tab',document_revision:1,source_store_id:'normal',
    domains:['example.test'],partition_sites:[],overwrite:false};
  const chrome = {
    permissions:{request:details => {calls.push(['permission',details]); return Promise.resolve(true);},
      contains:async () => true},
    tabs:{get:async () => ({id:7,incognito:false})},
    cookies:{getAllCookieStores:async () => [{id:'normal',tabIds:[7]}],
      getAll:async details => {
        calls.push(['cookies',details]);
        return [{name:'session',value:'fixture-secret',domain:'example.test',path:'/',
          hostOnly:true,secure:true,httpOnly:true,session:true,sameSite:'lax',storeId:'normal'}];
      }},
  };
  const request = async (value,options) => {
    calls.push([Object.keys(value)[0],structuredClone(value),options]);
    return reply({PrepareBrowserImport:'prepared',ApproveBrowserImport:'approved',
      ClaimBrowserImportSource:'source_claimed',AuthorizeBrowserImportSource:'source_authorized',
      CancelBrowserImport:'cancelled'}[Object.keys(value)[0]]);
  };
  return {calls,options:{chrome,selection,sourceTabId:7,request},
    count:kind => calls.filter(([name]) => name === kind).length};
}

test('preparation reads no cookies, then gesture permissions precede kernel approval and source claim', async () => {
  const f = fixture();
  f.options.selection.cookie = 'must-not-travel';
  const flow = await prepareChromeCookieImport(f.options);
  assert.equal(flow.state,'prepared');
  assert.deepEqual(f.calls.map(([kind]) => kind),['PrepareBrowserImport']);
  assert.equal(Object.isFrozen(flow.selection),true);
  assert.equal(Object.isFrozen(flow.selection.domains),true);
  assert.equal(flow.selection.cookie,undefined);
  const reading = flow.confirmAndRead();
  // The permission API must be called before yielding the confirmation gesture.
  assert.equal(f.count('permission'),1);
  assert.equal(f.count('ApproveBrowserImport'),0);
  assert.deepEqual(f.calls[1][1],{permissions:['cookies'],origins:['*://example.test/*']});
  const result = await reading;
  assert.equal(flow.state,'source_read');
  assert.equal(result.cookies[0].value,'fixture-secret');
  assert.equal(f.count('ClaimBrowserImportSource'),1);
  assert.ok(f.calls.findIndex(([k]) => k === 'ApproveBrowserImport') < f.calls.findIndex(([k]) => k === 'cookies'));
  assert.equal(JSON.stringify(f.calls).includes('fixture-secret'),false);
  assert.equal(JSON.stringify(f.calls).includes('must-not-travel'),false);
  assert.equal(JSON.stringify(flow).includes('fixture-secret'),false);
  assert.equal(f.count('CancelBrowserImport'),0);
  await assert.rejects(flow.confirmAndRead(),{code:'cookie_source_denied'});
  assert.deepEqual(await flow.cancel(),{kernelCancellationConfirmed:true});
});

test('trusted confirmation delivers directly and scrubs the source batch after encrypted handoff', async () => {
  const f = fixture();
  const generatedValue = crypto.randomUUID();
  f.options.chrome.cookies.getAll = async () => [{name:'session',value:generatedValue,
    domain:'example.test',path:'/',hostOnly:true,secure:true,httpOnly:true,session:true,
    sameSite:'lax',storeId:'normal'}];
  let delivered;
  const flow = await prepareChromeCookieImport(f.options);
  const completing = flow.confirmAndDeliver(async value => {
    delivered = value;
    assert.equal(value.requestId,id);
    assert.equal(value.cookies[0].value,generatedValue);
    return {BrowserImportDelivered:{results:[{domain:'example.test',status:'imported',cookie_count:1}]}};
  });
  assert.equal(f.count('permission'),1);
  assert.deepEqual(await completing,{results:[{domain:'example.test',status:'imported',cookie_count:1}]});
  assert.equal(flow.state,'delivered');
  assert.equal(delivered.cookies[0].value,'');
  assert.equal(JSON.stringify(f.calls).includes(generatedValue),false);
});

test('permission lifecycle reserves before confirmation, activates after grant and releases explicitly', async () => {
  const f = fixture();
  const lifecycle = [];
  f.options.permissionLifecycle = {
    reserve:async permission => lifecycle.push(['reserve',permission]),
    activate:async () => lifecycle.push(['activate']),
    release:async () => lifecycle.push(['release']),
  };
  const flow = await prepareChromeCookieImport(f.options);
  assert.deepEqual(lifecycle,[['reserve',{permissions:['cookies'],origins:['*://example.test/*']}]]);
  const reading = flow.confirmAndRead();
  assert.equal(f.count('permission'),1);
  await reading;
  assert.deepEqual(lifecycle.map(([kind]) => kind),['reserve','activate']);
  assert.equal(flow.permissionsReleased,false);
  await flow.releasePermissions();
  await flow.releasePermissions();
  assert.deepEqual(lifecycle.map(([kind]) => kind),['reserve','activate','release']);
  assert.equal(flow.permissionsReleased,true);
});

test('permission lifecycle releases on denial, cancellation and idle timeout', async () => {
  for (const mode of ['denial','cancellation','timeout']) {
    const f = fixture();
    const lifecycle = [];
    f.options.permissionLifecycle = {reserve:async () => lifecycle.push('reserve'),
      activate:async () => lifecycle.push('activate'),release:async () => lifecycle.push('release')};
    if (mode === 'denial') f.options.chrome.permissions.request = async () => false;
    if (mode === 'timeout') f.options.timeoutMs = 20;
    const flow = await prepareChromeCookieImport(f.options);
    if (mode === 'denial') await assert.rejects(flow.confirmAndRead(),{code:'cookie_source_denied'});
    if (mode === 'cancellation') await flow.cancel();
    if (mode === 'timeout') {
      await new Promise(resolve => setTimeout(resolve,40));
      assert.equal(flow.state,'cancelled');
    }
    assert.deepEqual(lifecycle.filter(value => value === 'release'),['release']);
  }
});

test('caller mutations cannot change displayed selection, granted hosts or source store', async () => {
  const f = fixture();
  const flow = await prepareChromeCookieImport(f.options);
  f.options.selection.domains[0] = 'unapproved.test';
  f.options.selection.source_store_id = 'other';
  f.options.selection.overwrite = true;
  await flow.confirmAndRead();
  assert.deepEqual(flow.selection.domains,['example.test']);
  assert.deepEqual(f.calls.find(([k]) => k === 'permission')[1].origins,['*://example.test/*']);
  for (const [kind,value] of f.calls.filter(([kind]) => kind.endsWith('BrowserImport') || kind.endsWith('BrowserImportSource'))) {
    const selected = value[kind].selection;
    assert.equal(selected.source_store_id,'normal');
    assert.equal(selected.overwrite,false);
  }
  await flow.cancel();
});

test('concurrent confirmation cannot prompt, approve or read twice', async () => {
  const f = fixture();
  const permission = deferred();
  f.options.chrome.permissions.request = () => {f.calls.push(['permission']); return permission.promise;};
  const flow = await prepareChromeCookieImport(f.options);
  const first = flow.confirmAndRead();
  await assert.rejects(flow.confirmAndRead(),{code:'cookie_source_denied'});
  permission.resolve(true);
  await first;
  assert.equal(f.count('permission'),1);
  assert.equal(f.count('ApproveBrowserImport'),1);
  assert.equal(f.count('cookies'),1);
  await flow.cancel();
});

test('denied, rejected or throwing permission requests cancel consent without approving or reading', async () => {
  for (const mode of ['deny','reject','throw']) {
    const f = fixture();
    f.options.chrome.permissions.request = () => {
      if (mode === 'throw') throw new Error('private-marker');
      return mode === 'reject' ? Promise.reject(new Error('private-marker')) : Promise.resolve(false);
    };
    const flow = await prepareChromeCookieImport(f.options);
    await assert.rejects(flow.confirmAndRead(),error => {
      assert.equal(String(error).includes('private-marker'),false);
      return ['cookie_source_denied','cookie_source_unavailable'].includes(error.code);
    });
    assert.equal(flow.state,'cancelled');
    assert.equal(f.count('ApproveBrowserImport'),0);
    assert.equal(f.count('cookies'),0);
    assert.equal(f.count('CancelBrowserImport'),1);
  }
});

test('cancel during permission prompt ignores a late grant and does not revoke shared permissions', async () => {
  const f = fixture();
  const permission = deferred();
  f.options.chrome.permissions.request = () => permission.promise;
  f.options.chrome.permissions.remove = () => {throw new Error('must not remove preexisting permissions');};
  const flow = await prepareChromeCookieImport(f.options);
  const reading = flow.confirmAndRead();
  const rejected = assert.rejects(reading,{code:'cookie_source_cancelled'});
  const first = flow.cancel();
  assert.equal(first,flow.cancel());
  assert.deepEqual(await first,{kernelCancellationConfirmed:true});
  permission.resolve(true);
  await rejected;
  await tick();
  assert.equal(f.count('ApproveBrowserImport'),0);
  assert.equal(f.count('cookies'),0);
});

test('caller abort stops a stalled approval using its owned signal and one separate cancel request', async () => {
  const f = fixture();
  const caller = new AbortController();
  const began = deferred();
  const approval = deferred();
  let approvalSignal;
  const send = f.options.request;
  f.options.signal = caller.signal;
  f.options.request = (payload,options) => {
    if (!payload.ApproveBrowserImport) return send(payload,options);
    approvalSignal = options.signal;
    began.resolve();
    return approval.promise;
  };
  const flow = await prepareChromeCookieImport(f.options);
  const reading = flow.confirmAndRead();
  const rejected = assert.rejects(reading,{code:'cookie_source_cancelled'});
  await began.promise;
  caller.abort('private-marker');
  await rejected;
  approval.resolve(reply('approved'));
  await tick();
  assert.equal(approvalSignal.aborted,true);
  assert.equal(f.count('cookies'),0);
  assert.equal(f.count('CancelBrowserImport'),1);
  const cancellationSignal = f.calls.find(([k]) => k === 'CancelBrowserImport')[2].signal;
  assert.notEqual(cancellationSignal,approvalSignal);
  assert.equal(cancellationSignal.aborted,true);
});

test('unacknowledged cancellation is reported without retry or transport payload leakage', async () => {
  const f = fixture();
  const send = f.options.request;
  f.options.request = (payload,options) => payload.CancelBrowserImport
    ? Promise.reject(new Error('private-marker')) : send(payload,options);
  const flow = await prepareChromeCookieImport(f.options);
  assert.deepEqual(await flow.cancel(),{kernelCancellationConfirmed:false});
  await assert.rejects(flow.confirmAndRead(),{code:'cookie_source_denied'});
});

test('invalid prepare and approve replies cannot authorize access', async () => {
  for (const stage of ['PrepareBrowserImport','ApproveBrowserImport']) {
    for (const response of [true,{},reply('source_authorized'),
      {BrowserImportConsent:{request_id:'bad',status:'prepared'}},
      {...reply('prepared'),cookie:'private-marker'}]) {
      const f = fixture();
      const send = f.options.request;
      f.options.request = (payload,options) => payload[stage] ? Promise.resolve(response) : send(payload,options);
      if (stage === 'PrepareBrowserImport') await assert.rejects(prepareChromeCookieImport(f.options),{code:'cookie_source_denied'});
      else {
        const flow = await prepareChromeCookieImport(f.options);
        await assert.rejects(flow.confirmAndRead(),{code:'cookie_source_denied'});
        assert.equal(f.count('CancelBrowserImport'),1);
      }
      assert.equal(f.count('cookies'),0);
    }
  }
});

test('kernel revocation during cookie query cancels the flow and never returns the batch', async () => {
  const f = fixture();
  let revoked = false;
  const read = f.options.chrome.cookies.getAll;
  const send = f.options.request;
  f.options.chrome.cookies.getAll = value => {revoked = true; return read(value);};
  f.options.request = (payload,options) => payload.AuthorizeBrowserImportSource && revoked
    ? Promise.resolve(reply('cancelled')) : send(payload,options);
  const flow = await prepareChromeCookieImport(f.options);
  await assert.rejects(flow.confirmAndRead(),{code:'cookie_source_denied'});
  assert.equal(flow.state,'cancelled');
  assert.equal(f.count('CancelBrowserImport'),1);
});

test('idle preparation expires without another user action', {timeout:1000}, async () => {
  const f = fixture();
  f.options.timeoutMs = 25;
  const cancelled = deferred();
  const send = f.options.request;
  f.options.request = (payload,options) => {
    if (payload.CancelBrowserImport) cancelled.resolve();
    return send(payload,options);
  };
  const flow = await prepareChromeCookieImport(f.options);
  await cancelled.promise;
  assert.equal(flow.state,'cancelled');
  await assert.rejects(flow.confirmAndRead(),{code:'cookie_source_denied'});
  assert.equal(f.count('permission'),0);
  assert.equal(f.count('cookies'),0);
});

test('stalled preparation has a total deadline and discards late replies', {timeout:1000}, async () => {
  const f = fixture();
  const late = deferred();
  f.options.timeoutMs = 20;
  let signal;
  f.options.request = (_,options) => {signal = options.signal; return late.promise;};
  await assert.rejects(prepareChromeCookieImport(f.options),{code:'cookie_source_timeout'});
  assert.equal(signal.aborted,true);
  late.resolve(reply('prepared'));
  await tick();
  assert.equal(f.count('permission'),0);
  assert.equal(f.count('cookies'),0);
});

test('invalid metadata and pre-cancelled calls perform no browser or kernel actions', async () => {
  for (const change of [o => {o.sourceTabId = -1;},o => {o.timeoutMs = 120001;},
    o => {o.selection.domains = ['*.test'];},o => {o.selection.domains = [];},
    o => {o.signal = {};},o => {o.request = undefined;}]) {
    const f = fixture();
    change(f.options);
    await assert.rejects(prepareChromeCookieImport(f.options),{code:'cookie_source_denied'});
    assert.equal(f.calls.length,0);
  }
  const f = fixture();
  f.options.signal = AbortSignal.abort('private-marker');
  await assert.rejects(prepareChromeCookieImport(f.options),{code:'cookie_source_cancelled'});
  assert.equal(f.calls.length,0);
});
