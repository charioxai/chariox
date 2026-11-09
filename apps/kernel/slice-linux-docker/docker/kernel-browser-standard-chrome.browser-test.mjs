// MP-08/MP-10/MP-11: the kernel browser must look like the Chrome a person installs. Sites such as Google
// sign-in refuse a browser that exposes automation ("This browser or app may not be secure").
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

const root = process.env.CHARIOX_MDACCESS_DRILL_ROOT;
assert(root, 'explicit disposable drill root required');

const probe = `(async () => {
  const native = fn => /\\{\\s*\\[native code\\]\\s*\\}/.test(Function.prototype.toString.call(fn));
  const worker = await new Promise(resolve => {
    const w = new Worker(URL.createObjectURL(new Blob(['postMessage("alive")'])));
    w.onmessage = event => resolve(event.data); setTimeout(() => resolve('hung'), 4000);
  });
  const named = Object.getOwnPropertyNames(window).filter(name => /chariox|^__(?!proto)/i.test(name));
  const own = [[navigator.permissions, 'query'], [navigator.geolocation, 'getCurrentPosition'], [navigator.clipboard, 'readText'], [navigator.mediaDevices, 'getUserMedia']].map(([o, k]) => Object.hasOwn(o, k));
  const fns = [navigator.permissions.query, navigator.geolocation.getCurrentPosition, navigator.geolocation.watchPosition, Notification.requestPermission,
    navigator.clipboard.readText, navigator.mediaDevices.getUserMedia, Object.getOwnPropertyDescriptor(Notification, 'permission').get, Function.prototype.toString];
  const status = await navigator.permissions.query({name: 'geolocation'});
  return JSON.stringify({
    webdriver: navigator.webdriver, worker, named, own,
    notNative: fns.map((fn, i) => native(fn) ? null : i).filter(i => i !== null),
    constructable: fns.slice(0, 7).map(fn => 'prototype' in fn),
    names: [navigator.permissions.query.name, Notification.requestPermission.name, Object.getOwnPropertyDescriptor(Notification, 'permission').get.name],
    lengths: [navigator.geolocation.getCurrentPosition.length, navigator.geolocation.watchPosition.length, navigator.permissions.query.length],
    stateOwn: Object.hasOwn(status, 'state'), state: status.state, notification: Notification.permission,
    plugins: navigator.plugins.length, chrome: typeof window.chrome, brands: navigator.userAgentData.brands.map(b => b.brand),
  });
})()`;

test('MP-08/10/11 the kernel browser exposes no automation and no replaced browser APIs', {timeout: 90000}, async () => {
  const home = await mkdtemp(path.join(root, 'standard-chrome-'));
  const host = new KernelBrowserHost(home, {appStorageRoot: path.join(home, 'apps')});
  const server = createServer((_q, r) => r.end('<!doctype html><title>Standard</title><p>Standard Chrome</p>'));
  try {
    await new Promise(r => server.listen(0, '127.0.0.1', r));
    const opened = await host.request({op: 'open', url: `http://127.0.0.1:${server.address().port}/`});
    const tab = host.tabs.get(opened.tabs[0].tab_id);
    const {connection, sessionId} = await host.browser.resolvePageTarget(tab.target_id);
    await new Promise(r => setTimeout(r, 1500));
    const response = await connection.send('Runtime.evaluate', {expression: probe, returnByValue: true, awaitPromise: true, userGesture: true}, sessionId);
    assert.ok(!response.exceptionDetails, 'probe threw');
    const seen = JSON.parse(response.result.value);
    // A running page worker must not break the browser state reads the agent and viewers depend on.
    await new Promise(r => setTimeout(r, 1000));
    assert.ok((await host.request({op: 'state'})).tabs.length > 0);
    assert.equal(seen.webdriver, false, 'navigator.webdriver must be false like a Chrome a person started');
    assert.equal(seen.worker, 'alive', 'dedicated workers must start');
    assert.deepEqual(seen.named, [], 'no automation or Chariox globals');
    assert.deepEqual(seen.own, [false, false, false, false], 'permission APIs live on their prototypes');
    assert.deepEqual(seen.notNative, [], 'every wrapped API reports native source');
    assert.deepEqual(seen.constructable, Array(7).fill(false), 'wrappers have no prototype');
    assert.deepEqual(seen.names, ['query', 'requestPermission', 'get permission']);
    assert.deepEqual(seen.lengths, [1, 1, 1], 'Chromium declares one parameter on each');
    assert.equal(seen.stateOwn, false);
    assert.equal(seen.state, 'prompt');
    assert.equal(seen.notification, 'default');
    assert.ok(seen.plugins > 0 && seen.chrome === 'object' && seen.brands.includes('Chromium'));
  } finally { await host.stop(); await new Promise(r => server.close(r)); await rm(home, {recursive: true, force: true}); }
});

