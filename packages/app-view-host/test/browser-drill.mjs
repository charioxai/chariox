// MP-08 / MP-09 / MP-10 / MP-11 — AV1 candidate, no product cutover.
// A nonzero exit is intentional when any required browser guarantee fails.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join, resolve, isAbsolute } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { packView } from '../pack.mjs';
import { digest } from '../bundle.mjs';
import { browserServer } from './browser-server.mjs';
import { networkCases } from './network-cases.mjs';
import { isolationCases, sourceBindingCases } from './isolation-cases.mjs';
import { referenceCases } from './reference-cases.mjs';
import { browserProfile } from './browser-profile.mjs';

const evidence = process.env.CHARIOX_APP_VIEW_EVIDENCE;
const builds = process.env.CHARIOX_APP_VIEW_BUILDS;
const webDist = process.env.CHARIOX_APP_VIEW_WEB_DIST;
assert(evidence && builds && webDist && [evidence, builds, webDist].every(isAbsolute), 'set absolute lane-owned CHARIOX_APP_VIEW_EVIDENCE, CHARIOX_APP_VIEW_BUILDS and built CHARIOX_APP_VIEW_WEB_DIST');
const require = createRequire(join(resolve(webDist, '..'), 'package.json'));
const { build } = require('esbuild');
const { chromium } = createRequire(join(resolve(webDist, '../../../'), 'package.json'))('playwright-core');
await mkdir(evidence, { recursive: true });
const results = {
  scope: 'MP-08 / MP-09 / MP-10 / MP-11 AV1 renderer and browser containment proof; no Web/TUI session acceptance',
  attacks: [], references: [], limitations: [], started: new Date().toISOString(),
};
const key = await crypto.subtle.generateKey('Ed25519', true, ['sign', 'verify']);
const manifest = { schema: 'chariox.app.v2', appContractVersion: 2, publisher: { id: 'com.example', keyId: 'test' }, ui: { entry: 'ui/index.html' } };
const pack = script => packView(manifest, { schema: 'chariox.app-view-source.v1', rootId: 'root', script, style: '' }, key);
const launchOptions = { executablePath: process.env.CHARIOX_TEST_CHROME ?? '/opt/google/chrome/chrome', headless: true,
  ignoreDefaultArgs: ['--disable-background-networking'], args: ['--no-sandbox', '--disable-dev-shm-usage'] };
