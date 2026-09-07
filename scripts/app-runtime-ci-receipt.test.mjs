import assert from 'node:assert/strict';
import { mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fingerprint, NATIVE_BUILD_INPUTS, readReceipt, receiptKey, RECEIPT_MAX_AGE_MS, RECEIPT_MAX_BYTES, validReceipt, verifyReceipt } from './app-runtime-ci-receipt.mjs';

const repository = 'charioxai/chariox';
const now = 1_800_000_000_000;
const entries = NATIVE_BUILD_INPUTS.map((path, index) => ({ path, type: 'blob', mode: '100644', sha: (index + 1).toString(16).padStart(40, '0') }));
const inputHash = fingerprint(entries);

function fixture() {
  const receipt = {
    schema: 'chariox.unsigned-native-build-receipt.v1', evidence: 'unsigned-compilation-only',
    target: 'linux-x64', repository, input_hash: inputHash, head_sha: 'b'.repeat(40),
    completed_at_ms: now - 1000, run_id: '123', run_attempt: '1', artifact_id: '456',
    artifact_url: `https://github.com/${repository}/actions/runs/123/artifacts/456`,
  };
  const run = { id: 123, run_attempt: 1, status: 'completed', conclusion: 'success',
    path: '.github/workflows/app-runtime-native.yml', repository: { full_name: repository }, head_sha: receipt.head_sha };
  const artifact = { id: 456, expired: false, workflow_run: { id: run.id, head_sha: receipt.head_sha },
    name: `UNSIGNED-NONRELEASE-linux-x64-${receipt.head_sha}`, created_at: new Date(now - 2000).toISOString(), expires_at: new Date(now + 6 * 86_400_000).toISOString() };
  const commit = { sha: receipt.head_sha, tree: { sha: 'c'.repeat(40) } };
  const tree = { truncated: false, tree: structuredClone(entries) };
  const paths = [
    `/repos/${repository}/actions/runs/123`, `/repos/${repository}/actions/artifacts/456`,
    `/repos/${repository}/git/commits/${receipt.head_sha}`, `/repos/${repository}/git/trees/${commit.tree.sha}?recursive=1`,
  ];
  const values = [run, artifact, commit, tree];
  const calls = [];
  return { receipt, run, artifact, commit, tree, calls, paths,
    get: async path => { calls.push(path); assert.ok(paths.includes(path)); return values[paths.indexOf(path)]; } };
}

test('fingerprint covers every native build input and ignores unrelated PR changes', () => {
  assert.equal(fingerprint([...entries].reverse()), inputHash);
  assert.equal(fingerprint([...entries, { path: 'apps/kernel/unrelated.rs', type: 'blob', mode: '100644', sha: 'f'.repeat(40) }]), inputHash);
  for (const path of NATIVE_BUILD_INPUTS) {
    const changed = structuredClone(entries);
    changed.find(entry => entry.path === path).sha = 'f'.repeat(40);
    assert.notEqual(fingerprint(changed), inputHash, `${path} must invalidate the receipt`);
  }
  const changedMode = structuredClone(entries); changedMode[0].mode = '100755';
  assert.notEqual(fingerprint(changedMode), inputHash);
});

test('incomplete, duplicate and symlink input trees cannot identify a build', () => {
  assert.throws(() => fingerprint(entries.slice(1)));
  assert.throws(() => fingerprint([...entries.slice(1), entries[1]]));
  for (const patch of [{ mode: '120000' }, { sha: 'not-a-commit' }, { type: 'tree' }]) {
    const changed = structuredClone(entries); Object.assign(changed[0], patch);
    assert.throws(() => fingerprint(changed));
  }
});

test('receipt refresh is bounded below artifact retention and cannot claim release trust', () => {
  const { receipt } = fixture();
  assert.ok(RECEIPT_MAX_AGE_MS < 7 * 86_400_000);
  assert.ok(validReceipt(receipt, repository, inputHash, now));
  assert.equal(receiptKey(inputHash, 1), receiptKey(inputHash, RECEIPT_MAX_AGE_MS - 1));
  assert.notEqual(receiptKey(inputHash, 1), receiptKey(inputHash, RECEIPT_MAX_AGE_MS));
  for (const patch of [
    { completed_at_ms: now - RECEIPT_MAX_AGE_MS }, { completed_at_ms: now + 1 },
    { evidence: 'signed-release' }, { input_hash: 'f'.repeat(64) },
    { repository: 'another/repo' }, { artifact_url: 'https://example.test/forged' },
    { run_id: '../secrets' }, { head_sha: 'untrusted-ref' },
  ]) assert.equal(validReceipt({ ...receipt, ...patch }, repository, inputHash, now), false);
  assert.throws(() => receiptKey('bad', now));
});

test('reuse checks actual original success, retained artifact and committed native inputs', async () => {
  const f = fixture();
  assert.equal(await verifyReceipt(f.receipt, repository, inputHash, now, f.get), true);
  assert.deepEqual(f.calls, f.paths);
  // A later unrelated commit has the same input fingerprint. A pending native
  // edit has a different fingerprint even when its final push changed no inputs.
  const changed = structuredClone(entries); changed[0].sha = 'f'.repeat(40);
  assert.equal(await verifyReceipt(f.receipt, repository, fingerprint(changed), now, f.get), false);
});

test('forged PR receipts and unavailable or expired original evidence force a new build', async () => {
  const mutations = [
    f => { f.run.conclusion = 'failure'; },
    f => { f.run.status = 'in_progress'; },
    f => { f.run.run_attempt = 2; },
    f => { f.run.path = '.github/workflows/untrusted.yml'; },
    f => { f.run.repository.full_name = 'another/repo'; },
    f => { f.run.head_sha = 'd'.repeat(40); },
    f => { f.artifact.expired = true; },
    f => { f.artifact.workflow_run.id = 789; },
    f => { f.artifact.expires_at = new Date(now).toISOString(); },
    f => { f.artifact.created_at = new Date(now - RECEIPT_MAX_AGE_MS).toISOString(); },
    f => { f.artifact.name = 'forged-artifact'; },
    f => { f.commit.sha = 'd'.repeat(40); },
    f => { f.tree.truncated = true; },
    // Merely writing the desired hash into a cache receipt cannot hide that the
    // original successful run used another workflow or source implementation.
    f => { f.tree.tree.find(entry => entry.path.includes('/workflows/')).sha = 'f'.repeat(40); },
  ];
  for (const mutate of mutations) {
    const f = fixture(); mutate(f);
    assert.equal(await verifyReceipt(f.receipt, repository, inputHash, now, f.get), false);
  }
  const f = fixture();
  await assert.rejects(verifyReceipt(f.receipt, repository, inputHash, now, async () => { throw new Error('network unavailable'); }));
});

test('restored receipt bytes are bounded and symlinks are never followed', async () => {
  const root = await mkdtemp(join(tmpdir(), 'chariox-native-receipt-'));
  try {
    const file = join(root, 'receipt.json');
    const { receipt } = fixture();
    await writeFile(file, JSON.stringify(receipt));
    assert.deepEqual(await readReceipt(file), receipt);
    const alias = join(root, 'alias'); await symlink(file, alias);
    await assert.rejects(readReceipt(alias));
    await writeFile(file, ' '.repeat(RECEIPT_MAX_BYTES + 1));
    await assert.rejects(readReceipt(file), /invalid receipt|exceeded/);
  } finally { await rm(root, { recursive: true, force: true }); }
});
