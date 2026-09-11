import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';

const root = new URL('./',import.meta.url);
const manifest = JSON.parse(await readFile(new URL('manifest.json',root),'utf8'));

test('MV3 manifest is installable and keeps cookie and host access optional', () => {
  assert.equal(manifest.manifest_version,3);
  assert.deepEqual(manifest.permissions,['activeTab','storage','alarms']);
  assert.deepEqual(manifest.optional_permissions,['cookies']);
  assert.deepEqual(manifest.optional_host_permissions,['http://*/*','https://*/*']);
  assert.equal(manifest.host_permissions,undefined);
  assert.equal(manifest.content_scripts,undefined);
  assert.deepEqual(manifest.externally_connectable,{matches:[
    'https://chariox.com/*','https://staging.chariox.com/*',
  ]});
  assert.equal(manifest.web_accessible_resources,undefined);
});

test('connector persists only permission lease metadata and exposes only the encrypted external broker', async () => {
  const files = ['background.mjs','connector.mjs','connector-core.mjs','delivery-adapter.mjs',
    'permission-coordinator.mjs','external-port-broker.mjs','web-bridge-protocol.mjs'];
  const source = (await Promise.all(files.map(file => readFile(new URL(file,root),'utf8')))).join('\n');
  for (const forbidden of ['chrome.storage.local','chrome.storage.sync','localStorage','sessionStorage','window.postMessage',
    'onMessageExternal','console.','history.','analytics','captureVisibleTab','executeScript']) {
    assert.equal(source.includes(forbidden),false,forbidden);
  }
  assert.match(source,/chrome\.storage\.session/);
  assert.match(source,/port\.sender\?\.documentId/);
  assert.match(source,/runtime\.getContexts/);
  assert.match(source,/permissions\.request/);
  assert.match(source,/deliverBrowserImport/);
  assert.match(source,/browser_import_delivery_unavailable/);
  const connector = await readFile(new URL('connector.mjs',root),'utf8');
  const click = connector.slice(connector.indexOf("element('start').addEventListener"),
    connector.indexOf('async function finishImport'));
  assert.ok(click.indexOf('flow.confirmAndDeliver') < click.indexOf('finishImport(delivery)'));
  assert.equal(click.includes('await '),false);
  assert.match(connector,/finally \{\s*await flow\?\.releasePermissions\(\)/);
  assert.match(connector,/pagehide.*flow\?\.cancel\(\)/);
  assert.match(connector,/kernelCancellationConfirmed === true/);
  assert.match(connector,/browser_import_cancellation_unconfirmed/);
  const html = await readFile(new URL('connector.html',root),'utf8');
  assert.doesNotMatch(html,/<textarea|bootstrap-response|relay_auth_token|pairing response/i);
  assert.match(connector,/acceptEncryptedBootstrap/);
  assert.match(connector,/acceptEncryptedPairing/);
  assert.match(connector,/webPort\.onDisconnect\.addListener/);
  assert.match(connector,/webBridgeDisconnected/);
});
