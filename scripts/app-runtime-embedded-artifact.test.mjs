import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { artifactRedirect, MAX_ARCHIVE, NATIVE_HEAD, NATIVE_RUN, saveArchive, selectArtifact } from './app-runtime-embedded-artifact.mjs';

const now = Date.parse('2026-09-08T01:00:00Z');
function metadata() {
  const run = { id: Number(NATIVE_RUN), repository: { full_name: 'charioxai/chariox' },
    head_repository: { full_name: 'charioxai/chariox' }, path: '.github/workflows/app-runtime-native.yml',
    head_sha: NATIVE_HEAD, event: 'pull_request', status: 'completed', conclusion: 'success', run_attempt: 1,
    run_started_at: '2026-09-08T00:00:00Z' };
  const artifact = { id: 123, name: `UNSIGNED-NONRELEASE-linux-x64-${NATIVE_HEAD}`, expired: false,
    workflow_run: { id: Number(NATIVE_RUN), head_sha: NATIVE_HEAD }, size_in_bytes: 100, digest: `sha256:${'a'.repeat(64)}`,
    created_at: '2026-09-08T00:30:00Z', expires_at: '2026-09-15T00:30:00Z' };
  return { run, artifact, get: async path => path.includes('/artifacts?') ? { total_count: 1, artifacts: [artifact] } : run };
}

test('native execution accepts only the reviewed successful run and retained exact artifact', async () => {
  const good = metadata();
  const selected = await selectArtifact(NATIVE_RUN, good.get, now);
  assert.equal(selected.ready, true); assert.equal(selected.artifactId, '123');
  for (const change of [
    f => { f.run.repository.full_name = 'attacker/chariox'; },
    f => { f.run.head_repository.full_name = 'attacker/chariox'; },
    f => { f.run.path = '.github/workflows/other.yml'; },
    f => { f.run.head_sha = 'b'.repeat(40); },
    f => { f.run.run_attempt = 2; },
    f => { f.run.conclusion = 'failure'; },
    f => { f.artifact.expired = true; },
    f => { f.artifact.workflow_run.id++; },
    f => { f.artifact.size_in_bytes = MAX_ARCHIVE + 1; },
    f => { f.artifact.digest = 'untrusted'; },
    f => { f.artifact.name = 'other'; },
    f => { f.artifact.expires_at = '2026-09-07T00:00:00Z'; },
  ]) {
    const f = metadata(); change(f); await assert.rejects(selectArtifact(NATIVE_RUN, f.get, now));
  }
  await assert.rejects(selectArtifact('123', good.get, now));
});

test('pending native compilation is explicit and does not request or select an artifact', async () => {
  const f = metadata(); f.run.status = 'in_progress'; let calls = 0;
  const result = await selectArtifact(NATIVE_RUN, async path => { calls++; return f.get(path); }, now);
  assert.equal(calls, 1); assert.deepEqual(result, { ready: false, runId: NATIVE_RUN, sourceCommit: NATIVE_HEAD });
});

test('artifact redirects cannot expose a GitHub credential to caller-selected hosts', () => {
  assert.equal(artifactRedirect('https://results.blob.core.windows.net/path?secret=fixture').hostname, 'results.blob.core.windows.net');
  for (const location of ['http://results.blob.core.windows.net/path', 'https://github.com/path',
    'https://results.blob.core.windows.net.attacker.invalid/path', 'https://user:pass@results.blob.core.windows.net/path',
    'https://results.blob.core.windows.net:444/path']) assert.throws(() => artifactRedirect(location));
});

test('streamed archive bytes must exactly match the observed GitHub size and digest', async t => {
  const root = await mkdtemp(join(tmpdir(), 'chariox-embedded-artifact-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const bytes = Buffer.from('small non-executable archive fixture');
  const evidence = { size: bytes.length, digest: `sha256:${createHash('sha256').update(bytes).digest('hex')}` };
  const path = join(root, 'download');
  await saveArchive(new Response(bytes), path, evidence);
  assert.deepEqual(await readFile(path), bytes);
  await assert.rejects(saveArchive(new Response(bytes), path, evidence), { code: 'EEXIST' });
  for (const [index, changed] of [{ ...evidence, size: bytes.length - 1 }, { ...evidence, size: bytes.length + 1 },
    { ...evidence, digest: `sha256:${'0'.repeat(64)}` }].entries()) {
    const bad = join(root, `bad-${index}`);
    await assert.rejects(saveArchive(new Response(bytes), bad, changed));
    await assert.rejects(readFile(bad), { code: 'ENOENT' });
  }
});
