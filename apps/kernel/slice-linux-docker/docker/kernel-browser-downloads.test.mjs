// MP-08/MP-09/MP-10/MP-11: the host's CDP download lifecycle and storage contract.
import test from 'node:test';
import assert from 'node:assert/strict';
import { access, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { HostDownloads } from './kernel-browser-downloads.mjs';

async function using(run, limits) {
  const handlers = new Set(), sent = [], frames = new Map([['fa','ta'],['fb','tb']]);
  const connection = { subscribe: f => { handlers.add(f); return () => handlers.delete(f); }, send: async (method, params) => { sent.push({method,params}); } };
  const browser = { targetsByFrame: frames, targetsByDownload: new Map() };
  const ownerRoot = randomUUID();
  const downloads = new HostDownloads({ ownerRoot, notify: () => {}, ...limits });
  await downloads.start(connection, browser);
  downloads.bind({tab_id:'a',target_id:'ta'}, 'room-a');
  downloads.bind({tab_id:'b',target_id:'tb'}, 'room-b');
  const emit = (method, params) => { for (const handler of handlers) handler({method:`Browser.${method}`,params}); };
  const begin = (guid, frameId='fa') => emit('downloadWillBegin',{guid,frameId,suggestedFilename:'../../report.txt'});
  const progress = (guid,state,receivedBytes,totalBytes=receivedBytes) => emit('downloadProgress',{guid,state,receivedBytes,totalBytes});
  try { await run({downloads, sent, begin, progress, ownerRoot}); }
  finally { await downloads.close(); assert.equal(handlers.size,0); }
}

test('MP-08/MP-10 two tabs keep independent download identities and readable completed files', () => using(async ({downloads,sent,begin,progress}) => {
  assert.equal(sent[0].params.behavior,'allowAndName');
  assert.equal(sent[0].params.eventsEnabled,true);
  begin('one'); begin('two','fb'); progress('one','inProgress',2,5);
  assert.deepEqual(downloads.state().map(d=>[d.download_id,d.tab_id,d.state,d.bytes,d.path]),[['one','a','in_progress',2,null],['two','b','in_progress',0,null]]);
  await writeFile(path.join(downloads.spool,'one'),'hello'); progress('one','completed',5);
  await downloads.settle();
  const done=downloads.state()[0]; assert.equal(done.state,'done'); assert.equal(await readFile(done.path,'utf8'),'hello');
  assert.equal(done.name,'report.txt'); assert.ok(!done.path.includes('/.chariox/'));
  const saved=done.path; await downloads.cleanupSession('room-a');
  await assert.rejects(access(saved)); assert.equal(downloads.state().length,1);
}));

test('MP-08/MP-10 cancel settles after CDP and removes partial bytes; terminal failure has a reason', () => using(async ({downloads,begin,progress,sent}) => {
  begin('one'); await writeFile(path.join(downloads.spool,'one.crdownload'),'partial');
  await assert.rejects(downloads.cancel('one','b'),/tab/);
  await downloads.cancel('one','a'); assert.equal(downloads.state()[0].state,'in_progress');
  assert.equal(sent.at(-1).method,'Browser.cancelDownload'); progress('one','canceled',7);
  await downloads.settle(); assert.equal(downloads.state()[0].state,'cancelled'); assert.equal(downloads.state()[0].path,null);
  await assert.rejects(access(path.join(downloads.spool,'one.crdownload')));
  begin('two'); progress('two','canceled',0); await downloads.settle();
  assert.equal(downloads.state()[1].state,'failed'); assert.equal(downloads.state()[1].reason,'interrupted');
  await assert.rejects(downloads.cancel('one','a'),/active/);
}));

test('MP-08/MP-10 file and Room caps cancel unknown-length and oversized downloads', () => using(async ({downloads,begin,progress,sent}) => {
  begin('one'); progress('one','inProgress',7,0); await downloads.settle();
  assert.equal(sent.filter(c=>c.method==='Browser.cancelDownload').length,1);
  progress('one','canceled',7); await downloads.settle(); assert.equal(downloads.state()[0].reason,'file_size_limit');
  begin('two'); progress('two','inProgress',4,4);
  begin('three'); progress('three','inProgress',4,4); await downloads.settle();
  progress('three','canceled',4); await downloads.settle(); assert.equal(downloads.state().at(-1).reason,'session_size_limit');
}, {maxFileBytes:6,maxSessionBytes:7}));

test('MP-08/MP-11 a missing completed file fails and native downloads remain allowed', () => using(async ({downloads,begin,progress,sent}) => {
  begin('one'); progress('one','completed',2); await downloads.settle(); assert.equal(downloads.state()[0].state,'failed');
  const calls=sent.length; begin('foreign','unknown'); await downloads.settle(); assert.equal(sent.length,calls);
  assert.equal(downloads.state().length,1);
}));


test('MP-08/MP-10 a replacement controller removes artifacts from its stopped predecessor', () => using(async ({downloads,begin,progress,ownerRoot}) => {
  begin('one'); await writeFile(path.join(downloads.spool,'one'),'saved'); progress('one','completed',5); await downloads.settle();
  const saved=downloads.state()[0].path; downloads.off();
  const replacement=new HostDownloads({ownerRoot});
  // Recreate the same kernel/profile binding after losing in-memory controller state.
  await replacement.start({subscribe:()=>()=>{},send:async()=>{}},{});
  try { await assert.rejects(access(saved)); assert.deepEqual(replacement.state(),[]); }
  finally { await replacement.close(); }
}));


test('MP-08/MP-10 the bounded record cap reports the refused download and removes its bytes', () => using(async ({downloads,begin,sent}) => {
  for(let index=0;index<200;index++)begin(`download-${index}`);
  await writeFile(path.join(downloads.spool,'overflow'),'partial');
  begin('overflow');await downloads.settle();
  assert.equal(downloads.state().length,201);
  assert.equal(downloads.state().at(-1).reason,'download_count_limit');
  assert.equal(sent.at(-1).method,'Browser.cancelDownload');
  await assert.rejects(access(path.join(downloads.spool,'overflow')));
  begin('latest-overflow');await downloads.settle();
  assert.equal(downloads.state().length,201);assert.equal(downloads.state().at(-1).download_id,'latest-overflow');
}));
