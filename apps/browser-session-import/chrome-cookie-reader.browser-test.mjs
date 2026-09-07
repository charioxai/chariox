import assert from 'node:assert/strict';
import {copyFile, mkdir, mkdtemp, readFile, rm, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {createRelayKeypair, decryptRelayPayload} from '../../packages/kernel-client/src/relay-crypto.ts';
import {applyCookieImport, createCdpCookieStore} from './cookie-import-transaction.mjs';

assert.ok(process.env.PLAYWRIGHT_MODULE, 'PLAYWRIGHT_MODULE must name the installed test dependency');
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE);
assert.ok(process.env.TYPESCRIPT_MODULE, 'TYPESCRIPT_MODULE must name the installed test compiler');
const ts = (await import(process.env.TYPESCRIPT_MODULE)).default;

test('real MV3 cookies cross an encrypted relay envelope and authenticate an isolated destination', {timeout:30000}, async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'chariox-cookie-reader-'));
  const extension = path.join(root, 'extension');
  let context;
  try {
    await mkdir(extension);
    for (const file of ['chrome-cookie-reader.mjs','chrome-cookie-batch.mjs']) {
      await copyFile(new URL(file, import.meta.url), path.join(extension, file));
    }
    const cryptoSource = await readFile(new URL('../../packages/kernel-client/src/browser-relay-crypto.ts',import.meta.url),'utf8');
    await writeFile(path.join(extension,'browser-relay-crypto.mjs'),ts.transpileModule(cryptoSource,{
      compilerOptions:{module:ts.ModuleKind.ES2022,target:ts.ScriptTarget.ES2022},
    }).outputText);
    // Test-only extension; exact fixture hosts, no real profile or network endpoint.
    await writeFile(path.join(extension, 'manifest.json'), JSON.stringify({
      manifest_version:3, name:'Chariox cookie reader fixture', version:'0.0.1',
      permissions:['cookies'], host_permissions:['*://example.test/*'],
      background:{service_worker:'worker.mjs', type:'module'},
    }));
    await writeFile(path.join(extension, 'worker.mjs'),
      "import {readApprovedChromeCookies} from './chrome-cookie-reader.mjs'; import {encryptRelayPayload} from './browser-relay-crypto.mjs'; globalThis.readFixture = readApprovedChromeCookies; globalThis.encryptFixture = encryptRelayPayload;");
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
    const selection = await worker.evaluate(async () => {
      const tabs = await chrome.tabs.query({});
      const tab = tabs.find(t => t.url === 'https://example.test/');
      if (!tab) throw new Error('fixture_tab_missing');
      const stores = await chrome.cookies.getAllCookieStores();
      const store = stores.find(s => s.tabIds.includes(tab.id));
      if (!store) throw new Error('fixture_store_missing');
      return {sourceTabId:tab.id,scope:{approvedDomains:['example.test'],sourceStoreId:store.id,
        approvedPartitionSites:['https://top.test']}};
    });
    const recipient = createRelayKeypair();
    const encrypted = await worker.evaluate(async ({selection,publicKey}) => {
      const collected = await globalThis.readFixture({chrome,...selection,authorize:async () => true});
      return (await globalThis.encryptFixture(publicKey,JSON.stringify(collected))).payload;
    },{selection,publicKey:recipient.publicKeyBase64});
    // The simulated relay receives ciphertext only. Destination policy comes from
    // the independently selected fixture scope, not the decrypted cookie payload.
    assert.equal(JSON.stringify(encrypted).includes('not-a-real-credential'),false);
    const collected = JSON.parse(decryptRelayPayload(recipient.privateKey,encrypted));
    const result = {summary:collected.summary,records:collected.cookies.map(c => ({name:c.name,
      httpOnly:c.httpOnly,partitionKey:c.partitionKey ?? null})).sort((a,b) => a.name.localeCompare(b.name))};
    assert.deepEqual(result, {summary:{cookieCount:2,domains:['example.test']},
      records:[{name:'fixture',httpOnly:true,partitionKey:null},
        {name:'partitioned',httpOnly:true,partitionKey:{topLevelSite:'https://top.test',hasCrossSiteAncestor:true}}]});
    assert.deepEqual(await context.cookies(), before);

    const destination = await context.browser().newContext();
    try {
      await destination.route('**/*',async route => {
        const headers = await route.request().allHeaders();
        return route.fulfill({status:200,body:headers.cookie?.includes('fixture=not-a-real-credential') ? 'SIGNED_IN' : 'SIGNED_OUT'});
      });
      const targetPage = await destination.newPage();
      await targetPage.goto('https://example.test/inbox');
      assert.equal(await targetPage.textContent('body'),'SIGNED_OUT');
      const browserCdp = await context.browser().newBrowserCDPSession();
      const pageCdp = await destination.newCDPSession(targetPage);
      const store = await createCdpCookieStore({browserCdp,pageCdp});
      await applyCookieImport({source:collected.cookies,scope:selection.scope,store,
        authorize:async () => true,runExclusive:async fn => fn()});
      await targetPage.reload();
      assert.equal(await targetPage.textContent('body'),'SIGNED_IN');
      assert.equal(await targetPage.evaluate(() => document.cookie),'');
      await pageCdp.detach();
      await browserCdp.detach();
      assert.deepEqual(await context.cookies(),before);
    } finally { await destination.close(); }
  } finally {
    try { await context?.close(); }
    finally { await rm(root, {recursive:true, force:true}); }
  }
});
