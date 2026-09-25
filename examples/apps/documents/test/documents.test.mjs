// Exercises the Documents backend against an in-memory stand-in for the
// kernel's App state (compare-and-set) and a temporary private data root.
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { AppError } from '../../../../packages/app-sdk/src/errors.js';
import register from '../bundle/runtime/main.mjs';
import { markdownToHtml } from '../bundle/ui/preview.js';

async function fakeKernel() {
  const data = await mkdtemp(join(tmpdir(), 'chariox-documents-'));
  const state = new Map();
  const tools = new Map();
  const exports = [];
  let beforeCommit = async () => {};
  const chariox = {
    AppError,
    paths: { package: '/package', data, temporary: '/tmp' },
    tools: { register: (name, handler) => tools.set(name, handler) },
    files: {
      async atomicReplace(path, contents) {
        await mkdir(dirname(join(data, path)), { recursive: true });
        await writeFile(join(data, path), contents);
        return { bytesWritten: Buffer.byteLength(contents) };
      },
      async import(grantId, destination) {
        await mkdir(dirname(join(data, destination)), { recursive: true });
        await writeFile(join(data, destination), '<h1>Imported</h1><p>From a grant</p>');
        return { bytesWritten: 1 };
      },
      async export(path) { exports.push(path); return { operationId: 'export-1' }; },
    },
    host: { pickFile: async () => ({ grantIds: ['grant-1'] }) },
    state: {
      async get(key) { return state.get(key) ?? null; },
      async transaction({ checks, writes }) {
        await beforeCommit();
        for (const check of checks) {
          if ((state.get(check.key)?.version ?? null) !== check.version) {
            throw Object.assign(new Error('conflict'), { code: 'CONFLICT' });
          }
        }
        for (const write of writes) state.set(write.key, { value: write.value, version: (state.get(write.key)?.version ?? 0) + 1 });
        return { revision: 1, receipts: [] };
      },
    },
  };
  register(chariox);
  const call = (name, input = {}) => tools.get(name)(input);
  return { call, data, exports, setBeforeCommit: (hook) => { beforeCommit = hook; }, cleanup: () => rm(data, { recursive: true, force: true }) };
}

test('documents are created, edited with revision checks, restored and deleted', async () => {
  const kernel = await fakeKernel();
  const created = await kernel.call('create_document', { title: 'Plan', content: '# Plan', folder: 'work' });
  assert.deepEqual([created.revision, created.versions, created.folder], [1, [1], 'work']);
  const saved = await kernel.call('update_document', { id: created.id, expected_revision: 1, content: '# Plan v2' });
  assert.equal(saved.revision, 2);
  // A stale editor (still at revision 1) is refused, not merged or overwritten.
  await assert.rejects(kernel.call('update_document', { id: created.id, expected_revision: 1, content: 'stale' }),
    (error) => error instanceof AppError && error.code === 'CONFLICT' && /revision 2/.test(error.message));
  assert.equal((await kernel.call('read_document', { id: created.id })).content, '# Plan v2');
  assert.equal((await kernel.call('read_document', { id: created.id, revision: 1 })).content, '# Plan');
  const restored = await kernel.call('restore_version', { id: created.id, revision: 1, expected_revision: 2 });
  assert.equal((await kernel.call('read_document', { id: restored.id })).content, '# Plan');
  assert.deepEqual((await kernel.call('list_documents', { folder: 'work' })).documents.map((doc) => doc.id), [created.id]);
  await kernel.call('delete_document', { id: created.id });
  assert.deepEqual((await kernel.call('list_documents')).documents, []);
  assert.deepEqual(await readdir(join(kernel.data, 'documents')), [], 'deleted content is removed');
  await kernel.cleanup();
});

test('history is bounded and superseded files are pruned', async () => {
  const kernel = await fakeKernel();
  const doc = await kernel.call('create_document', { title: 'Log', content: 'v1' });
  let revision = 1;
  for (let index = 2; index <= 14; index += 1) {
    revision = (await kernel.call('update_document', { id: doc.id, expected_revision: revision, content: `v${index}` })).revision;
  }
  const current = await kernel.call('read_document', { id: doc.id });
  assert.equal(current.versions.length, 10);
  assert.deepEqual(current.versions, [5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
  assert.equal((await readdir(join(kernel.data, 'documents', doc.id))).length, 10);
  await kernel.cleanup();
});

test('a save that loses a concurrent race never overwrites the winner', async () => {
  const kernel = await fakeKernel();
  const doc = await kernel.call('create_document', { title: 'Shared', content: 'base' });
  // Both editors start from revision 1; the agent's save commits while the
  // person's save is between writing its file and committing the index.
  let raced = false;
  kernel.setBeforeCommit(async () => {
    if (raced) return;
    raced = true;
    await kernel.call('update_document', { id: doc.id, expected_revision: 1, content: 'agent edit' });
  });
  await assert.rejects(kernel.call('update_document', { id: doc.id, expected_revision: 1, content: 'person edit' }), { code: 'CONFLICT' });
  assert.equal((await kernel.call('read_document', { id: doc.id })).content, 'agent edit');
  await kernel.cleanup();
});

test('import and export go through kernel file grants', async () => {
  const kernel = await fakeKernel();
  const imported = await kernel.call('import_document', { title: 'From disk' });
  assert.equal(imported.kind, 'html');
  assert.match((await kernel.call('read_document', { id: imported.id })).content, /Imported/);
  assert.deepEqual(await readdir(join(kernel.data, 'imports')), [], 'the staged import is removed');
  await kernel.call('export_document', { id: imported.id });
  assert.match(kernel.exports[0], new RegExp(`^documents/${imported.id}/1-[0-9a-f]{8}\\.html$`));
  await kernel.cleanup();
});

test('Markdown preview escapes document text and emits only its own tags', () => {
  const html = markdownToHtml('# Title <script>x</script>\n- **bold** `a<b>`\n\n```\n<img src=x onerror=1>\n```');
  assert.doesNotMatch(html, /<script|<img/);
  assert.match(html, /<h1>Title &lt;script&gt;x&lt;\/script&gt;<\/h1>/);
  assert.match(html, /<ul>\n<li><strong>bold<\/strong> <code>a&lt;b&gt;<\/code><\/li>\n<\/ul>/);
  assert.match(html, /<pre><code>&lt;img src=x onerror=1&gt;<\/code><\/pre>/);
});
