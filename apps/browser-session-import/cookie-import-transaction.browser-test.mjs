import assert from 'node:assert/strict';
import test from 'node:test';
import {applyCookieImport, createCdpCookieStore} from './cookie-import-transaction.mjs';

assert.ok(process.env.PLAYWRIGHT_MODULE, 'PLAYWRIGHT_MODULE must name the installed test dependency');
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE);
const source = [{name:'session',value:'fixture-import',domain:'example.test',path:'/',
  secure:true,httpOnly:true,hostOnly:true,session:true,sameSite:'lax',storeId:'source'}];
const scope = {approvedDomains:['example.test'],sourceStoreId:'source'};

test('real CDP import verifies sign-in and restores the previous session after cancellation', {timeout:30000}, async () => {
  const browser = await chromium.launch({channel:'chrome',headless:true,chromiumSandbox:true});
  try {
    const context = await browser.newContext();
    await context.route('**/*', async route => {
      const headers = await route.request().allHeaders();
      return route.fulfill({status:200,body:headers.cookie?.includes('session=fixture-import') ? 'SIGNED_IN' : 'SIGNED_OUT'});
    });
    const page = await context.newPage();
    await page.goto('https://example.test/');
    assert.equal(await page.textContent('body'),'SIGNED_OUT');
    const browserCdp = await browser.newBrowserCDPSession();
    const pageCdp = await context.newCDPSession(page);
    const store = await createCdpCookieStore({browserCdp,pageCdp});
    const options = {source,scope,store,authorize:async () => true,runExclusive:async fn => fn()};
    await applyCookieImport(options);
    await page.reload();
    assert.equal(await page.textContent('body'),'SIGNED_IN');

    await store.write([{name:'session',value:'partition-control',url:'https://example.test/',path:'/',
      secure:true,httpOnly:true,sameSite:'None',
      partitionKey:{topLevelSite:'https://unselected.test',hasCrossSiteAncestor:true}}]);
    const before = await store.read();
    const controller = new AbortController();
    const write = store.write;
    const interrupted = {...store,write:async cookies => {await write(cookies); controller.abort();}};
    await assert.rejects(applyCookieImport({...options,source:[{...source[0],value:'replaced'}],
      store:interrupted,overwrite:true,signal:controller.signal}), {
      code:'cookie_import_cancelled',recoveryRequired:false,
    });
    const order = cookies => cookies.sort((a,b) => Number(!!a.partitionKey) - Number(!!b.partitionKey));
    assert.deepEqual(order(await store.read()),order(before));
    await page.reload();
    assert.equal(await page.textContent('body'),'SIGNED_IN');
    await pageCdp.detach();
    await browserCdp.detach();
  } finally { await browser.close(); }
});
