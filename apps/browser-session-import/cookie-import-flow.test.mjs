import assert from 'node:assert/strict';
import test from 'node:test';
import {readApprovedChromeCookies} from './chrome-cookie-reader.mjs';
import {applyCookieImport} from './cookie-import-transaction.mjs';

test('source collection feeds destination validation without changing cookie formats twice', async () => {
  const source = [{name:'session',value:'fixture-only',domain:'example.test',path:'/',
    secure:true,httpOnly:true,hostOnly:true,session:true,sameSite:'lax',storeId:'normal'}];
  const scope = {approvedDomains:['example.test'],sourceStoreId:'normal'};
  const chrome = {tabs:{get:async () => ({id:7,incognito:false})},
    permissions:{contains:async () => true},cookies:{
      getAllCookieStores:async () => [{id:'normal',tabIds:[7]}],
      getAll:async () => source,
    }};
  const collected = await readApprovedChromeCookies({chrome,scope,sourceTabId:7,authorize:async () => true});
  let stored = [];
  const store = {read:async () => stored, write:async cookies => {
    stored = cookies.map(({url,...cookie}) => ({...cookie,domain:cookie.domain ?? new URL(url).hostname,
      session:cookie.expires === undefined,expires:cookie.expires ?? -1}));
  },remove:async () => {stored=[];}};
  const result = await applyCookieImport({source:collected.cookies,scope,store,
    authorize:async () => true,runExclusive:async fn => fn()});
  assert.deepEqual(result,{cookieCount:1,domains:['example.test']});
  assert.equal(stored[0].httpOnly,true);
  assert.equal(source[0].storeId,'normal');
});
