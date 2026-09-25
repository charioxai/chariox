// Documents reference App. Markdown and HTML documents live as private files
// in the App's data root; a small index in kernel-managed state names them.
// Every edit states the revision it was made from, so concurrent human and
// agent edits never silently overwrite each other. Import/export arrives with
// the kernel's user-selected file grants (host.pickFile, files.import/export).
import { randomUUID } from 'node:crypto';
import { mkdir, readFile, readdir, rm } from 'node:fs/promises';
import { join } from 'node:path';

const INDEX = 'index';
const SCHEMA = 1;
// The index is one state value (at most 256 KiB); content lives in files.
const MAX_DOCUMENTS = 500;
const MAX_CONTENT = 512 * 1024;
const KEEP_VERSIONS = 10;
const KINDS = { markdown: 'md', html: 'html' };

export default function register(chariox) {
  const fail = (code, message) => new chariox.AppError(code, message);
  // Each write attempt gets its own file: a save that loses the index
  // compare-and-set can never overwrite the winner's committed content.
  const fileOf = (doc, revision = doc.revision) => doc.versions.find(version => version.revision === revision)?.file;
  const contentPath = (doc, file) => `documents/${doc.id}/${file}`;

  async function load() {
    const record = await chariox.state.get(INDEX);
    return { docs: record?.value?.docs ?? [], version: record?.version ?? null };
  }

  // Compare-and-set on the index; the content file for a new revision is
  // written first, so a committed index entry always names an existing file.
  async function change(mutate) {
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const { docs, version } = await load();
      const next = docs.map(doc => ({ ...doc }));
      const result = await mutate(next);
      try {
        await chariox.state.transaction({
          schemaVersion: SCHEMA,
          checks: [{ key: INDEX, version }],
          writes: [{ key: INDEX, value: { docs: next } }],
        });
        return result;
      } catch (error) {
        if (error?.code !== 'CONFLICT') throw error;
      }
    }
    throw fail('CONFLICT', 'Documents changed too often; try again');
  }

  function find(docs, id) {
    const doc = docs.find(item => item.id === id);
    if (!doc) throw fail('NOT_FOUND', `No document with id ${id}`);
    return doc;
  }

  async function read(doc, revision = doc.revision) {
    const file = fileOf(doc, revision);
    if (!file) throw fail('NOT_FOUND', `No revision ${revision}`);
    try {
      return await readFile(join(chariox.paths.data, contentPath(doc, file)), 'utf8');
    } catch (error) {
      // A concurrent save or delete pruned it after this index was read.
      if (error?.code === 'ENOENT') throw fail('CONFLICT', 'Document changed; reload and try again');
      throw error;
    }
  }

  // Writes a new revision's content and records it (bounded history).
  async function write(doc, content) {
    if (Buffer.byteLength(content) > MAX_CONTENT) throw fail('LIMIT_EXCEEDED', 'Document is larger than 512 KiB');
    const file = `${doc.revision}-${randomUUID().slice(0, 8)}.${KINDS[doc.kind]}`;
    // Atomic replace does not create parents; the data root is the App's own.
    await mkdir(join(chariox.paths.data, 'documents', doc.id), { recursive: true });
    await chariox.files.atomicReplace(contentPath(doc, file), content);
    doc.versions = [...doc.versions, { revision: doc.revision, file }].slice(-KEEP_VERSIONS);
  }

  // After the index commits, remove files it no longer names (older versions,
  // deleted documents). Best effort: a leftover file is never referenced.
  // Only revisions below the oldest kept one are removed, so a concurrent
  // save's uncommitted file (a newer revision) is never touched.
  async function prune(id, keep) {
    const folder = join(chariox.paths.data, 'documents', id);
    if (!keep) return rm(folder, { recursive: true, force: true });
    const oldest = Math.min(...keep.map(name => Number.parseInt(name, 10)));
    const names = await readdir(folder).catch(() => []);
    await Promise.all(names
      .filter(name => !keep.includes(name) && Number.parseInt(name, 10) < oldest)
      .map(name => rm(join(folder, name), { force: true })));
  }

  const summary = ({ id, title, folder, kind, revision, updated_at_ms, versions }) =>
    ({ id, title, folder, kind, revision, updated_at_ms, versions: versions.map(version => version.revision) });

  chariox.tools.register('list_documents', async ({ folder } = {}) => {
    const { docs } = await load();
    return { documents: docs.filter(doc => folder === undefined || doc.folder === folder).map(summary) };
  });

  chariox.tools.register('read_document', async ({ id, revision }) => {
    const { docs } = await load();
    const doc = find(docs, id);
    return { ...summary(doc), content: await read(doc, revision ?? doc.revision) };
  });

  chariox.tools.register('create_document', ({ title, kind = 'markdown', folder = '', content = '' }) => {
    // One id for every retry, so a lost compare-and-set leaves no stray folder.
    const id = randomUUID().slice(0, 8);
    return change(async docs => {
      if (docs.length >= MAX_DOCUMENTS) throw fail('LIMIT_EXCEEDED', `At most ${MAX_DOCUMENTS} documents`);
      const doc = { id, title, folder, kind, revision: 1, versions: [], updated_at_ms: Date.now() };
      await write(doc, content);
      docs.push(doc);
      return summary(doc);
    });
  });

  // `expected_revision` is the revision the editor started from. A mismatch
  // is a conflict the caller resolves by re-reading, never a silent merge.
  chariox.tools.register('update_document', async ({ id, expected_revision: expected, content, title, folder }) => {
    const updated = await change(async docs => {
      const doc = find(docs, id);
      if (doc.revision !== expected) {
        throw fail('CONFLICT', `Document changed (now revision ${doc.revision}); reload before saving`);
      }
      // Every change, including a rename or move, is a new revision, so a
      // stale editor can never silently revert another person's or agent's edit.
      if (title !== undefined) doc.title = title;
      if (folder !== undefined) doc.folder = folder;
      const previous = fileOf(doc);
      doc.revision += 1;
      if (content !== undefined) await write(doc, content);
      else doc.versions = [...doc.versions, { revision: doc.revision, file: previous }].slice(-KEEP_VERSIONS);
      doc.updated_at_ms = Date.now();
      return { ...summary(doc), kept: doc.versions.map(version => version.file) };
    });
    await prune(id, updated.kept);
    delete updated.kept;
    return updated;
  });

  chariox.tools.register('restore_version', async ({ id, revision, expected_revision: expected }) => {
    const restored = await change(async docs => {
      const doc = find(docs, id);
      if (doc.revision !== expected) throw fail('CONFLICT', `Document changed (now revision ${doc.revision})`);
      const content = await read(doc, revision);
      doc.revision += 1;
      await write(doc, content);
      doc.updated_at_ms = Date.now();
      return { ...summary(doc), kept: doc.versions.map(version => version.file) };
    });
    await prune(id, restored.kept);
    delete restored.kept;
    return restored;
  });

  chariox.tools.register('delete_document', async ({ id }) => {
    const deleted = await change(async docs => {
      const index = docs.findIndex(doc => doc.id === id);
      if (index < 0) throw fail('NOT_FOUND', `No document with id ${id}`);
      docs.splice(index, 1);
      return { deleted: id };
    });
    await prune(id, null);
    return deleted;
  });
}
