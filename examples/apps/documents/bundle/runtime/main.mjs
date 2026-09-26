// Documents reference App. Markdown and HTML documents live as private files
// in the App's data root; a small index in kernel-managed state names them.
// Every edit states the revision it was made from, so concurrent human and
// agent edits never silently overwrite each other. Import uses the kernel's
// user-selected file grants: the owner chooses files in a trusted prompt, and
// the App receives private copies, never host paths.
import { randomUUID } from 'node:crypto';
import { mkdir, readFile, readdir, rm } from 'node:fs/promises';
import { join } from 'node:path';

const INDEX = 'index';
// The package's data schema version: 0 until it declares migrations.
const SCHEMA = 0;
// The index is one state value, and the kernel bounds both its size (256 KiB)
// and its JSON node count (16384). An entry is ~0.6 KiB typically and ~3 KiB
// at worst (long titles); `fits` checks the size before any content file is
// written. With full history an entry is ~38 nodes, so ~431 documents would
// exceed the node budget; MAX_DOCUMENTS keeps the count well under it.
const MAX_INDEX_BYTES = 256 * 1024;
const MAX_DOCUMENTS = 300;
const MAX_CONTENT = 512 * 1024;
const KEEP_VERSIONS = 10;
const KINDS = { markdown: 'md', html: 'html' };
const IMPORTS = { '.md': 'markdown', '.markdown': 'markdown', '.txt': 'markdown', '.html': 'html', '.htm': 'html' };

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

  function fits(docs) {
    if (Buffer.byteLength(JSON.stringify({ docs })) > MAX_INDEX_BYTES) {
      throw fail('LIMIT_EXCEEDED', 'The document index is full; delete documents or shorten titles and folders');
    }
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

  // Records a new revision (bounded history) and writes its content, only
  // after the resulting index is known to fit.
  async function write(docs, doc, content) {
    if (Buffer.byteLength(content) > MAX_CONTENT) throw fail('LIMIT_EXCEEDED', 'Document is larger than 512 KiB');
    const file = `${doc.revision}-${randomUUID().slice(0, 8)}.${KINDS[doc.kind]}`;
    doc.versions = [...doc.versions, { revision: doc.revision, file }].slice(-KEEP_VERSIONS);
    fits(docs);
    // Atomic replace does not create parents; the data root is the App's own.
    await mkdir(join(chariox.paths.data, 'documents', doc.id), { recursive: true });
    await chariox.files.atomicReplace(contentPath(doc, file), content);
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

  function create({ title, kind = 'markdown', folder = '', content = '' }) {
    // One id for every retry, so a lost compare-and-set leaves no stray folder.
    const id = randomUUID().slice(0, 8);
    return change(async docs => {
      if (docs.length >= MAX_DOCUMENTS) throw fail('LIMIT_EXCEEDED', `At most ${MAX_DOCUMENTS} documents`);
      const doc = { id, title, folder, kind, revision: 1, versions: [], updated_at_ms: Date.now() };
      docs.push(doc);
      await write(docs, doc, content);
      return summary(doc);
    });
  }
  chariox.tools.register('create_document', create);

  // The owner may take minutes to choose, so the tool returns at once and the
  // import finishes in the background; the list shows the new documents.
  async function importGranted(grantIds, folder) {
    await mkdir(join(chariox.paths.data, 'imports'), { recursive: true });
    for (const grantId of grantIds) {
      const staged = `imports/${grantId}`;
      try {
        const { name } = await chariox.files.import(grantId, staged);
        const extension = name.slice(name.lastIndexOf('.')).toLowerCase();
        const content = await readFile(join(chariox.paths.data, staged), 'utf8');
        const title = name.slice(0, name.length - extension.length).slice(0, 200) || name.slice(0, 200);
        await create({ title, kind: IMPORTS[extension] ?? 'markdown', folder, content });
      } catch (error) {
        await chariox.log.write('warn', 'A document import failed', { code: String(error?.code ?? 'ERROR') }).catch(() => {});
      } finally {
        await rm(join(chariox.paths.data, staged), { force: true });
      }
    }
  }

  chariox.tools.register('import_documents', ({ folder = '' } = {}) => {
    chariox.host.pickFile({ multiple: true, accept: Object.keys(IMPORTS) })
      .then(({ grantIds }) => importGranted(grantIds, folder))
      .catch(error => chariox.log.write('info', 'No documents imported', { code: String(error?.code ?? 'ERROR') }).catch(() => {}));
    return { requested: true };
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
      if (content !== undefined) await write(docs, doc, content);
      else {
        doc.versions = [...doc.versions, { revision: doc.revision, file: previous }].slice(-KEEP_VERSIONS);
        fits(docs);
      }
      doc.updated_at_ms = Date.now();
      return { ...summary(doc), kept: doc.versions.map(version => version.file) };
    });
    await prune(id, updated.kept);
    delete updated.kept;
    return updated;
  });

  // Restores content only; a rename or move revision restores nothing visible.
  chariox.tools.register('restore_version', async ({ id, revision, expected_revision: expected }) => {
    const restored = await change(async docs => {
      const doc = find(docs, id);
      if (doc.revision !== expected) throw fail('CONFLICT', `Document changed (now revision ${doc.revision})`);
      const content = await read(doc, revision);
      doc.revision += 1;
      await write(docs, doc, content);
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
