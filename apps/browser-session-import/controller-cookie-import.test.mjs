import assert from 'node:assert/strict';
import test from 'node:test';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {randomBytes} from 'node:crypto';
import {openCookieImportJournal} from './cookie-import-journal.mjs';
import {applyControllerCookieImport} from './controller-cookie-import.mjs';

const source = [{name:'session',value:'fixture-only',domain:'example.test',path:'/',
  secure:true,httpOnly:true,hostOnly:true,session:true,sameSite:'lax',storeId:'source'}];
const scope = {approvedDomains:['example.test'],sourceStoreId:'source'};

async function withJournal(operation) {
  const directory = await mkdtemp(path.join(tmpdir(),'chariox-controller-journal-'));
  const key = randomBytes(32);
  let journal;
  try {
    journal = await openCookieImportJournal({directory,key,
      binding:{userId:'user',roomId:'room',environmentId:'environment'}});
    await operation(journal);
  } finally {
    try { await journal?.close(); } finally {
      key.fill(0);
      await rm(directory,{recursive:true,force:true});
    }
  }
}

function fixture(journal, beforeWrite = async () => {}) {
  const calls = [];
  let cookies = [];
  const connection = {isOpen:() => true,send:async (method,params) => {
    calls.push(method);
    if (method === 'Page.getFrameTree') return {frameTree:{frame:{loaderId:'document'}}};
    if (method === 'Target.getTargetInfo') return {targetInfo:{targetId:'target'}};
    if (method === 'Storage.getCookies') return {cookies:structuredClone(cookies)};
    if (method === 'Storage.setCookies') {
      await beforeWrite();
      cookies = params.cookies.map(({url,...cookie}) => ({...cookie,
        domain:cookie.domain ?? new URL(url).hostname,
        session:cookie.expires === undefined,expires:cookie.expires ?? -1}));
      return {};
    }
    if (method === 'Network.deleteCookies') { cookies=[]; return {}; }
    throw new Error('unexpected fixture command');
  }};
  const controller = {browserGeneration:1,
    resolvePageTarget:async () => ({connection,sessionId:'cdp'}),
    withCookieWritersQuiesced:async operation => operation({retain:()=>{}})};
  return {calls,selection:{source,scope,browserGeneration:1,targetId:'target',documentId:'document',controller},
    authority:{journal,authorize:async () => true,runExclusive:async operation => operation(),
      complete:async ({receipt}) => journal.discard(receipt)}};
}

test('controller bridge refuses mutation without a recovery journal',async () => {
  const {selection,authority,calls} = fixture();
  await assert.rejects(applyControllerCookieImport(selection,authority),{code:'cookie_import_journal_required'});
  assert.deepEqual(calls,[]);
});

test('controller bridge blocks an existing recovery record before cookie access',async () => {
  await withJournal(async journal => {
    const receipt = await journal.prepare(Buffer.from('fixture prior recovery'));
    const {selection,authority,calls} = fixture(journal);
    await assert.rejects(applyControllerCookieImport(selection,authority),
      {code:'cookie_import_recovery_required',recoveryRequired:true});
    assert.equal(calls.some(method => method.startsWith('Storage.')),false);
    const pending = await journal.read();
    assert.equal(pending.receipt,receipt);
    pending.bytes.fill(0);
  });
});

test('controller bridge acknowledges journal before cookie write and clears it only after completion',async () => {
  await withJournal(async journal => {
    let receipt;
    let journalPresentAtCompletion = false;
    const {selection,authority} = fixture(journal,async () => {
      const pending = await journal.read();
      assert.ok(pending,'cookie write must have an acknowledged journal');
      receipt=pending.receipt;
      pending.bytes.fill(0);
    });
    authority.complete = async ({receipt:completedReceipt}) => {
      const pending = await journal.read();
      journalPresentAtCompletion = pending?.receipt === completedReceipt;
      pending?.bytes.fill(0);
      await journal.discard(completedReceipt);
    };
    assert.deepEqual(await applyControllerCookieImport(selection,authority),
      {cookieCount:1,domains:['example.test'],results:[
        {domain:'example.test',status:'imported',cookie_count:1},
      ]});
    assert.equal(journalPresentAtCompletion,true);
    assert.equal(typeof receipt,'string');
    assert.equal(await journal.read(),null);
  });
});

test('controller bridge stops before mutation if journal preparation fails',async () => {
  const journal = {read:async () => null,prepare:async () => {throw new Error('fixture secret');},
    discard:async () => {throw new Error('must not discard');}};
  const {selection,authority,calls} = fixture(journal);
  await assert.rejects(applyControllerCookieImport(selection,authority),error => {
    assert.equal(error.code,'cookie_import_failed');
    assert.equal(String(error).includes('fixture secret'),false);
    return true;
  });
  assert.equal(calls.includes('Storage.setCookies'),false);
});

test('acknowledged request cancellation after journal preparation prevents mutation and resolves rollback',async () => {
  await withJournal(async durable => {
    const cancellation=new AbortController();
    const journal={read:() => durable.read(),discard:receipt => durable.discard(receipt),
      prepare:async bytes => { const receipt=await durable.prepare(bytes); cancellation.abort(); return receipt; }};
    const {selection,authority,calls}=fixture(journal);
    authority.signal=cancellation.signal;
    await assert.rejects(applyControllerCookieImport(selection,authority),error => {
      assert.equal(error.code,'cookie_import_cancelled');
      assert.equal(error.recoveryRequired,false);
      return true;
    });
    assert.equal(calls.includes('Storage.setCookies'),false);
    assert.equal(await durable.read(),null);
  });
});
