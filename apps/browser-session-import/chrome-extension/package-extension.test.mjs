import assert from 'node:assert/strict';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {extensionIdForPublicKey,packageChromeExtension} from './package-extension.mjs';

test('packager emits a loadable extension tree and transpiles shared browser modules', async t => {
  assert.ok(process.env.TYPESCRIPT_MODULE,'TYPESCRIPT_MODULE must name an existing TypeScript module');
  const typescript = (await import(process.env.TYPESCRIPT_MODULE)).default;
  const root = await mkdtemp(path.join(tmpdir(),'chariox-chrome-connector-'));
  t.after(() => rm(root,{recursive:true,force:true}));
  await packageChromeExtension(root,typescript);
  const manifest = JSON.parse(await readFile(path.join(root,'manifest.json'),'utf8'));
  assert.equal(manifest.background.service_worker,
    'apps/browser-session-import/chrome-extension/background.mjs');
  const consent = await readFile(path.join(root,'apps/browser-session-import/chrome-import-consent-flow.mjs'),'utf8');
  assert.equal(consent.includes('browser-import-requests.ts'),false);
  const requests = await readFile(path.join(root,'packages/kernel-client/src/browser-import-requests.js'),'utf8');
  assert.equal(requests.includes('export type'),false);
  const connector = await readFile(path.join(root,'apps/browser-session-import/chrome-extension/connector.mjs'),'utf8');
  assert.match(connector,/browser-relay-crypto\.js/);
  assert.match(connector,/flow\.confirmAndDeliver/);
  assert.match(connector,/relay\.deliver/);
  assert.doesNotMatch(connector,/flow\.confirmAndRead/);
  const delivery = await readFile(path.join(root,
    'apps/browser-session-import/chrome-extension/delivery-adapter.mjs'),'utf8');
  assert.match(delivery,/deliverBrowserImport/);
  assert.doesNotMatch(delivery,/expectedDeliveryEnvelope|cookie_count:1/);
  const permissions = await readFile(path.join(root,
    'apps/browser-session-import/chrome-extension/permission-coordinator.mjs'),'utf8');
  assert.match(permissions,/PermissionGrantCoordinator/);
});

test('packager binds the reviewed public key to one stable extension ID',async t => {
  const typescript=(await import(process.env.TYPESCRIPT_MODULE)).default;
  const root=await mkdtemp(path.join(tmpdir(),'chariox-signed-connector-'));
  t.after(()=>rm(root,{recursive:true,force:true}));
  const publicKey=Buffer.from(Uint8Array.from({length:96},(_,index)=>index+1)).toString('base64');
  const expected=extensionIdForPublicKey(publicKey);
  const result=await packageChromeExtension(root,typescript,{extensionPublicKey:publicKey});
  const manifest=JSON.parse(await readFile(path.join(root,'manifest.json'),'utf8'));
  assert.equal(manifest.key,publicKey);
  assert.equal(result.extensionId,expected);
  assert.equal((await readFile(path.join(root,'browser-import-extension-id.txt'),'utf8')).trim(),expected);
  assert.match(expected,/^[a-p]{32}$/);
});
