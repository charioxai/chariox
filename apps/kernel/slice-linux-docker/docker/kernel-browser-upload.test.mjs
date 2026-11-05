// MP-08/MP-09/MP-10/MP-11: the kernel browser's document-bound upload input.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, mkdir, symlink, rm, truncate } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { uploadHostFiles, publicUploadFailure } from './kernel-browser-upload.mjs';
import { BrowserFileTransferError } from './browser-controller-files.mjs';

async function fixture(run) {
  const root = await mkdtemp(path.join(tmpdir(), 'kernel-upload-'));
  const workspace = path.join(root, 'workspace');
  await mkdir(workspace);
  await writeFile(path.join(workspace, 'one.txt'), 'one');
  await writeFile(path.join(workspace, 'two.txt'), 'two');
  await writeFile(path.join(root, 'outside.txt'), 'outside');
  const calls = [];
  let inspected = 'file', document = 'doc', dispatched = 0, staged = 0;
  const connection = { async send(method, params) {
    calls.push({ method, params });
    if (method === 'Page.getFrameTree') return { frameTree: { frame: { id: 'frame', loaderId: document } } };
    if (method === 'DOM.resolveNode') return { object: { objectId: 'input' } };
    if (method === 'Runtime.callFunctionOn') return { result: { value: inspected } };
    return {};
  } };
  const browser = { async resolvePageTarget() { return { connection, sessionId: 'page' }; } };
  const options = { uploadRoots: [workspace], onDispatch() { dispatched++; },
    stageUploads: async ({ files }) => { staged++; return { files, async markExposed() {}, async discard() {} }; } };
  const upload = (input = {}, overrides = {}) => uploadHostFiles(browser, { target_id: 'target', document_id: 'doc' },
    { kind: 'upload', target: 'backend:7', path: 'one.txt', ...input }, { ...options, ...overrides });
  try { await run({ root, workspace, calls, upload, setInspected: value => inspected = value,
    setDocument: value => document = value, dispatched: () => dispatched, staged: () => staged }); }
  finally { await rm(root, { recursive: true, force: true }); }
}

test('workspace file and two files reach CDP once without returning paths', () => fixture(async f => {
  const one = await f.upload();
  assert.equal(one.file_count, 1); assert.equal(one.total_bytes, 3);
  const two = await f.upload({ path: undefined, paths: ['one.txt', 'two.txt'] });
  assert.equal(two.file_count, 2); assert.equal(two.total_bytes, 6);
  assert.equal(f.dispatched(), 2);
  assert.equal(f.calls.filter(c => c.method === 'DOM.setFileInputFiles').length, 2);
  assert(!JSON.stringify(two).includes(f.root));
}));

test('outside paths, traversal and symlink escapes use existing domain refusal', () => fixture(async f => {
  await symlink(path.join(f.root, 'outside.txt'), path.join(f.workspace, 'escape.txt'));
  for (const candidate of [path.join(f.root, 'outside.txt'), '../outside.txt', 'a/../one.txt', 'escape.txt']) {
    await assert.rejects(f.upload({ path: candidate }), { code: 'user_domain_not_granted' });
  }
  assert.equal(f.dispatched(), 0); assert.equal(f.staged(), 0);
}));

test('missing files, directories, non-regular files and byte/count caps fail plainly', () => fixture(async f => {
  execFileSync('mkfifo', [path.join(f.workspace, 'pipe')]);
  for (const candidate of ['missing.txt', '.', 'pipe']) {
    await assert.rejects(f.upload({ path: candidate }), { code: 'browser_upload_invalid' });
  }
  await writeFile(path.join(f.workspace, 'large'), '');
  await truncate(path.join(f.workspace, 'large'), 512 * 1024 * 1024 + 1);
  await assert.rejects(f.upload({ path: 'large' }), /total bytes/);
  await assert.rejects(f.upload({ path: undefined, paths: Array(21).fill('one.txt') }), /20/);
  assert.equal(f.dispatched(), 0); assert.equal(f.staged(), 0);
}));

test('public upload failures expose only fixed reasons', () => {
  assert.equal(publicUploadFailure(new BrowserFileTransferError('browser_upload_invalid', '/private/value')), null);
  assert.equal(publicUploadFailure(new Error('upload target must allow multiple files')), null);
  assert.deepEqual(publicUploadFailure(new BrowserFileTransferError('browser_upload_invalid', 'upload target must allow multiple files')),
    { code: 'browser_upload_invalid', message: 'upload target must allow multiple files' });
});

test('non-file controls never click a chooser and stale nodes never dispatch', () => fixture(async f => {
  f.setInspected('invalid');
  await assert.rejects(f.upload(), /input type=file/);
  f.setInspected('detached');
  await assert.rejects(f.upload(), { code: 'stale_element_reference' });
  f.setInspected('single');
  await assert.rejects(f.upload({ path: undefined, paths: ['one.txt', 'two.txt'] }), /multiple/);
  assert.equal(f.dispatched(), 0);
  assert(!f.calls.some(c => c.method === 'Input.dispatchMouseEvent' || c.method === 'Page.setInterceptFileChooserDialog'));
}));

test('document retirement and grant cancellation fence the physical upload', () => fixture(async f => {
  f.setDocument('retired');
  await assert.rejects(f.upload(), { code: 'stale_document_reference' });
  f.setDocument('doc');
  const cancelled = new AbortController();
  await assert.rejects(f.upload({}, { signal: cancelled.signal, inputGuard() { cancelled.abort(); } }), { code: 'browser_action_cancelled' });
  assert.equal(f.dispatched(), 0);
}));
