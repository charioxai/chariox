import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {chmod, link, mkdtemp, readFile, readdir, rename, rm, stat, symlink, truncate, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {openCookieImportJournal} from './cookie-import-journal.mjs';

const binding = {userId:'owner-1',roomId:'room-1',environmentId:'environment-1'};
const key = Buffer.alloc(32, 71);
async function fixture(t) {
  const directory = await mkdtemp(path.join(tmpdir(),'chariox-import-journal-'));
  const journals = [];
  t.after(async () => {
    try { for (const journal of journals) await journal.close(); }
    finally { await rm(directory,{recursive:true,force:true}); }
  });
  const open = async (options = {}) => {
    const journal = await openCookieImportJournal({directory,key,binding,...options});
    journals.push(journal);
    return journal;
  };
  return {directory,open};
}

test('pending recovery bytes survive reopening without plaintext on disk', async t => {
  const {directory,open} = await fixture(t);
  const bytes = Buffer.from('fixture-cookie-recovery-value');
  const journal = await open();
  assert.equal(await journal.read(),null);
  const receipt = await journal.prepare(bytes);
  await journal.close();
  const files = await readdir(directory);
  assert.equal(files.length,1);
  const stored = await readFile(path.join(directory,files[0]));
  assert.equal(stored.includes(bytes),false);
  assert.equal(stored.includes(key),false);
  const reopened = await open();
  const recovered = await reopened.read();
  assert.deepEqual(recovered.bytes,bytes);
  assert.equal(recovered.receipt,receipt);
  recovered.bytes.fill(0);
  await reopened.discard(receipt);
  assert.equal(await reopened.read(),null);
});

test('a trailing slash does not allow a symlink to stand in for the private journal directory', async t => {
  const {directory,open} = await fixture(t);
  const alias = path.join(directory,'alias');
  await symlink(directory,alias);
  await assert.rejects(open({directory:alias + '/'}),{code:'cookie_import_journal_directory',recoveryRequired:true});
});

test('uncertain cleanup disables the handle even after the filesystem is repaired',
  {skip:process.getuid() === 0 && 'root bypasses directory write permissions'}, async t => {
  const {directory,open} = await fixture(t);
  const journal = await open();
  const receipt = await journal.prepare(Buffer.from('fixture-recovery'));
  await chmod(directory,0o500);
  try {
    await assert.rejects(journal.discard(receipt),{code:'cookie_import_journal_io_failed',recoveryRequired:true});
  } finally { await chmod(directory,0o700); }
  await assert.rejects(journal.prepare(Buffer.from('another import')),{code:'cookie_import_journal_unavailable',recoveryRequired:true});
  await assert.rejects(journal.read(),{code:'cookie_import_journal_unavailable',recoveryRequired:true});
  await journal.close();
  const reopened = await open();
  const pending = await reopened.read();
  assert.equal(pending.bytes.toString(),'fixture-recovery');
  pending.bytes.fill(0);
});

test('competing prepares cannot overwrite pending recovery, and stale receipts cannot delete it', async t => {
  const {open} = await fixture(t);
  const first = await open(), second = await open();
  const results = await Promise.allSettled([
    first.prepare(Buffer.from('first fixture')),second.prepare(Buffer.from('second fixture')),
  ]);
  assert.equal(results.filter(result => result.status === 'fulfilled').length,1);
  assert.equal(results.find(result => result.status === 'rejected').reason.code,'cookie_import_journal_busy');
  const pending = await first.read();
  const selected = results[0].status === 'fulfilled' ? 'first fixture' : 'second fixture';
  assert.equal(pending.bytes.toString(),selected);
  pending.bytes.fill(0);
  await assert.rejects(second.prepare(Buffer.from('later import')),{code:'cookie_import_recovery_required'});
  await assert.rejects(first.discard('0'.repeat(64)),{code:'cookie_import_journal_receipt'});
  await first.discard(pending.receipt);
  const next = await second.prepare(Buffer.from('next fixture'));
  await assert.rejects(first.discard(pending.receipt),{code:'cookie_import_journal_receipt'});
  await second.discard(next);
  assert.equal(await second.read(),null);
});

test('an overlapping call on one handle rejects without interrupting the pending write', async t => {
  const {open} = await fixture(t);
  const journal = await open();
  const bytes = Buffer.from('original fixture');
  const prepare = journal.prepare(bytes);
  bytes.fill(0);
  await assert.rejects(journal.read(),{code:'cookie_import_journal_busy'});
  await assert.rejects(journal.close(),{code:'cookie_import_journal_busy'});
  const receipt = await prepare;
  const pending = await journal.read();
  assert.equal(pending.bytes.toString(),'original fixture');
  pending.bytes.fill(0);
  await journal.discard(receipt);
});

test('separate handles cannot interleave discard and prepare on the same directory', async t => {
  const {open} = await fixture(t);
  const first = await open(), second = await open();
  const receipt = await first.prepare(Buffer.from('fixture pending recovery'));
  const discard = first.discard(receipt);
  await assert.rejects(second.prepare(Buffer.from('next import')),{code:'cookie_import_journal_busy'});
  await discard;
  const next = await second.prepare(Buffer.from('next import'));
  await assert.rejects(first.discard(receipt),{code:'cookie_import_journal_receipt'});
  await second.discard(next);
});

test('wrong keys and cross-user, Room or Environment recovery fail without deleting the record', async t => {
  const {open} = await fixture(t);
  const original = await open();
  const receipt = await original.prepare(Buffer.from('fixture private state'));
  await original.close();
  for (const options of [
    {key:Buffer.alloc(32,72)},
    ...['userId','roomId','environmentId'].map(field => ({binding:{...binding,[field]:'different'}})),
  ]) {
    const wrong = await open(options);
    await assert.rejects(wrong.read(),{code:'cookie_import_journal_invalid',recoveryRequired:true});
    await assert.rejects(wrong.discard(receipt),{code:'cookie_import_journal_invalid',recoveryRequired:true});
    await wrong.close();
  }
  const recovered = await open();
  const pending = await recovered.read();
  assert.equal(pending.bytes.toString(),'fixture private state');
  pending.bytes.fill(0);
});

test('corruption and truncated records never authorize overwrite or return plaintext', async t => {
  const {directory,open} = await fixture(t);
  const journal = await open();
  await journal.prepare(Buffer.from('fixture private state'));
  const filename = path.join(directory,(await readdir(directory))[0]);
  const original = await readFile(filename);
  const modified = Buffer.from(original);
  modified[modified.length - 1] ^= 1;
  for (const bytes of [modified,original.subarray(0,1),original.subarray(0,-1),Buffer.alloc(0)]) {
    await writeFile(filename,bytes);
    await assert.rejects(journal.read(),{code:'cookie_import_journal_invalid',recoveryRequired:true});
    await assert.rejects(journal.prepare(Buffer.from('new import')),{code:'cookie_import_recovery_required'});
    assert.deepEqual(await readFile(filename),bytes);
  }
});

test('journal files must be private regular files with no extra hard links', async t => {
  const {directory,open} = await fixture(t);
  const journal = await open();
  await journal.prepare(Buffer.from('fixture private state'));
  const filename = path.join(directory,(await readdir(directory))[0]);
  assert.equal((await stat(filename)).mode & 0o777,0o600);
  await chmod(filename,0o644);
  await assert.rejects(journal.read(),{code:'cookie_import_journal_invalid'});
  await chmod(filename,0o600);
  const alias = path.join(directory,'duplicate');
  await link(filename,alias);
  await assert.rejects(journal.read(),{code:'cookie_import_journal_invalid'});
  await rm(alias);
  await rename(filename,alias);
  await symlink(alias,filename);
  await assert.rejects(journal.read(),{code:'cookie_import_journal_io_failed'});
  assert.equal((await readFile(alias)).includes(Buffer.from('fixture private state')),false);
});

test('a replaced or exposed directory is not accepted by an existing handle', async t => {
  const {directory,open} = await fixture(t);
  const journal = await open();
  await chmod(directory,0o755);
  await assert.rejects(journal.prepare(Buffer.from('fixture')),{code:'cookie_import_journal_directory'});
  await chmod(directory,0o700);
  // Renaming the owned directory preserves it for cleanup, while changing the caller's path.
  const moved = directory + '-moved';
  await rename(directory,moved);
  try {
    await symlink(moved,directory);
    await assert.rejects(journal.read(),{code:'cookie_import_journal_directory'});
  } finally {
    await rm(directory,{force:true});
    await rename(moved,directory);
  }
});

test('invalid configuration rejects with a fixed code before opening storage', async t => {
  const {directory} = await fixture(t);
  for (const options of [undefined,null,{},
    {directory:'relative',key,binding},
    {directory,key:Buffer.alloc(31),binding},
    {directory,key,binding:{...binding,roomId:'bad\nroom'}},
    {directory,key,binding:{...binding,userId:''}},
  ]) {
    await assert.rejects(openCookieImportJournal(options),{code:'cookie_import_journal_configuration',recoveryRequired:false});
  }
  assert.deepEqual(await readdir(directory),[]);
});

test('payload and file size limits are enforced without losing the existing recovery record', async t => {
  const {directory,open} = await fixture(t);
  const journal = await open();
  for (const bytes of ['',Buffer.alloc(0),Buffer.alloc(8 * 1024 * 1024 + 1)]) {
    await assert.rejects(journal.prepare(bytes),{code:'cookie_import_journal_payload',recoveryRequired:false});
  }
  assert.deepEqual(await readdir(directory),[]);
  const bytes = Buffer.alloc(8 * 1024 * 1024,93);
  const receipt = await journal.prepare(bytes);
  const pending = await journal.read();
  assert.deepEqual(pending.bytes,bytes);
  pending.bytes.fill(0);
  bytes.fill(0);
  const filename = path.join(directory,(await readdir(directory))[0]);
  await truncate(filename,9 * 1024 * 1024);
  await assert.rejects(journal.read(),{code:'cookie_import_journal_invalid',recoveryRequired:true});
  await assert.rejects(journal.discard(receipt),{code:'cookie_import_journal_invalid'});
});

test('closed handles cannot read or mutate, and closing does not destroy the caller key', async t => {
  const {open} = await fixture(t);
  const callerKey = Buffer.alloc(32,92);
  const journal = await open({key:callerKey});
  const receipt = await journal.prepare(Buffer.from('fixture'));
  await journal.close();
  await journal.close();
  assert.deepEqual(callerKey,Buffer.alloc(32,92));
  await assert.rejects(journal.read(),{code:'cookie_import_journal_closed'});
  await assert.rejects(journal.discard(receipt),{code:'cookie_import_journal_closed'});
  await assert.rejects(journal.prepare(Buffer.from('next')),{code:'cookie_import_journal_closed'});
});

async function childRun(t,directory,body,limited = false) {
  const script = `import {openCookieImportJournal} from ${JSON.stringify(new URL('./cookie-import-journal.mjs',import.meta.url).href)};
    const journal = await openCookieImportJournal({directory:process.env.JOURNAL_TEST_DIR,
      key:Buffer.alloc(32,71),binding:${JSON.stringify(binding)}});
    ${body}`;
  const args = ['--max-old-space-size=64','--input-type=module','--eval',script];
  const child = limited
    ? spawn('/bin/sh',['-c','ulimit -c 0; ulimit -f 1; exec "$@"','journal-drill',process.execPath,...args],
      {stdio:'ignore',env:{...process.env,JOURNAL_TEST_DIR:directory}})
    : spawn(process.execPath,args,{stdio:'ignore',env:{...process.env,JOURNAL_TEST_DIR:directory}});
  const timeout = setTimeout(() => child.kill('SIGKILL'),5000);
  const done = new Promise((resolve,reject) => {
    child.once('error',reject);
    child.once('exit',(code,signal) => resolve({code,signal}));
  });
  t.after(async () => {
    clearTimeout(timeout);
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    await done;
  });
  try { return await done; }
  finally { clearTimeout(timeout); }
}

test('an abrupt writer process death leaves acknowledged recovery readable', async t => {
  const {directory,open} = await fixture(t);
  const result = await childRun(t,directory,`
    await journal.prepare(Buffer.from('fixture persisted before death'));
    process.kill(process.pid,'SIGKILL');`);
  assert.equal(result.signal,'SIGKILL');
  const journal = await open();
  const pending = await journal.read();
  assert.equal(pending.bytes.toString(),'fixture persisted before death');
  pending.bytes.fill(0);
  await assert.rejects(journal.prepare(Buffer.from('new import')),{code:'cookie_import_recovery_required'});
  await journal.discard(pending.receipt);
});

test('independent writer processes cannot replace each other\'s pending record', async t => {
  const {directory,open} = await fixture(t);
  const results = await Promise.all(['first writer','second writer'].map(value => childRun(t,directory,`
    try { await journal.prepare(Buffer.from(${JSON.stringify(value)})); process.exit(0); }
    catch (error) { process.exit(error.code === 'cookie_import_recovery_required' ? 17 : 20); }`)));
  assert.deepEqual(results.map(result => result.code).sort((a,b) => a-b),[0,17]);
  const journal = await open();
  const pending = await journal.read();
  assert.equal(pending.bytes.toString(),results[0].code === 0 ? 'first writer' : 'second writer');
  pending.bytes.fill(0);
});

test('an OS-limited interrupted write leaves a blocking record instead of silently restarting', async t => {
  const {directory,open} = await fixture(t);
  const result = await childRun(t,directory,`
    try { await journal.prepare(Buffer.alloc(65536,98)); process.exit(12); }
    catch (error) { process.exit(error.code === 'cookie_import_journal_io_failed' ? 0 : 20); }`,true);
  assert.ok(result.signal === 'SIGXFSZ' || result.code === 0,JSON.stringify(result));
  const journal = await open();
  await assert.rejects(journal.read(),{code:'cookie_import_journal_invalid',recoveryRequired:true});
  await assert.rejects(journal.prepare(Buffer.from('new import')),{code:'cookie_import_recovery_required'});
});
