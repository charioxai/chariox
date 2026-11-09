// MP-08/MP-09/MP-10/MP-11: browsertools r1 op 2. Save a resource the page shows into the Room workspace.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, mkdir, symlink, rm, readFile, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { saveResource, SAVE_LIMIT } from './kernel-browser-save.mjs';

const PNG = Buffer.concat([Buffer.from('\x89PNG\r\n\x1a\n', 'binary'), Buffer.from('image bytes')]);

async function fixture(run, { reads = [{ data: PNG.toString('base64'), base64Encoded: true, eof: true }], resource = {}, node = { url: 'https://img.example/logo.png?token=SECRET-TOKEN' } } = {}) {
  const root = await mkdtemp(path.join(tmpdir(), 'kernel-save-'));
  const workspace = path.join(root, 'workspace');
  await mkdir(workspace);
  await writeFile(path.join(root, 'outside.txt'), 'outside');
  const calls = [];
  let read = 0;
  const connection = { async send(method, params) {
    calls.push({ method, params });
    if (method === 'Page.getFrameTree') return { frameTree: { frame: { id: 'frame-1', loaderId: 'doc' } } };
    if (method === 'DOM.resolveNode') return { object: { objectId: 'obj' } };
    if (method === 'Runtime.callFunctionOn') return { result: { value: node } };
    if (method === 'Network.loadNetworkResource') return { resource: { success: true, httpStatusCode: 200, stream: 'stream-1', headers: { 'Content-Type': 'image/png' }, ...resource } };
    if (method === 'IO.read') return reads[Math.min(read++, reads.length - 1)];
    return {};
  } };
  const host = { generation: 3, protection: { targets: [] }, browser: { fillTargets: new Map(), resolvePageTarget: async () => ({ connection, sessionId: 'page' }) } };
  const tab = { tab_id: 'tab-1', target_id: 'target', document_id: 'doc' };
  const save = (command = {}) => saveResource(host, tab, { op: 'save', url: 'https://img.example/logo.png?token=SECRET-TOKEN', path: 'images/logo.png', _save_roots: [workspace], ...command });
  const network = () => calls.filter(call => call.method === 'Network.loadNetworkResource');
  try { await run({ root, workspace, calls, host, save, network }); }
  finally { await rm(root, { recursive: true, force: true }); }
}

test('a URL is fetched with the page session and written inside the workspace', () => fixture(async f => {
  const saved = await f.save();
  assert.deepEqual(saved, { generation: 3, tab_id: 'tab-1', document_id: 'doc', path: 'images/logo.png', bytes: PNG.length, mime_type: 'image/png', sha256: createHash('sha256').update(PNG).digest('hex') });
  assert.deepEqual(await readFile(path.join(f.workspace, 'images/logo.png')), PNG);
  assert.equal(f.network().length, 1);
  assert.equal(f.network()[0].params.frameId, 'frame-1');
  assert.equal(f.network()[0].params.options.includeCredentials, true);
  assert(!JSON.stringify(saved).includes(f.root) && !JSON.stringify(saved).includes('SECRET-TOKEN'));
  assert.equal(f.calls.filter(call => call.method === 'IO.close').length, 1, 'the stream handle is released');
  assert.deepEqual(await readdir(path.join(f.workspace, 'images')), ['logo.png'], 'no partial file is left');
}));

test('a node_ref saves the resource its element shows', () => fixture(async f => {
  const saved = await f.save({ url: undefined, node_ref: 'backend:7', path: 'logo.png' });
  assert.equal(saved.path, 'logo.png');
  assert.equal(f.calls.find(call => call.method === 'DOM.resolveNode').params.backendNodeId, 7);
  assert.equal(f.network()[0].params.url, 'https://img.example/logo.png?token=SECRET-TOKEN');
}));

test('a data URL is decoded without a network request, and an overwrite keeps a regular file', () => fixture(async f => {
  const saved = await f.save({ url: `data:image/png;base64,${PNG.toString('base64')}`, path: 'pixel.png' });
  assert.deepEqual([saved.bytes, saved.mime_type], [PNG.length, 'image/png']);
  const text = await f.save({ url: 'data:text/plain,hello%20world', path: 'pixel.png' });
  assert.equal(await readFile(path.join(f.workspace, 'pixel.png'), 'utf8'), 'hello world');
  assert.equal(text.mime_type, 'text/plain');
  assert.equal(f.network().length, 0);
}));

