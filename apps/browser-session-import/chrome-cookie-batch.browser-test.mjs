import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import test from 'node:test';
import { prepareChromeCookieBatch } from './chrome-cookie-batch.mjs';

// Explicit opt-in; never downloads a browser or opens a real user's profile.
assert.ok(process.env.PLAYWRIGHT_MODULE, 'PLAYWRIGHT_MODULE must name the installed test dependency');
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE);

test('converted cookies authenticate a separate browser context without changing the source or broadening host scope', {timeout: 30000}, async () => {
  const browser = await chromium.launch({channel: 'chrome', headless: true,
    chromiumSandbox: true, args: ['--enable-automation']});
  const secret = randomUUID();
  try {
    const browserCdp = await browser.newBrowserCDPSession();
    const commandLine = await browserCdp.send('Browser.getBrowserCommandLine');
    assert.equal(commandLine.arguments.includes('--no-sandbox'), false);
    assert.equal(commandLine.arguments.some(arg => arg.startsWith('--unsafely-treat-insecure-origin-as-secure')), false);
    const source = await browser.newContext();
    const destination = await browser.newContext();
    for (const context of [source, destination]) {
      await context.route('**/*', async route => {
        const url = new URL(route.request().url());
        if (!url.hostname.endsWith('.example.test')) return route.abort();
        if (url.pathname === '/login' && context === source) {
          return route.fulfill({status: 200, headers: {
            'set-cookie': `session=${secret}; Secure; HttpOnly; Path=/; SameSite=Lax`,
          }, body: 'SIGNED_IN'});
        }
        const headers = await route.request().allHeaders();
        return route.fulfill({status: 200,
          body: headers.cookie?.split('; ').includes(`session=${secret}`) ? 'SIGNED_IN' : 'SIGN_IN_REQUIRED'});
      });
    }
    const sourcePage = await source.newPage();
    const targetPage = await destination.newPage();
    await sourcePage.goto('https://login.example.test/login');
    await targetPage.goto('https://login.example.test/inbox');
    assert.equal(await targetPage.textContent('body'), 'SIGN_IN_REQUIRED');
    await destination.addCookies([{name:'unrelated', value:'keep', url:'https://control.test/'}]);

    const before = await source.cookies();
    assert.equal(before.length, 1);
    const cookie = before[0];
    const chromeCookie = {
      name: cookie.name, value: cookie.value, domain: cookie.domain,
      path: cookie.path, secure: cookie.secure, httpOnly: cookie.httpOnly,
      hostOnly: !cookie.domain.startsWith('.'), session: cookie.expires === -1,
      sameSite: {Lax:'lax', Strict:'strict', None:'no_restriction'}[cookie.sameSite],
      storeId: 'source-store',
    };
    const prepared = prepareChromeCookieBatch([chromeCookie], {
      approvedDomains: ['login.example.test'], sourceStoreId: 'source-store',
    });
    const cdp = await destination.newCDPSession(targetPage);
    await cdp.send('Network.setCookies', {cookies: prepared.cookies});
    await targetPage.reload();
    assert.equal(await targetPage.textContent('body'), 'SIGNED_IN');
    assert.equal(await targetPage.evaluate(() => document.cookie), '', 'HttpOnly remains hidden from page scripts');
    const imported = (await destination.cookies()).find(c => c.name === 'session');
    assert.deepEqual({domain:imported.domain, expires:imported.expires, secure:imported.secure,
      httpOnly:imported.httpOnly, sameSite:imported.sameSite}, {
      domain:'login.example.test', expires:-1, secure:true, httpOnly:true, sameSite:'Lax',
    });
    await targetPage.goto('https://sub.login.example.test/inbox');
    assert.equal(await targetPage.textContent('body'), 'SIGN_IN_REQUIRED', 'host-only access must not reach a subdomain');
    assert.equal((await destination.cookies()).find(c => c.name === 'unrelated')?.value, 'keep');
    assert.deepEqual(await source.cookies(), before);

    const partitionKey = {topLevelSite:'https://top.test', hasCrossSiteAncestor:true};
    const partitioned = prepareChromeCookieBatch([{...chromeCookie,
      name:'partitioned', domain:'embedded.example.test', partitionKey,
    }], {approvedDomains:['embedded.example.test'], sourceStoreId:'source-store',
      approvedPartitionSites:['https://top.test']});
    await cdp.send('Network.setCookies', {cookies:partitioned.cookies});
    const {targetInfo} = await cdp.send('Target.getTargetInfo');
    assert.ok(targetInfo.browserContextId);
    const stored = await browserCdp.send('Storage.getCookies', {browserContextId:targetInfo.browserContextId});
    assert.deepEqual(stored.cookies.find(c => c.name === 'partitioned')?.partitionKey, partitionKey);
    await cdp.detach();
    await browserCdp.detach();
  } finally {
    await browser.close();
  }
});
