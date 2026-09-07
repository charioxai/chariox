import assert from 'node:assert/strict';
import {copyFile, mkdir, mkdtemp, rm, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import test from 'node:test';

assert.ok(process.env.PLAYWRIGHT_MODULE, 'PLAYWRIGHT_MODULE must name the installed test dependency');
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE);

test('reads HttpOnly cookies through a real MV3 extension without reading other domains', {timeout:30000}, async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'chariox-cookie-reader-'));
  const extension = path.join(root, 'extension');
  let context;
  try {
    await mkdir(extension);
    for (const file of ['chrome-cookie-reader.mjs','chrome-cookie-batch.mjs']) {
      await copyFile(new URL(file, import.meta.url), path.join(extension, file));
    }
    // Test-only extension; exact fixture hosts, no real profile or network endpoint.
    await writeFile(path.join(extension, 'manifest.json'), JSON.stringify({
      manifest_version:3, name:'Chariox cookie reader fixture', version:'0.0.1',
      permissions:['cookies'], host_permissions:['*://example.test/*'],
      background:{service_worker:'worker.mjs', type:'module'},
    }));
    await writeFile(path.join(extension, 'worker.mjs'),
      "import {readApprovedChromeCookies} from './chrome-cookie-reader.mjs'; globalThis.readFixture = readApprovedChromeCookies;");
    context = await chromium.launchPersistentContext(path.join(root, 'profile'), {
      channel:'chromium', headless:true, chromiumSandbox:true,
      ...(process.env.CHARIOX_TEST_CHROMIUM ? {executablePath:process.env.CHARIOX_TEST_CHROMIUM} : {}),
      args:[`--disable-extensions-except=${extension}`, `--load-extension=${extension}`],
    });
    await context.route('**/*', route => route.fulfill({status:200, body:'fixture'}));
    const page = await context.newPage();
    await page.goto('https://example.test/');
    await context.addCookies([
      {name:'fixture', value:'not-a-real-credential', url:'https://example.test/', httpOnly:true, secure:true},
      {name:'other', value:'not-for-import', url:'https://other.test/'},
    ]);
    const cdp = await context.newCDPSession(page);
    await cdp.send('Network.setCookies', {cookies:[{
      name:'partitioned', value:'partition-fixture', url:'https://example.test/',
      path:'/', secure:true, httpOnly:true, sameSite:'None',
      partitionKey:{topLevelSite:'https://top.test', hasCrossSiteAncestor:true},
    }]});
    await cdp.detach();
    const before = await context.cookies();
    const worker = context.serviceWorkers()[0] ?? await context.waitForEvent('serviceworker', {timeout:5000});
    const result = await worker.evaluate(async () => {
      const tabs = await chrome.tabs.query({});
      const tab = tabs.find(t => t.url === 'https://example.test/');
      if (!tab) throw new Error('fixture_tab_missing');
      const stores = await chrome.cookies.getAllCookieStores();
      const store = stores.find(s => s.tabIds.includes(tab.id));
      if (!store) throw new Error('fixture_store_missing');
      const prepared = await globalThis.readFixture({chrome, sourceTabId:tab.id,
        scope:{approvedDomains:['example.test'], sourceStoreId:store.id,
          approvedPartitionSites:['https://top.test']}, authorize:async () => true});
      // Secret values never leave the extension through this test result.
      return {summary:prepared.summary, records:prepared.cookies.map(c => ({name:c.name,
        httpOnly:c.httpOnly, partitionKey:c.partitionKey ?? null})).sort((a,b) => a.name.localeCompare(b.name))};
    });
    assert.deepEqual(result, {summary:{cookieCount:2,domains:['example.test']},
      records:[{name:'fixture',httpOnly:true,partitionKey:null},
        {name:'partitioned',httpOnly:true,partitionKey:{topLevelSite:'https://top.test',hasCrossSiteAncestor:true}}]});
    assert.deepEqual(await context.cookies(), before);
  } finally {
    try { await context?.close(); }
    finally { await rm(root, {recursive:true, force:true}); }
  }
});
