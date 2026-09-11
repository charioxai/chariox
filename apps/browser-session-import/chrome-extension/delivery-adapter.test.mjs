import assert from 'node:assert/strict';
import test from 'node:test';
import {deliverBrowserImport} from './delivery-adapter.mjs';

const requestId = 'a'.repeat(32);
const selection = Object.freeze({session_id:'room',attachment_id:'attachment',environment_id:'environment',
  runtime_generation:1,tab_id:'tab',document_revision:1,source_store_id:'normal',
  domains:Object.freeze(['example.test','login.example.test']),partition_sites:Object.freeze([]),overwrite:false});
const response = results => ({BrowserImportDelivered:{results}});
const cookies = value => [{name:'session',value,url:'https://example.test/',path:'/',httpOnly:true,secure:true}];

test('adapter uses the paired encrypted deliver method and returns exact public domain results', async () => {
  const source = cookies('private-marker');
  const signal = new AbortController().signal;
  const calls = [];
  const result = await deliverBrowserImport({deliver:async (value,options) => {
    calls.push({value,options});
    assert.equal(value.cookies[0].value,'private-marker');
    return response([{domain:'example.test',status:'imported',cookie_count:1},
      {domain:'login.example.test',status:'no_cookies',cookie_count:0}]);
  },requestId,selection,cookies:source},{signal,timeoutMs:30000});
  assert.deepEqual(result,{requestId,status:'completed',domains:[
    {domain:'example.test',status:'imported'},
    {domain:'login.example.test',status:'sign_in_required'},
  ]});
  assert.equal(calls.length,1);
  assert.equal(calls[0].value.requestId,requestId);
  assert.equal(calls[0].value.selection,selection);
  assert.deepEqual(calls[0].options,{signal,timeoutMs:30000});
  assert.equal(source[0].value,'');
  assert.equal(JSON.stringify(result).includes('private-marker'),false);
});

test('adapter rejects aggregate and malformed delivery results and always scrubs cookie values', async () => {
  const invalid = [
    {BrowserImportDelivered:{cookie_count:1}},
    response([{domain:'example.test',status:'imported',cookie_count:1}]),
    response([{domain:'login.example.test',status:'imported',cookie_count:1},
      {domain:'example.test',status:'no_cookies',cookie_count:0}]),
    response([{domain:'example.test',status:'unsupported',cookie_count:1},
      {domain:'login.example.test',status:'no_cookies',cookie_count:0}]),
    response([{domain:'example.test',status:'imported',cookie_count:0},
      {domain:'login.example.test',status:'no_cookies',cookie_count:0}]),
    response([{domain:'example.test',status:'imported',cookie_count:1,detail:'private-marker'},
      {domain:'login.example.test',status:'no_cookies',cookie_count:0}]),
  ];
  for (const value of invalid) {
    const source = cookies('private-marker');
    await assert.rejects(deliverBrowserImport({deliver:async () => value,
      requestId,selection,cookies:source}),error => {
      assert.equal(error.code,'browser_import_delivery_unavailable');
      assert.equal(String(error).includes('private-marker'),false);
      return true;
    });
    assert.equal(source[0].value,'');
  }
});

test('adapter redacts paired transport failures and scrubs the source batch', async () => {
  const source = cookies('private-marker');
  await assert.rejects(deliverBrowserImport({deliver:async () => {
    throw new Error('transport private-marker');
  },requestId,selection,cookies:source}),error => {
    assert.equal(error.code,'browser_import_delivery_unavailable');
    assert.equal(error.message,'browser_import_delivery_unavailable');
    return true;
  });
  assert.equal(source[0].value,'');
});
