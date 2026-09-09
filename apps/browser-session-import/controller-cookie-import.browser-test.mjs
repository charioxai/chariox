import assert from 'node:assert/strict';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {randomBytes} from 'node:crypto';
import {setTimeout as delay} from 'node:timers/promises';
import {openCookieImportJournal} from './cookie-import-journal.mjs';
import {BrowserCdpClient} from '../kernel/slice-linux-docker/docker/browser-controller-cdp.mjs';
import {applyControllerCookieImport,recoverControllerCookieImport} from './controller-cookie-import.mjs';
import {completeCookieImport} from './cookie-import-completion.mjs';

assert.ok(process.env.PLAYWRIGHT_MODULE,'PLAYWRIGHT_MODULE must name the installed dependency');
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE);
const source = [{name:'session',value:'fixture-controller',domain:'example.test',path:'/',
  secure:true,httpOnly:true,hostOnly:true,session:true,sameSite:'lax',storeId:'source'}];
const scope = {approvedDomains:['example.test'],sourceStoreId:'source'};
const viewport = {css_width:1280,css_height:800,device_scale_factor:1,desktop_pixel_width:1280,desktop_pixel_height:800};

test('controller imports into its registered target and rejects stale browser and document identities', {timeout:30000}, async () => {
  const profile = await mkdtemp(path.join(tmpdir(),'chariox-controller-import-'));
  const journalDirectory = await mkdtemp(path.join(tmpdir(),'chariox-controller-journal-'));
  const key = randomBytes(32);
  let context, controller, journal;
  try {
    journal = await openCookieImportJournal({directory:journalDirectory,key,
      binding:{userId:'fixture',roomId:'fixture',environmentId:'fixture'}});
    context = await chromium.launchPersistentContext(profile,{channel:'chrome',headless:true,
      chromiumSandbox:true,args:['--remote-debugging-port=0']});
    const port = Number((await readFile(path.join(profile,'DevToolsActivePort'),'utf8')).split('\n')[0]);
    controller = new BrowserCdpClient({debuggerEndpoint:`http://127.0.0.1:${port}`});
    let writerSequence = 0;
    let workerWriterSequence = 0;
    let workerRequestStarted;
    let workerResponseGate;
    await context.route('**/*',async route => {
      const requestPath = new URL(route.request().url()).pathname;
      if (requestPath === '/cookie-worker.js') {
        return route.fulfill({status:200,contentType:'application/javascript',body:`
          postMessage('ready');
          onmessage = () => fetch('/worker-writer')
            .then(() => postMessage('done'), () => postMessage('failed'));
        `});
      }
      if (requestPath === '/worker-writer') {
        workerWriterSequence += 1;
        workerRequestStarted?.resolve();
        await workerResponseGate?.promise;
        try {
          return await route.fulfill({status:200,headers:{
            'set-cookie':`session=worker-${workerWriterSequence}; Path=/; Secure; HttpOnly; SameSite=Lax`,
          },body:'worker'});
        } catch {
          return undefined;
        }
      }
      if (requestPath === '/writer') {
        writerSequence += 1;
        return route.fulfill({status:200,headers:{
          'set-cookie':`session=writer-${writerSequence}; Path=/; Secure; HttpOnly; SameSite=Lax`,
        },body:'writer'});
      }
      const headers = await route.request().allHeaders();
      await route.fulfill({body:headers.cookie?.includes('session=fixture-controller') ? 'SIGNED_IN' : 'SIGNED_OUT'});
    });
    const page = context.pages()[0];
    await page.goto('https://example.test/');
    const state = await controller.reconcile(viewport);
    const tab = state.tabs.find(tab => tab.url === 'https://example.test/');
    const selection = {controller,browserGeneration:state.browser_generation,
      targetId:tab.target_id,documentId:tab.document_id,source,scope};
    const completions = [];
    const authority = {journal,authorize:async () => true,runExclusive:async operation => operation(),
      complete:async ({receipt,result,binding}) => {
        assert.equal(result.cookieCount,1);
        assert.deepEqual(binding.approvedDomains,['example.test']);
        await completeCookieImport({receipt,journal,
          recordOutcome:async()=>{completions.push('outcome');},
          clearPending:async()=>{completions.push('clear');}});
      }};
    await assert.rejects(applyControllerCookieImport(selection,{...authority,journal:undefined}),
      {code:'cookie_import_journal_required'});
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
    assert.deepEqual(completions,['outcome','clear']);
    assert.equal(await journal.read(),null);
    await page.reload();
    assert.equal(await page.textContent('body'),'SIGNED_IN');
    await assert.rejects(applyControllerCookieImport({...selection,overwrite:true},authority),
      {code:'cookie_import_target_stale'});
    let refreshed = await controller.reconcile(viewport);
    let current = refreshed.tabs.find(tab => tab.url === 'https://example.test/');
    await page.evaluate(() => {
      globalThis.__charioxWriter = setInterval(() => fetch('/writer').catch(() => {}),5);
    });
    await waitFor(async () => (await context.cookies()).some(cookie => cookie.value.startsWith('writer-')));
    completions.length = 0;
    const guardedAuthority = {...authority,complete:async payload => {
      await delay(100);
      assert.equal((await context.cookies()).find(cookie => cookie.name === 'session').value,
        'fixture-controller','page/network writers must stay paused through durable cleanup');
      await authority.complete(payload);
    }};
    assert.deepEqual(await applyControllerCookieImport({...selection,overwrite:true,
      browserGeneration:refreshed.browser_generation,documentId:current.document_id},guardedAuthority),
    {cookieCount:1,domains:['example.test']});
    assert.deepEqual(completions,['outcome','clear']);
    await waitFor(async () => (await context.cookies()).some(cookie => cookie.value.startsWith('writer-')));
    await page.evaluate(() => clearInterval(globalThis.__charioxWriter));

    await page.evaluate(() => new Promise((resolve,reject) => {
      const worker = new Worker('/cookie-worker.js');
      worker.addEventListener('error',reject,{once:true});
      worker.addEventListener('message',event => {
        if (event.data === 'ready') {
          globalThis.__charioxCookieWorker = worker;
          resolve();
        }
      });
    }));
    // Do not reconcile after creating the worker: the fence must treat a writer
    // first discovered during acquisition as untracked and close it.
    workerRequestStarted = Promise.withResolvers();
    workerResponseGate = Promise.withResolvers();
    await page.evaluate(() => globalThis.__charioxCookieWorker.postMessage('write'));
    await workerRequestStarted.promise;
    let durableCompletionStarted = false;
    completions.length = 0;
    const workerImport = applyControllerCookieImport({...selection,overwrite:true,
      browserGeneration:refreshed.browser_generation,documentId:current.document_id},{...authority,
      complete:async payload => {
        durableCompletionStarted = true;
        workerResponseGate.resolve();
        await delay(100);
        assert.equal((await context.cookies()).find(cookie => cookie.name === 'session').value,
          'fixture-controller','an untracked worker response cannot mutate cookies after import');
        await authority.complete(payload);
      }});
    assert.deepEqual(await workerImport,{cookieCount:1,domains:['example.test']});
    assert.equal(durableCompletionStarted,true);
    assert.deepEqual(completions,['outcome','clear']);
    workerRequestStarted = undefined;
    workerResponseGate = undefined;
    await page.evaluate(() => {
      globalThis.__charioxCookieWorker.terminate();
      delete globalThis.__charioxCookieWorker;
    });

    refreshed = await controller.reconcile(viewport);
    current = refreshed.tabs.find(tab => tab.url === 'https://example.test/');
    const beforeRecovery = await context.cookies();
    await assert.rejects(applyControllerCookieImport({...selection,overwrite:true,
      browserGeneration:refreshed.browser_generation,documentId:current.document_id},{...authority,
      complete:async()=>{throw new Error('private durable store failure');}}),error =>
      error.code === 'cookie_import_completion_failed' && error.recoveryRequired === true
        && !String(error).includes('private durable store failure'));
    await assert.rejects(applyControllerCookieImport({...selection,overwrite:true,
      browserGeneration:refreshed.browser_generation,documentId:current.document_id},authority),
      {code:'cookie_import_recovery_required',recoveryRequired:true});
    assert.equal((await context.cookies()).find(cookie => cookie.name === 'session').value,'fixture-controller');
    // Replace the executor connection before replay. Recovery must acquire a
    // fresh writer fence before it reads or restores the retained journal.
    await controller.close();
    controller = new BrowserCdpClient({debuggerEndpoint:`http://127.0.0.1:${port}`});
    const recoveredState = await controller.reconcile(viewport);
    const recoveredTab = recoveredState.tabs.find(tab => tab.url === 'https://example.test/');
    const completion = [];
    const recovery = await recoverControllerCookieImport({controller,targetId:recoveredTab.target_id},{
      journal,authorize:async()=>true,runExclusive:async operation=>operation(),
      complete:async result => completeCookieImport({receipt:result.receipt,journal,
        recordOutcome:async()=>{completion.push('outcome');},
        clearPending:async()=>{completion.push('clear');}}),
    });
    assert.equal(recovery.recovered,true);
    assert.deepEqual(await context.cookies(),beforeRecovery);
    await page.reload();
    assert.equal(await page.textContent('body'),'SIGNED_OUT');
    assert.deepEqual(completion,['outcome','clear']);
  } finally {
    const closed = await Promise.allSettled([
      journal?.close(), controller?.close(), context?.close(),
    ]);
    key.fill(0);
    await Promise.all([rm(journalDirectory,{recursive:true,force:true}),
      rm(profile,{recursive:true,force:true})]);
    const failures = closed.filter(result => result.status === 'rejected');
    if (failures.length) throw new AggregateError(failures.map(result => result.reason),'fixture cleanup failed');
  }
});

async function waitFor(predicate) {
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await delay(10);
  }
  assert.fail('timed out waiting for concurrent cookie writer');
}
