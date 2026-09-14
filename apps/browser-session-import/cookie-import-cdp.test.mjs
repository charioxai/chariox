import assert from 'node:assert/strict';
import test from 'node:test';
import {setTimeout as delay} from 'node:timers/promises';
import {applyCookieImport, createCdpCookieStore} from './cookie-import-transaction.mjs';

test('destination discovery has a bounded deadline when Chrome never replies', {timeout:500}, async () => {
  const stalled = {send:() => new Promise(() => {})};
  await assert.rejects(createCdpCookieStore({browserCdp:stalled,pageCdp:stalled,timeoutMs:20}), {
    code:'cookie_import_cdp_timeout',recoveryRequired:false,
  });
});

test('destination discovery uses one deadline across page and browser identity checks', {timeout:500}, async () => {
  const slow = {send:async () => {
    await delay(30);
    return {targetInfo:{targetId:'page-1',browserContextId:'context-1'}};
  }};
  await assert.rejects(createCdpCookieStore({browserCdp:slow,pageCdp:slow,timeoutMs:50}), {
    code:'cookie_import_cdp_timeout',recoveryRequired:false,
  });
});

const targetInfo = {targetId:'page-1',browserContextId:'context-1'};
const source = [{name:'session',value:'fixture-only',domain:'example.test',path:'/',
  secure:true,httpOnly:true,hostOnly:true,session:true,sameSite:'lax',storeId:'normal'}];
const scope = {approvedDomains:['example.test'],sourceStoreId:'normal'};

test('a timed-out write requires recovery and a late acknowledgement cannot reopen the store', {timeout:500}, async () => {
  let acknowledge;
  const commands = [];
  const cdp = {send:async method => {
    commands.push(method);
    if (method === 'Target.getTargetInfo') return {targetInfo};
    if (method === 'Target.getBrowserContexts') return {browserContextIds:['context-1']};
    if (method === 'Storage.getCookies') return {cookies:[]};
    if (method === 'Storage.setCookies') return new Promise(resolve => {acknowledge=resolve;});
    throw new Error('unexpected fixture command');
  }};
  const store = await createCdpCookieStore({browserCdp:cdp,pageCdp:cdp,timeoutMs:20});
  await assert.rejects(applyCookieImport({source,scope,store,
    authorize:async () => true,runExclusive:async fn => fn()}), {
    code:'cookie_import_cdp_timeout',recoveryRequired:true,
  });
  acknowledge({});
  await assert.rejects(store.read(),{code:'cookie_import_cdp_unavailable',recoveryRequired:true});
  await assert.rejects(store.remove([]),{code:'cookie_import_cdp_unavailable',recoveryRequired:true});
  assert.equal(commands.at(-1),'Storage.setCookies');
});

test('deleting a batch shares one deadline rather than multiplying it by cookie count', {timeout:500}, async () => {
  const cdp = {send:async method => {
    if (method === 'Target.getTargetInfo') return {targetInfo};
    if (method === 'Target.getBrowserContexts') return {browserContextIds:['context-1']};
    await delay(30);
    return {};
  }};
  const store = await createCdpCookieStore({browserCdp:cdp,pageCdp:cdp,timeoutMs:50});
  await assert.rejects(store.remove([
    {name:'one',domain:'example.test',path:'/'},
    {name:'two',domain:'example.test',path:'/'},
  ]),{code:'cookie_import_cdp_timeout',recoveryRequired:true});
});

test('invalid timeout settings cannot disable the destination deadline', async () => {
  const cdp = {send:() => {throw new Error('must not reach Chrome');}};
  for (const timeoutMs of [0,-1,Infinity,NaN,5001,1.5,'20']) {
    await assert.rejects(createCdpCookieStore({browserCdp:cdp,pageCdp:cdp,timeoutMs}), {
      code:'cookie_import_invalid_timeout',recoveryRequired:false,
    });
  }
});

test('transport errors are redacted and prevent further commands on the failed store', async () => {
  const cdp = {send:async method => {
    if (method === 'Target.getTargetInfo') return {targetInfo};
    if (method === 'Target.getBrowserContexts') return {browserContextIds:['context-1']};
    throw new Error('fixture-cookie-value-must-not-escape');
  }};
  const store = await createCdpCookieStore({browserCdp:cdp,pageCdp:cdp});
  await assert.rejects(store.read(),error => {
    assert.equal(error.code,'cookie_import_cdp_failed');
    assert.equal(error.recoveryRequired,false);
    assert.equal(String(error).includes('fixture-cookie-value'),false);
    return true;
  });
  await assert.rejects(store.write([]),{code:'cookie_import_cdp_unavailable',recoveryRequired:false});
});

test('unknown context identifiers never fall back to the default cookie store', async () => {
  const cdp = {send:async method => {
    if (method === 'Target.getTargetInfo') return {targetInfo};
    if (method === 'Target.getBrowserContexts') return {browserContextIds:[],defaultBrowserContextId:'different-default'};
    throw new Error('cookie storage must not be accessed');
  }};
  await assert.rejects(createCdpCookieStore({browserCdp:cdp,pageCdp:cdp}), {
    code:'cookie_import_context_unsupported',recoveryRequired:false,
  });
});
