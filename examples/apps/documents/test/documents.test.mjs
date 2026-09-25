// Exercises the Documents backend against an in-memory stand-in for the
// kernel's App state (compare-and-set) and a temporary private data root.
import assert from 'node:assert/strict';
import { access, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
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
  let beforeCommit = async () => {};
  const chariox = {
    AppError,
    paths: { package: '/package', data, temporary: '/tmp' },
    tools: { register: (name, handler) => tools.set(name, handler) },
    files: {
      // Like the kernel's private-data replace: parents must already exist.
      async atomicReplace(path, contents) {
        await access(dirname(join(data, path))).catch(() => {
          throw new AppError('APP_FILE_UNAVAILABLE', 'Private file operation did not complete');
        });
        await writeFile(join(data, path), contents);
        return { bytesWritten: Buffer.byteLength(contents) };
      },
    },
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
  return { call, data, setBeforeCommit: (hook) => { beforeCommit = hook; }, cleanup: () => rm(data, { recursive: true, force: true }) };
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

test('a rename is a revision too, so a stale editor cannot revert it', async () => {
  const kernel = await fakeKernel();
  const doc = await kernel.call('create_document', { title: 'Draft', content: 'text' });
  const renamed = await kernel.call('update_document', { id: doc.id, expected_revision: 1, title: 'Final' });
  assert.equal(renamed.revision, 2);
  await assert.rejects(kernel.call('update_document', { id: doc.id, expected_revision: 1, title: 'Draft', content: 'text' }), { code: 'CONFLICT' });
  const current = await kernel.call('read_document', { id: doc.id });
  assert.deepEqual([current.title, current.content], ['Final', 'text']);
  await kernel.cleanup();
});

test('reading content pruned by a concurrent save is a conflict, not a crash', async () => {
  const kernel = await fakeKernel();
  const doc = await kernel.call('create_document', { title: 'Racy', content: 'v1' });
  await rm(join(kernel.data, 'documents', doc.id), { recursive: true });
  await assert.rejects(kernel.call('read_document', { id: doc.id }), (error) => error instanceof AppError && error.code === 'CONFLICT');
  await kernel.cleanup();
});

test('the index stays under the kernel value limit and a refused create leaves no file', async () => {
  const kernel = await fakeKernel();
  const title = '\u{1F4DD}'.repeat(200);
  let created = 0;
  let refused;
  while (!refused) {
    try {
      await kernel.call('create_document', { title, folder: title, content: 'x' });
      created += 1;
    } catch (error) {
      refused = error;
    }
  }
  assert.equal(refused.code, 'LIMIT_EXCEEDED');
  assert.match(refused.message, /index is full/);
  assert.ok(created > 50, `${created} worst-case documents fit`);
  assert.equal((await readdir(join(kernel.data, 'documents'))).length, created);
  await kernel.cleanup();
});

test('Markdown preview escapes document text and emits only its own tags', () => {
  const html = markdownToHtml('# Title <script>x</script>\n- **bold** `a<b>`\n\n```\n<img src=x onerror=1>\n```');
  assert.doesNotMatch(html, /<script|<img/);
  assert.match(html, /<h1>Title &lt;script&gt;x&lt;\/script&gt;<\/h1>/);
  assert.match(html, /<ul>\n<li><strong>bold<\/strong> <code>a&lt;b&gt;<\/code><\/li>\n<\/ul>/);
  assert.match(html, /<pre><code>&lt;img src=x onerror=1&gt;<\/code><\/pre>/);
});
