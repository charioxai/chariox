import assert from 'node:assert/strict';
import test from 'node:test';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {randomBytes} from 'node:crypto';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {openCookieImportJournal} from './cookie-import-journal.mjs';
import { applyCookieImport } from './cookie-import-transaction.mjs';

const source = [{name:'session', value:'fixture-only', domain:'example.test', path:'/',
  secure:true, httpOnly:true, hostOnly:true, session:true, sameSite:'lax', storeId:'source'}];
const scope = {approvedDomains:['example.test'], sourceStoreId:'source'};

test('executor death during write leaves a recovery record that blocks a restarted import', async () => {
  const directory = await mkdtemp(path.join(tmpdir(),'chariox-import-crash-'));
  const key = Buffer.alloc(32,73); // Synthetic fixture key, never a user credential.
  const binding = {userId:'user',roomId:'room',environmentId:'environment'};
  let journal;
  try {
    const script = `
      import {applyCookieImport} from ${JSON.stringify(new URL('./cookie-import-transaction.mjs',import.meta.url).href)};
      import {openCookieImportJournal} from ${JSON.stringify(new URL('./cookie-import-journal.mjs',import.meta.url).href)};
      const journal = await openCookieImportJournal({directory:process.argv[1],key:Buffer.alloc(32,73),binding:${JSON.stringify(binding)}});
      await applyCookieImport({source:${JSON.stringify(source)},scope:${JSON.stringify(scope)},journal,
        authorize:async()=>true,runExclusive:async fn=>fn(),
        store:{read:async()=>[],write:async()=>process.exit(86),remove:async()=>{throw Error('unexpected rollback');}}});
    `;
    await assert.rejects(promisify(execFile)(process.execPath,
      ['--input-type=module','-e',script,directory],{timeout:5000,maxBuffer:8192}),
      error => error.code === 86 && !error.killed);
    journal = await openCookieImportJournal({directory,key,binding});
    const pending = await journal.read();
    assert.ok(pending);
    try {
      const record = JSON.parse(pending.bytes.toString());
      assert.equal(record.schema,1);
      assert.deepEqual(record.before,[]);
      assert.equal(record.imported[0].value,'fixture-only');
    } finally { pending.bytes.fill(0); }
    const {options,writes} = fixture();
    await assert.rejects(applyCookieImport({...options,journal}),
      {code:'cookie_import_recovery_required',recoveryRequired:true});
    assert.equal(writes(),0);
  } finally {
    await journal?.close();
    key.fill(0);
    await rm(directory,{recursive:true,force:true});
  }
});

async function withJournal(operation) {
  const directory = await mkdtemp(path.join(tmpdir(),'chariox-cookie-transaction-'));
  const key = randomBytes(32);
  const journal = await openCookieImportJournal({directory,key,
    binding:{userId:'user',roomId:'room',environmentId:'environment'}});
  try { await operation(journal); }
  finally { await journal.close(); key.fill(0); await rm(directory,{recursive:true,force:true}); }
}
const stored = (changes = {}) => ({name:'session', value:'fixture-only', domain:'example.test', path:'/',
  secure:true, httpOnly:true, session:true, expires:-1, sameSite:'Lax', ...changes});
function fixture(initial = []) {
  let jar = structuredClone(initial);
  let writes = 0;
  const identity = c => JSON.stringify([c.name,c.domain,c.path,c.partitionKey ?? null]);
  const store = {
    read:async () => structuredClone(jar),
    write:async cookies => {
      writes++;
      for (const c of cookies) {
        const record = {...c, domain:c.domain ?? new URL(c.url).hostname,
          expires:c.expires ?? -1, session:c.expires === undefined};
        delete record.url;
        jar = jar.filter(other => identity(other) !== identity(record));
        jar.push(record);
      }
    },
    remove:async keys => { jar = jar.filter(c => !keys.some(k => identity(k) === identity(c))); },
  };
  return {store, writes:() => writes,
    options:{source,scope,store,authorize:async () => true, runExclusive:async fn => fn()}};
}

test('publishes a verified import while leaving unrelated cookies untouched', async () => {
  const unrelated = stored({domain:'other.test', value:'control'});
  const {options,store} = fixture([unrelated]);
  assert.deepEqual(await applyCookieImport(options), {cookieCount:1, domains:['example.test']});
  const after = await store.read();
  assert.deepEqual(after.find(c => c.domain === 'other.test'), unrelated);
  assert.equal(after.find(c => c.domain === 'example.test').httpOnly, true);
});

