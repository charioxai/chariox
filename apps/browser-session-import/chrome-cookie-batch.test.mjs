import assert from 'node:assert/strict';
import test from 'node:test';
import { prepareChromeCookieBatch } from './chrome-cookie-batch.mjs';

const scope = { approvedDomains: ['example.test'], sourceStoreId: 'source-store', nowSeconds: 1000 };
const cookie = (change = {}) => ({
  name: 'session', value: 'fixture-only', domain: 'example.test', path: '/',
  hostOnly: true, httpOnly: true, secure: true, session: true,
  sameSite: 'lax', storeId: 'source-store', ...change,
});

test('preserves domain scope, persistent expiry and unspecified SameSite', () => {
  const result = prepareChromeCookieBatch([cookie({ domain: '.example.test', hostOnly: false,
    session: false, expirationDate: 2000, path: '/account', sameSite: 'unspecified' })], scope);
  assert.deepEqual(result.cookies, [{ name: 'session', value: 'fixture-only',
    url: 'https://example.test/', domain: '.example.test', path: '/account',
    httpOnly: true, secure: true, expires: 2000 }]);
});

test('keeps partition ancestry and requires explicit partition-site approval', () => {
  const partitionKey = { topLevelSite: 'https://top.test', hasCrossSiteAncestor: true };
  const source = [cookie({ partitionKey })];
  const approvedScope = { ...scope, approvedPartitionSites: ['https://top.test'] };
  assert.deepEqual(prepareChromeCookieBatch(source, approvedScope).cookies[0].partitionKey, partitionKey);
  assert.throws(() => prepareChromeCookieBatch(source, scope));
  assert.throws(() => prepareChromeCookieBatch([cookie({partitionKey: {topLevelSite:'https://top.test'}})], approvedScope));
  assert.throws(() => prepareChromeCookieBatch([cookie({partitionKey: {...partitionKey, topLevelSite:'https://top.test/secret'}})], approvedScope));
  assert.throws(() => prepareChromeCookieBatch([cookie({partitionKey, secure:false})], approvedScope));
});

test('rejects unknown cookie semantics, normalized duplicates and aggregate payload overflow', () => {
  assert.throws(() => prepareChromeCookieBatch([cookie({partitionKeyOpaque: true})], scope),
    error => error.code === 'unsupported_cookie_fields');
  assert.throws(() => prepareChromeCookieBatch([
    cookie({domain:'.example.test', hostOnly:false}), cookie({domain:'example.test', hostOnly:false}),
  ], scope), error => error.code === 'duplicate_cookie_import');
  assert.throws(() => prepareChromeCookieBatch(Array.from({length: 140}, (_, n) => cookie({
    name:`n${n}`, value:'x'.repeat(4000),
  })), scope), error => error.code === 'cookie_import_too_large');
});

test('rejects malformed, expired, ambiguous and oversized batches', () => {
  for (const change of [
    { name: 'bad;name' }, { value: 'bad\r\nvalue' }, { path: 'relative' },
    { domain: '.example.test', hostOnly: true }, { secure: 'true' },
    { session: false }, { session: false, expirationDate: 999 },
    { expirationDate: 2000 }, { sameSite: 'unknown' },
    { name: '__Host-session', path: '/account' },
    { name: '__Secure-session', secure: false },
    { sameSite: 'no_restriction', secure: false },
    { name: 'session', value: 'x'.repeat(4097) },
  ]) assert.throws(() => prepareChromeCookieBatch([cookie(change)], scope));
  assert.throws(() => prepareChromeCookieBatch([cookie(), cookie()], scope));
  assert.throws(() => prepareChromeCookieBatch(Array.from({ length: 513 }, (_, n) => cookie({name:`n${n}`})), scope));
  assert.throws(() => prepareChromeCookieBatch([null], scope));
  assert.throws(() => prepareChromeCookieBatch([cookie()], { ...scope, sourceStoreId: '' }));
});

test('rejects the entire batch for unapproved domains and mismatched source stores without echoing values', () => {
  for (const change of [
    { domain: 'evil-example.test' }, { domain: 'sub.example.test' },
    { domain: 'example.test.attacker.test' }, { domain: 'example.test@attacker.test' },
    { storeId: 'other-store' },
  ]) {
    assert.throws(() => prepareChromeCookieBatch([cookie(), cookie({ ...change, value: 'private-marker' })], scope), error => {
      assert.equal(String(error).includes('private-marker'), false);
      return ['cookie_scope_denied', 'cookie_store_mismatch', 'invalid_cookie_import'].includes(error.code);
    });
  }
});

test('preserves host-only HttpOnly session cookies without inventing an expiry or destination store', () => {
  const source = [cookie()];
  const before = structuredClone(source);
  const result = prepareChromeCookieBatch(source, scope);
  assert.deepEqual(result.cookies, [{
    name: 'session', value: 'fixture-only', url: 'https://example.test/',
    path: '/', httpOnly: true, secure: true, sameSite: 'Lax',
  }]);
  assert.deepEqual(result.summary, { cookieCount: 1, domains: ['example.test'],results:[
    {domain:'example.test',status:'imported',cookie_count:1},
  ] });
  assert.deepEqual(source, before);
});