// The Mac kernel always launches Chrome headless. Headless Chrome announces itself ("HeadlessChrome" in the User-Agent)
// and reports no pointer or hover; Google sign-in answered it with "This browser or app may not be secure" (2026-11-09).
test('MP-08/10/11 a headless launch (the Mac kernel) reports the identity of the Chrome a person runs', {timeout: 90000}, async () => {
  const { HostChromium } = await import('./kernel-browser-process.mjs');
  const home = await mkdtemp(path.join(root, 'standard-chrome-headless-'));
  const appStorageRoot = path.join(home, 'apps');
  const host = new KernelBrowserHost(home, {appStorageRoot, chromium: new HostChromium(home, {appStorageRoot, environment: {...process.env, CHARIOX_KERNEL_BROWSER_HEADLESS: '1'}})});
  const seenHeaders = [];
  const server = createServer((request, response) => { seenHeaders.push(request.headers['user-agent']); response.end('<!doctype html><title>Headless</title><p>Headless</p>'); });
  try {
    await new Promise(r => server.listen(0, '127.0.0.1', r));
    const opened = await host.request({op: 'open', url: `http://127.0.0.1:${server.address().port}/`});
    const tab = host.tabs.get(opened.tabs[0].tab_id);
    const {connection, sessionId} = await host.browser.resolvePageTarget(tab.target_id);
    await new Promise(r => setTimeout(r, 1500));
    const evaluate = async expression => { const r = await connection.send('Runtime.evaluate', {expression, returnByValue: true, awaitPromise: true}, sessionId); assert.ok(!r.exceptionDetails, 'probe threw'); return r.result.value; };
    const seen = JSON.parse(await evaluate(`(async () => {
      const worker = await new Promise(resolve => { const w = new Worker(URL.createObjectURL(new Blob(['postMessage(navigator.userAgent)']))); w.onmessage = e => resolve(e.data); setTimeout(() => resolve('hung'), 4000); });
      return JSON.stringify({ua: navigator.userAgent, worker, appVersion: navigator.appVersion, brands: navigator.userAgentData.brands.map(b => b.brand),
        pointer: [matchMedia('(pointer: fine)').matches, matchMedia('(hover: hover)').matches, matchMedia('(any-pointer: fine)').matches]});
    })()`));
    const major = (await connection.send('Browser.getVersion')).product.match(/\/(\d+)\./)[1];
    const expected = `Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${major}.0.0.0 Safari/537.36`;
    assert.equal(seen.ua, expected, 'navigator.userAgent must not say HeadlessChrome');
    assert.equal(seen.worker, expected, 'workers report the same User-Agent as the page');
    assert.equal(seenHeaders.at(-1), expected, 'the User-Agent request header must match navigator.userAgent');
    assert.ok(!/Headless/.test(seen.appVersion) && !seen.brands.some(b => /Headless/.test(b)));
    assert.deepEqual(seen.pointer, [true, true, true], 'a desktop Chrome has a fine pointer and hover');
  } finally { await host.stop(); await new Promise(r => server.close(r)); await rm(home, {recursive: true, force: true}); }
});
