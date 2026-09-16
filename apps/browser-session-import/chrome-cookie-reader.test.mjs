import assert from 'node:assert/strict';
import test from 'node:test';
import { readApprovedChromeCookies } from './chrome-cookie-reader.mjs';

const scope = {approvedDomains:['example.test'], sourceStoreId:'normal'};
const cookie = (changes = {}) => ({name:'session', value:'fixture-only',
  domain:'example.test', path:'/', secure:true, httpOnly:true, hostOnly:true,
  session:true, sameSite:'lax', storeId:'normal', ...changes});
function fixture() {
  const reads = [];
  const chrome = {
    tabs:{get:async () => ({id:7, incognito:false})},
    permissions:{contains:async () => true},
    cookies:{getAllCookieStores:async () => [{id:'normal', tabIds:[7]}],
      getAll:async details => { reads.push(details); return [cookie(), cookie({domain:'sub.example.test'})]; }},
  };
  return {chrome, reads, options:{chrome, scope, sourceTabId:7, authorize:async () => true}};
}

test('reads the selected normal store and returns only exact approved domains', async () => {
  const {options, reads} = fixture();
  const result = await readApprovedChromeCookies(options);
  assert.equal(result.cookies.length, 1);
  assert.deepEqual(result.summary, {cookieCount:1, domains:['example.test']});
  assert.deepEqual(reads, [{domain:'example.test', storeId:'normal'}]);
  assert.equal(result.cookies[0].httpOnly, true);
});

test('rejects missing consent, host permissions, incognito and mismatched stores before cookie reads', async () => {
  for (const mode of ['missing-consent','denied','permission','incognito','wrong-store','missing-tab']) {
    const {options, chrome, reads} = fixture();
    if (mode === 'missing-consent') delete options.authorize;
    if (mode === 'denied') options.authorize = async () => false;
    if (mode === 'permission') chrome.permissions.contains = async () => false;
    if (mode === 'incognito') chrome.tabs.get = async () => ({id:7, incognito:true});
    if (mode === 'wrong-store') options.scope = {...scope, sourceStoreId:'private'};
    if (mode === 'missing-tab') chrome.tabs.get = async () => ({id:8, incognito:false});
    await assert.rejects(readApprovedChromeCookies(options), {code:'cookie_source_denied'}, mode);
    assert.equal(reads.length, 0);
  }
});

test('does not publish cookies after revocation, permission removal or cancellation during a read', async () => {
  for (const mode of ['revoked','permission','cancelled']) {
    const {options, chrome} = fixture();
    const controller = new AbortController();
    options.signal = controller.signal;
    let allowed = true;
    options.authorize = async () => allowed;
    chrome.cookies.getAll = async () => {
      if (mode === 'revoked') allowed = false;
      if (mode === 'permission') chrome.permissions.contains = async () => false;
      if (mode === 'cancelled') controller.abort('private-marker');
      return [cookie()];
    };
    await assert.rejects(readApprovedChromeCookies(options), error => {
      assert.equal(String(error).includes('private-marker'), false);
      return ['cookie_source_denied','cookie_source_cancelled'].includes(error.code);
    });
  }
});

test('reads only explicitly approved partition sites and preserves both ancestor variants', async () => {
  const {options, chrome, reads} = fixture();
  options.scope = {...scope, approvedPartitionSites:['https://top.test']};
  chrome.cookies.getAll = async details => {
    reads.push(details);
    return details.partitionKey ? [false, true].map(hasCrossSiteAncestor => cookie({
      partitionKey:{topLevelSite:'https://top.test', hasCrossSiteAncestor},
    })) : [cookie()];
  };
  const result = await readApprovedChromeCookies(options);
  assert.equal(result.cookies.length, 3);
  assert.deepEqual(reads, [{domain:'example.test',storeId:'normal'},
    {domain:'example.test',storeId:'normal',partitionKey:{topLevelSite:'https://top.test'}}]);
  assert.deepEqual(result.cookies.slice(1).map(c => c.partitionKey.hasCrossSiteAncestor), [false,true]);
});

test('sanitizes API failures and aborts before starting any query when already cancelled', async () => {
  const {options, chrome, reads} = fixture();
  chrome.cookies.getAll = async () => {throw new Error('private-marker');};
  await assert.rejects(readApprovedChromeCookies(options), error =>
    error.code === 'cookie_source_unavailable' && !String(error).includes('private-marker'));
  const controller = new AbortController();
  controller.abort('private-marker');
  options.signal = controller.signal;
  chrome.tabs.get = async () => {reads.push('tab'); throw new Error('must not read');};
  await assert.rejects(readApprovedChromeCookies(options), {code:'cookie_source_cancelled'});
  assert.equal(reads.length, 0);
});

test('settles cancellation even when Chrome has not settled its cookie query', {timeout:1000}, async () => {
  const {options, chrome} = fixture();
  const controller = new AbortController();
  options.signal = controller.signal;
  let began;
  const ready = new Promise(resolve => {began = resolve;});
  chrome.cookies.getAll = () => {began(); return new Promise(() => {});};
  const pending = readApprovedChromeCookies(options);
  await ready;
  controller.abort();
  await assert.rejects(pending, {code:'cookie_source_cancelled'});
});

test('expires a stalled browser call without requiring user action', {timeout:1000}, async () => {
  const {options, chrome} = fixture();
  options.timeoutMs = 20;
  chrome.cookies.getAll = () => new Promise(() => {});
  await assert.rejects(readApprovedChromeCookies(options), {code:'cookie_source_timeout'});
});

test('snapshots the approved scope before awaiting Chrome or consent', async () => {
  const {options, reads} = fixture();
  options.scope = {...scope, approvedDomains:['example.test']};
  options.authorize = async () => {options.scope.approvedDomains[0] = 'unapproved.test'; return true;};
  const result = await readApprovedChromeCookies(options);
  assert.deepEqual(reads, [{domain:'example.test', storeId:'normal'}]);
  assert.equal(result.cookies.length, 1);
});

test('rejects oversized browser replies before issuing another site query', async () => {
  const {options, chrome, reads} = fixture();
  options.scope = {...scope, approvedDomains:['example.test','second.test']};
  chrome.cookies.getAll = async details => {
    reads.push(details);
    return Array.from({length:513}, (_, n) => cookie({name:`n${n}`}));
  };
  await assert.rejects(readApprovedChromeCookies(options), {code:'cookie_source_too_large'});
  assert.equal(reads.length, 1);
});

test('binds each authorization check to an immutable source selection', async () => {
  const {options} = fixture();
  options.authorize = async selection => {
    assert.equal(selection.sourceTabId, 7);
    assert.deepEqual(selection.scope.approvedDomains, ['example.test']);
    assert.equal(selection.scope.sourceStoreId, 'normal');
    assert.equal(Object.isFrozen(selection), true);
    assert.equal(Object.isFrozen(selection.scope), true);
    assert.equal(Object.isFrozen(selection.scope.approvedDomains), true);
    return true;
  };
  assert.equal((await readApprovedChromeCookies(options)).cookies.length, 1);
});