test('encrypted recovery record is durable before the first cookie write and cleared after verification', async () => {
  await withJournal(async journal => {
    const {options,store} = fixture();
    const write = store.write;
    store.write = async cookies => {
      const pending = await journal.read();
      assert.ok(pending);
      try {
        const record = JSON.parse(pending.bytes.toString());
        assert.equal(record.schema,1);
        assert.deepEqual(record.before,[]);
        assert.equal(record.imported.length,1);
      } finally { pending.bytes.fill(0); }
      await write(cookies);
    };
    await applyCookieImport({...options,journal});
    assert.equal(await journal.read(),null);
  });
});

test('unresolved journal blocks another import without changing cookies', async () => {
  await withJournal(async journal => {
    await journal.prepare(Buffer.from('retained encrypted recovery'));
    const {options,writes} = fixture();
    await assert.rejects(applyCookieImport({...options,journal}),
      {code:'cookie_import_recovery_required',recoveryRequired:true});
    assert.equal(writes(),0);
  });
});

test('uncertain mutation failure retains encrypted recovery state', async () => {
  await withJournal(async journal => {
    const {options,store} = fixture();
    store.write = async () => { throw Error('private controller payload'); };
    await assert.rejects(applyCookieImport({...options,journal}),
      {code:'cookie_import_failed',recoveryRequired:true});
    const pending = await journal.read();
    assert.ok(pending);
    pending.bytes.fill(0);
  });
});

test('revocation during journal preparation does not mutate the cookie store', async () => {
  await withJournal(async journal => {
    const {options,store,writes} = fixture();
    let allowed = true;
    let removals = 0;
    store.remove = async () => { removals++; };
    const guardedJournal = {...journal, prepare:async bytes => {
      const receipt = await journal.prepare(bytes);
      allowed = false;
      return receipt;
    }};
    await assert.rejects(applyCookieImport({...options,journal:guardedJournal,
      authorize:async () => allowed}), {code:'cookie_import_denied',recoveryRequired:false});
    assert.equal(writes(),0);
    assert.equal(removals,0);
    assert.equal(await journal.read(),null);
  });
});

test('failed journal preparation prevents mutation and preserves recovery classification', async () => {
  const {options,writes} = fixture();
  await assert.rejects(applyCookieImport({...options,journal:{
    read:async () => null,
    prepare:async () => { throw Object.assign(Error('private storage details'),{recoveryRequired:true}); },
  }}), {code:'cookie_import_failed',message:'cookie_import_failed',recoveryRequired:true});
  assert.equal(writes(),0);
});

test('failed journal cleanup after verified application blocks subsequent imports', async () => {
  await withJournal(async journal => {
    const {options,store,writes} = fixture();
    await assert.rejects(applyCookieImport({...options,journal:{...journal,
      discard:async () => { throw Error('private filesystem details'); },
    }}), {code:'cookie_import_recovery_required',recoveryRequired:true});
    assert.equal(writes(),1);
    assert.equal((await store.read())[0].value,'fixture-only');
    await assert.rejects(applyCookieImport({...options,journal}),
      {code:'cookie_import_recovery_required',recoveryRequired:true});
    assert.equal(writes(),1);
  });
});

test('requires a fresh exclusive authorization and explicit overwrite before mutation', async () => {
  for (const mode of ['denied','missing-authorization','missing-exclusive','cancelled','conflict']) {
    const {options,writes} = fixture(mode === 'conflict' ? [stored({value:'existing'})] : []);
    if (mode === 'denied') options.authorize = async () => false;
    if (mode === 'missing-authorization') delete options.authorize;
    if (mode === 'missing-exclusive') delete options.runExclusive;
    if (mode === 'cancelled') options.signal = AbortSignal.abort('secret-marker');
    await assert.rejects(applyCookieImport(options), error =>
      ['cookie_import_denied','cookie_import_cancelled','cookie_import_conflict'].includes(error.code));
    assert.equal(writes(), 0);
  }
});

test('detects a silently dropped cookie and restores the replaced destination session', async () => {
  const original = stored({value:'previous-session'});
  const {options,store} = fixture([original]);
  options.overwrite = true;
  const write = store.write;
  let first = true;
  store.write = async cookies => {
    if (first) { first = false; await store.remove([original]); return; }
    return write(cookies);
  };
  await assert.rejects(applyCookieImport(options), {code:'cookie_import_verification_failed', recoveryRequired:false});
  assert.deepEqual(await store.read(), [original]);
});