test('outside paths, traversal and symlink escapes use the existing domain refusal before any fetch', () => fixture(async f => {
  await symlink(path.join(f.root), path.join(f.workspace, 'linked'));
  await symlink(path.join(f.root, 'outside.txt'), path.join(f.workspace, 'escape.png'));
  for (const candidate of [path.join(f.root, 'new.png'), '../new.png', 'a/../../new.png', 'linked/new.png', 'escape.png', 'linked/deeper/new.png'])
    await assert.rejects(f.save({ path: candidate }), { code: 'user_domain_not_granted' }, candidate);
  assert.equal(f.network().length, 0);
  assert.equal(await readFile(path.join(f.root, 'outside.txt'), 'utf8'), 'outside');
  await assert.rejects(f.save({ path: '.' }), { code: 'user_domain_not_granted' });
  await assert.rejects(f.save({ _save_roots: undefined }), { code: 'user_domain_not_granted' });
}));

test('a directory, an empty or oversized path and a missing workspace fail plainly', () => fixture(async f => {
  await mkdir(path.join(f.workspace, 'dir'));
  for (const candidate of ['dir', '', 'x'.repeat(5000), 'a\0b']) await assert.rejects(f.save({ path: candidate }), { reason: 'save_path_invalid' }, candidate);
  await assert.rejects(f.save({ _save_roots: [path.join(f.root, 'missing')] }), { reason: 'workspace_unavailable' });
  assert.equal(f.network().length, 0);
}));

test('a resource over the size cap is refused and leaves no file', () => fixture(async f => {
  await assert.rejects(f.save(), { reason: 'save_too_large' });
  assert.deepEqual(await readdir(f.workspace), []);
  assert.equal(f.calls.filter(call => call.method === 'IO.close').length, 1);
}, { reads: Array.from({ length: SAVE_LIMIT / (1024 * 1024) + 1 }, () => ({ data: Buffer.alloc(1024 * 1024).toString('base64'), base64Encoded: true, eof: false })) }));

test('a failed fetch names no URL', () => fixture(async f => {
  await assert.rejects(f.save(), error => error.reason === 'save_fetch_failed' && !error.message.includes('SECRET-TOKEN') && !error.message.includes('img.example'));
  assert.deepEqual(await readdir(f.workspace), []);
}, { resource: { success: false } }));

test('a non-2xx answer is refused and its stream released', () => fixture(async f => {
  await assert.rejects(f.save(), { reason: 'save_fetch_failed' });
  assert.equal(f.calls.filter(call => call.method === 'IO.close').length, 1);
}, { resource: { httpStatusCode: 404 } }));

test('a source must be exactly one of node_ref or url, with a supported scheme', () => fixture(async f => {
  await assert.rejects(f.save({ node_ref: 'backend:7' }), { reason: 'save_source_conflict' });
  await assert.rejects(f.save({ url: undefined }), { reason: 'save_source_conflict' });
  for (const url of ['file:///etc/passwd', 'javascript:alert(1)', 'blob:https://a/b', 'not a url', 'https://user:pass@img.example/a.png'])
    await assert.rejects(f.save({ url }), { reason: 'save_url_unsupported' }, url);
  await assert.rejects(f.save({ url: undefined, node_ref: 'frame:a:backend:7' }), { reason: 'node_ref_invalid' });
  assert.equal(f.network().length, 0);
}));

test('a canvas saves as PNG, unless it is tainted or a protected field is on the page', () => fixture(async f => {
  const saved = await f.save({ url: undefined, node_ref: 'backend:9', path: 'canvas.png' });
  assert.deepEqual([saved.mime_type, saved.bytes], ['image/png', 3]);
  f.host.browser.fillTargets.set('field', {});
  await assert.rejects(f.save({ url: undefined, node_ref: 'backend:9', path: 'again.png' }), { reason: 'save_canvas_unavailable' });
  f.host.browser.fillTargets.clear();
  f.host.protection.targets.push({});
  await assert.rejects(f.save({ url: undefined, node_ref: 'backend:9', path: 'again.png' }), { reason: 'save_canvas_unavailable' });
}, { node: { canvas: true, url: 'data:image/png;base64,AAEC' } }));

test('a tainted canvas and a node without a resource are refused', async () => {
  await fixture(async f => { await assert.rejects(f.save({ url: undefined, node_ref: 'backend:9' }), { reason: 'save_canvas_unavailable' }); }, { node: { canvas: true, url: null } });
  await fixture(async f => { await assert.rejects(f.save({ url: undefined, node_ref: 'backend:9' }), { reason: 'save_url_unsupported' }); }, { node: null });
});

test('a missing or odd content type is recorded as a plain media type', async () => {
  await fixture(async f => { assert.equal((await f.save()).mime_type, 'image/jpeg'); }, { resource: { headers: { 'content-type': 'IMAGE/JPEG; charset=binary' } } });
  await fixture(async f => { assert.equal((await f.save()).mime_type, 'application/octet-stream'); }, { resource: { headers: {} } });
});