let browser, server, currentProfile;
const consoleMessages = [];
async function save() { await writeFile(join(evidence, 'results.json'), JSON.stringify(results, null, 2)); }
try {
  server = await browserServer({ evidence, webDist, build });
  results.builtAssets = {};
  for (const name of ['host.html', 'client.mjs']) results.builtAssets[name] = await digest(await readFile(join(webDist, 'assets/app-view-host', name)));
  await server.capture();
  results.profile = 'fresh ordinary Chromium profiles; background networking enabled; no incognito preconnect suppression';
  await mkdir(join(evidence, 'netlog'), { recursive: true });
  for (const [name, boundary, attack] of networkCases(server.origin)) {
    const item = { name, removedBoundary: boundary, runs: [] };
    for (const control of [true, false]) {
      const endpoint = control ? server.controlReceiver : server.strictReceiver;
      const phase = name + (control ? '-control' : '-strict');
      const netlog = join(evidence, 'netlog', phase + '.json');
      currentProfile = await browserProfile(chromium, { ...launchOptions, viewport: { width: 1440, height: 900 },
        args: [...launchOptions.args, `--log-net-log=${netlog}`] });
      const { context } = currentProfile;
      results.browser ??= context.browser().version();
      const page = await context.newPage();
      const initialPages = context.pages().length;
      page.on('console', message => consoleMessages.push({ name, control, type: message.type(), text: message.text().slice(0, 600) }));
      server.setBoundary(control ? boundary : null); server.setPhase(phase);
      const beforeHttp = server.hits.length, beforeUdp = server.packets.length, beforeTcp = server.connections.length;
      const code = `globalThis.probe={ran:true};top.postMessage({type:'av1.probe-start',name:${JSON.stringify(name)}},'*');try{${attack(endpoint.url + '/' + phase, endpoint.udpPort, phase)}}catch(e){probe.error=e.name+': '+e.message}top.postMessage({type:'av1.probe-result',probe},'*');`;
      const row = { control, startedAt: Date.now() };
      try {
        await server.open(page, await pack(code));
        await delay(['webrtc', 'nested-blank-realm', 'dns-prefetch', 'preconnect'].includes(name) ? 1300 : 450);
        row.started = await page.evaluate(name => window.probeStarts?.includes(name) === true, name).catch(() => false);
        // Top navigation deliberately removes the test shell in the red run.
        row.probe = await page.evaluate(() => window.probeResults?.at(-1)).catch(() => null);
        row.appUrls = page.frames().map(frame => frame.url());
        row.extraPages = context.pages().length - initialPages;
        row.topUrl = page.url();
      } catch (error) { row.error = error.stack; }
      row.received = server.hits.slice(beforeHttp).filter(hit => hit.control === control);
      row.connections = server.connections.slice(beforeTcp).filter(hit => hit.control === control);
      row.udpPackets = server.packets.slice(beforeUdp).filter(hit => hit.control === control);
      row.finishedAt = Date.now();
      await currentProfile.close(); currentProfile = null;
      const trace = JSON.parse(await readFile(netlog, 'utf8'));
      const queryType = trace.constants.logEventTypes.DNS_TRANSACTION_QUERY;
      row.dnsQueries = trace.events.filter(event => event.type === queryType && event.params?.qname === `appview-dns-${phase}.invalid`);
      item.runs.push(row);
    }
    const [red, green] = item.runs;
    item.blocked = green.received.length === 0 && green.udpPackets.length === 0 && green.connections.length === 0 && green.dnsQueries.length === 0
      && green.extraPages === 0 && green.topUrl === server.origin + '/probe.html' && green.started === true;
    item.payloadBlocked = green.received.length === 0 && green.udpPackets.length === 0 && green.dnsQueries.length === 0 && green.started === true;
    item.redCapable = red.received.length > 0 || red.udpPackets.length > 0 || red.connections.length > 0 || red.dnsQueries.length > 0 || red.probe?.historyChanged === true;
    if (!item.redCapable) results.limitations.push(name + ': positive receiving control was not demonstrated');
    results.attacks.push(item); await save();
    console.log(name, JSON.stringify({ blocked: item.blocked, payloadBlocked: item.payloadBlocked, redCapable: item.redCapable, strictConnections: green.connections.length, error: green.error?.slice(0, 140) }));
  }
  server.setBoundary(null);
  browser = await chromium.launch(launchOptions);
  results.isolation = await isolationCases({ browser, server, pack }); await save();
  results.sourceBinding = await sourceBindingCases({ browser, server, pack }); await save();

  // A valid signed package is required even for an entry that would otherwise
  // execute. This goes through the actual Cloud-built host, not verifyView alone.
  const context = await browser.newContext(), page = await context.newPage();
  const corrupt = await pack('top.postMessage({type:"unverified-code-ran"},"*");');
  corrupt.bundle.signature.value = btoa(String.fromCharCode(...new Uint8Array(64)));
  try { await server.open(page, corrupt); results.rejectedBeforeExecution = false; }
  catch (error) {
    const host = page.frames().find(frame => frame.url().endsWith('/assets/app-view-host/host.html'));
    results.rejectedBeforeExecution = /signature failed/.test(error.message) && host === undefined;
    results.signatureRejection = error.message;
  }
  await context.close(); await browser.close(); browser = null;
  results.references = await referenceCases({ chromium, launchOptions, server, evidence, builds });
  results.strictReceived = server.hits.filter(hit => !hit.control);
  results.receiver = { hits: server.hits, connections: server.connections, udpPackets: server.packets };
  results.rendererPassed = results.references.length === 4 && results.references.every(row => row.rendered && !row.failure);
  results.containmentPassed = results.attacks.every(item => item.blocked && item.redCapable)
    && results.isolation.passed && results.sourceBinding.passed && results.rejectedBeforeExecution;
  results.failedChecks = results.attacks.filter(item => !item.blocked || !item.redCapable).map(item => item.name);
  if (!results.isolation.passed) results.failedChecks.push('storage/native-input/compilation');
  if (!results.sourceBinding.passed) results.failedChecks.push('host-source-binding');
  if (!results.rejectedBeforeExecution) results.failedChecks.push('signature-before-execution');
  results.limitations.push('Real authenticated descriptor, Room conversation, backend broker, live WSS and TUI hosts require AV2–AV5; frontend-only fixture is not their acceptance.');
  results.finished = new Date().toISOString();
  if (!results.rendererPassed || !results.containmentPassed) process.exitCode = 1;
} catch (error) { results.failure = error.stack; console.error(error); process.exitCode = 1; }
finally {
  await currentProfile?.close(); await browser?.close(); await server?.close();
  results.cleanup = 'All browsers closed; own HTTP/TCP/UDP servers closed; own tcpdump stopped by guarded exact PID.';
  await save(); await writeFile(join(evidence, 'console.json'), JSON.stringify(consoleMessages, null, 2));
}
console.log(JSON.stringify({ rendererPassed: results.rendererPassed, containmentPassed: results.containmentPassed, failedChecks: results.failedChecks, failure: results.failure, limitations: results.limitations }, null, 2));