test('rolls back after revocation or cancellation during a completed write', async () => {
  for (const mode of ['revoked','cancelled']) {
    const original = stored({value:'previous-session'});
    const {options,store} = fixture([original]);
    options.overwrite = true;
    const controller = new AbortController();
    options.signal = controller.signal;
    let allowed = true;
    options.authorize = async () => allowed;
    const write = store.write;
    store.write = async cookies => {
      await write(cookies);
      if (mode === 'revoked') allowed = false;
      else controller.abort('private-marker');
    };
    await assert.rejects(applyCookieImport(options), {
      code:mode === 'revoked' ? 'cookie_import_denied' : 'cookie_import_cancelled', recoveryRequired:false,
    });
    assert.deepEqual(await store.read(), [original]);
  }
});

test('detects unrelated cookie loss and reports recovery required without rewriting that domain', async () => {
  const unrelated = stored({domain:'other.test', value:'control'});
  const {options,store} = fixture([unrelated]);
  const write = store.write;
  store.write = async cookies => {await write(cookies); await store.remove([unrelated]);};
  await assert.rejects(applyCookieImport(options), {code:'cookie_import_verification_failed', recoveryRequired:true});
  assert.deepEqual(await store.read(), []);
});

test('redacts failures and treats a failed write acknowledgement as uncertain even after rollback', async () => {
  for (const mode of ['read','write']) {
    const {options,store,writes} = fixture();
    if (mode === 'read') store.read = async () => {throw new Error('secret-marker');};
    else {
      const write = store.write;
      store.write = async cookies => {await write(cookies); throw new Error('secret-marker');};
    }
    await assert.rejects(applyCookieImport(options), error => {
      assert.equal(String(error).includes('secret-marker'), false);
      return error.code === 'cookie_import_failed' && error.recoveryRequired === (mode === 'write');
    });
    if (mode === 'read') assert.equal(writes(), 0);
    else assert.deepEqual(await store.read(), []);
  }
});

test('rollback preserves destination priority and source scheme/port metadata', async () => {
  const original = stored({value:'previous-session', priority:'High', sourceScheme:'Secure', sourcePort:443});
  const {options,store} = fixture([original]);
  options.overwrite = true;
  const controller = new AbortController();
  options.signal = controller.signal;
  const write = store.write;
  store.write = async cookies => {await write(cookies); controller.abort();};
  await assert.rejects(applyCookieImport(options), {code:'cookie_import_cancelled', recoveryRequired:false});
  assert.deepEqual(await store.read(), [original]);
});

test('rejects oversized or non-restorable snapshots before changing cookies', async () => {
  for (const initial of [
    Array.from({length:10001}, (_, n) => stored({name:`n${n}`})),
    [stored({partitionKeyOpaque:true})],
    [stored({futureSecurityField:true})],
    [stored({sourcePort:443}), stored({sourcePort:8443})],
  ]) {
    const {options,writes} = fixture(initial);
    options.overwrite = true;
    await assert.rejects(applyCookieImport(options), error =>
      ['cookie_import_snapshot_too_large','cookie_import_snapshot_unsupported'].includes(error.code));
    assert.equal(writes(),0);
  }
});

test('an empty approved import performs no cookie-store reads or writes', async () => {
  const {options,store,writes} = fixture();
  options.source = [];
  store.read = async () => {throw new Error('must not read unrelated cookies');};
  assert.deepEqual(await applyCookieImport(options), {cookieCount:0,domains:[]});
  assert.equal(writes(),0);
});

test('rejects non-restorable expiry and session combinations before replacing a cookie', async () => {
  for (const change of [
    {session:false,expires:null}, {session:false,expires:Infinity},
    {session:false,expires:NaN}, {session:false,expires:-1},
    {session:true,expires:null}, {session:true,expires:2000},
  ]) {
    const original = stored(change);
    const {options,store,writes} = fixture([original]);
    options.overwrite = true;
    await assert.rejects(applyCookieImport(options), {code:'cookie_import_snapshot_unsupported',recoveryRequired:false});
    assert.equal(writes(),0);
    assert.deepEqual(await store.read(),[original]);
  }
});
