import assert from 'node:assert/strict';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {BrowserCdpClient} from '../kernel/slice-linux-docker/docker/browser-controller-cdp.mjs';
import {applyControllerCookieImport} from './controller-cookie-import.mjs';

assert.ok(process.env.PLAYWRIGHT_MODULE,'PLAYWRIGHT_MODULE must name the installed dependency');
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE);
const source = [{name:'session',value:'fixture-controller',domain:'example.test',path:'/',
  secure:true,httpOnly:true,hostOnly:true,session:true,sameSite:'lax',storeId:'source'}];
const scope = {approvedDomains:['example.test'],sourceStoreId:'source'};
const viewport = {css_width:1280,css_height:800,device_scale_factor:1,desktop_pixel_width:1280,desktop_pixel_height:800};

test('controller imports into its registered target and rejects stale browser and document identities', {timeout:30000}, async () => {
  const profile = await mkdtemp(path.join(tmpdir(),'chariox-controller-import-'));
  let context, controller;
  try {
    context = await chromium.launchPersistentContext(profile,{channel:'chrome',headless:true,
      chromiumSandbox:true,args:['--remote-debugging-port=0']});
    const port = Number((await readFile(path.join(profile,'DevToolsActivePort'),'utf8')).split('\n')[0]);
    controller = new BrowserCdpClient({debuggerEndpoint:`http://127.0.0.1:${port}`});
    await context.route('**/*',async route => {
      const headers = await route.request().allHeaders();
      await route.fulfill({body:headers.cookie?.includes('session=fixture-controller') ? 'SIGNED_IN' : 'SIGNED_OUT'});
    });
    const page = context.pages()[0];
    await page.goto('https://example.test/');
    const state = await controller.reconcile(viewport);
    const tab = state.tabs.find(tab => tab.url === 'https://example.test/');
    const selection = {controller,browserGeneration:state.browser_generation,
      targetId:tab.target_id,documentId:tab.document_id,source,scope};
    const authority = {authorize:async () => true,runExclusive:async operation => operation()};
    await assert.rejects(applyControllerCookieImport(selection),{code:'cookie_import_denied'});
    await assert.rejects(applyControllerCookieImport(selection,{...authority,authorize:async binding => {
      assert.equal(Object.isFrozen(binding),true);
      assert.deepEqual(binding.approvedDomains,['example.test']);
      assert.equal('source' in binding,false);
      return false;
    }}),{code:'cookie_import_denied'});
    await assert.rejects(applyControllerCookieImport({...selection,browserGeneration:99},authority),
      {code:'cookie_import_target_stale'});
    assert.deepEqual(await context.cookies(),[]);
    assert.deepEqual(await applyControllerCookieImport(selection,authority),{cookieCount:1,domains:['example.test']});
    await page.reload();
    assert.equal(await page.textContent('body'),'SIGNED_IN');
    await assert.rejects(applyControllerCookieImport({...selection,overwrite:true},authority),
      {code:'cookie_import_target_stale'});
  } finally {
    try {await controller?.close();} finally {
      try {await context?.close();} finally {await rm(profile,{recursive:true,force:true});}
    }
  }
});
