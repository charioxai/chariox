// A cacheable receipt of unsigned compilation, never runtime release authority.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { constants } from 'node:fs';
import { appendFile, mkdir, open, realpath, rename, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const NATIVE_BUILD_INPUTS = [
  '.github/workflows/app-runtime-native.yml',
  'apps/app-worker/runtime.lock.json',
  'apps/app-worker/src/runtime.h',
  'apps/app-worker/src/node_runtime.cc',
  'scripts/build-app-runtime.mjs',
  'scripts/build-app-runtime.test.mjs',
  'scripts/app-runtime-ci-resources.mjs',
  'scripts/app-runtime-ci-resources.test.mjs',
  'scripts/app-runtime-ci-receipt.mjs',
  'scripts/app-runtime-ci-receipt.test.mjs',
  'scripts/run-app-runtime-native-ci.sh',
].sort();
export const RECEIPT_MAX_AGE_MS = 5 * 24 * 60 * 60 * 1000;
export const RECEIPT_MAX_BYTES = 4096;
const WORKFLOW = '.github/workflows/app-runtime-native.yml';
const SHA = /^[a-f0-9]{40}$/;
const REPO = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const ID = /^[1-9][0-9]{0,19}$/;
const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export function fingerprint(entries) {
  const selected = entries.filter(entry => NATIVE_BUILD_INPUTS.includes(entry.path));
  selected.sort((a, b) => a.path.localeCompare(b.path, 'en'));
  if (selected.length !== NATIVE_BUILD_INPUTS.length
      || new Set(selected.map(entry => entry.path)).size !== NATIVE_BUILD_INPUTS.length
      || selected.some(entry => entry.type !== 'blob' || !['100644', '100755'].includes(entry.mode) || !SHA.test(entry.sha))) {
    throw new Error('missing, duplicate or non-regular native build input');
  }
  return createHash('sha256').update(JSON.stringify(selected.map(({ path, mode, sha }) => ({ path, mode, sha })))).digest('hex');
}

export function currentInputs(repository = REPOSITORY) {
  const git = args => execFileSync('git', ['-C', repository, ...args], { encoding: 'utf8', maxBuffer: 256 * 1024, timeout: 10_000 });
  if (git(['status', '--porcelain', '--', ...NATIVE_BUILD_INPUTS])) throw new Error('native build inputs must be committed');
  const entries = git(['ls-tree', '-r', '-z', 'HEAD', '--', ...NATIVE_BUILD_INPUTS]).split('\0').filter(Boolean).map(line => {
    const match = /^(\d{6}) (blob) ([a-f0-9]{40})\t(.+)$/.exec(line);
    if (!match) throw new Error('invalid native input tree');
    return { mode: match[1], type: match[2], sha: match[3], path: match[4] };
  });
  return { input_hash: fingerprint(entries), head_sha: git(['rev-parse', 'HEAD']).trim() };
}

export function receiptKey(inputHash, now) {
  if (!/^[a-f0-9]{64}$/.test(inputHash) || !Number.isSafeInteger(now) || now < 0) throw new Error('invalid receipt key input');
  // Immutable cache entries refresh before their seven-day artifacts expire.
  return `app-runtime-linux-x64-v1-${inputHash}-${Math.floor(now / RECEIPT_MAX_AGE_MS)}`;
}

export function validReceipt(receipt, repository, inputHash, now) {
  return receipt?.schema === 'chariox.unsigned-native-build-receipt.v1'
    && receipt.target === 'linux-x64' && receipt.evidence === 'unsigned-compilation-only'
    && REPO.test(repository) && receipt.repository === repository && receipt.input_hash === inputHash
    && /^[a-f0-9]{64}$/.test(inputHash) && SHA.test(receipt.head_sha)
    && ID.test(receipt.run_id) && ID.test(receipt.run_attempt) && ID.test(receipt.artifact_id)
    && Number.isSafeInteger(receipt.completed_at_ms) && receipt.completed_at_ms >= 0
    && receipt.completed_at_ms <= now && now - receipt.completed_at_ms < RECEIPT_MAX_AGE_MS
    && receipt.artifact_url === `https://github.com/${repository}/actions/runs/${receipt.run_id}/artifacts/${receipt.artifact_id}`;
}

// A PR cache can be populated by PR code. Verify the original GitHub run,
// retained artifact and exact committed workflow/build tree before trusting a
// receipt even as compilation evidence. Never load cached source or binaries.
export async function verifyReceipt(receipt, repository, inputHash, now, get) {
  if (!validReceipt(receipt, repository, inputHash, now)) return false;
  const root = `/repos/${repository}`;
  const run = await get(`${root}/actions/runs/${receipt.run_id}`);
  if (String(run.id) !== receipt.run_id || String(run.run_attempt) !== receipt.run_attempt
      || run.status !== 'completed' || run.conclusion !== 'success' || run.path !== WORKFLOW
      || run.repository?.full_name !== repository || run.head_sha !== receipt.head_sha) return false;
  const artifact = await get(`${root}/actions/artifacts/${receipt.artifact_id}`);
  const created = Date.parse(artifact.created_at);
  if (String(artifact.id) !== receipt.artifact_id || artifact.expired !== false
      || artifact.workflow_run?.id !== run.id || artifact.workflow_run?.head_sha !== receipt.head_sha
      || artifact.name !== `UNSIGNED-NONRELEASE-linux-x64-${receipt.head_sha}`
      || !Number.isFinite(created) || created > now || now - created >= RECEIPT_MAX_AGE_MS
      || !Number.isFinite(Date.parse(artifact.expires_at)) || Date.parse(artifact.expires_at) <= now) return false;
  const commit = await get(`${root}/git/commits/${receipt.head_sha}`);
  if (commit.sha !== receipt.head_sha || !SHA.test(commit.tree?.sha)) return false;
  const tree = await get(`${root}/git/trees/${commit.tree.sha}?recursive=1`);
  if (tree.truncated !== false || !Array.isArray(tree.tree)) return false;
  return fingerprint(tree.tree) === inputHash;
}

async function getGithub(path) {
  if (!process.env.GITHUB_TOKEN) throw new Error('missing read-only GitHub token');
  const response = await fetch(`https://api.github.com${path}`, {
    headers: { Authorization: `Bearer ${process.env.GITHUB_TOKEN}`, Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28' },
    redirect: 'error', signal: AbortSignal.timeout(15_000),
  });
  if (!response.ok || !response.body) throw new Error('GitHub receipt evidence unavailable');
  const chunks = []; let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > 4 * 1024 * 1024) throw new Error('GitHub receipt evidence exceeded bound');
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}

export async function readReceipt(path) {
  const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const metadata = await file.stat();
    if (!metadata.isFile() || metadata.size > RECEIPT_MAX_BYTES) throw new Error('invalid receipt file');
    const bytes = Buffer.alloc(RECEIPT_MAX_BYTES + 1);
    const { bytesRead } = await file.read(bytes, 0, bytes.length, 0);
    if (bytesRead > RECEIPT_MAX_BYTES) throw new Error('receipt exceeded size bound');
    return JSON.parse(bytes.subarray(0, bytesRead).toString('utf8'));
  } finally { await file.close(); }
}

async function main(mode) {
  if (!['key', 'check', 'save'].includes(mode) || process.env.GITHUB_ACTIONS !== 'true'
      || process.env.RUNNER_ENVIRONMENT !== 'github-hosted' || !REPO.test(process.env.GITHUB_REPOSITORY ?? '')) {
    throw new Error('receipt tooling requires its hosted workflow');
  }
  const repository = process.env.GITHUB_REPOSITORY;
  const { input_hash, head_sha } = currentInputs();
  const directory = join(await realpath(process.env.RUNNER_TEMP), 'chariox-native-build-receipt');
  const path = join(directory, 'receipt.json');
  if (mode === 'key') {
    if (!ID.test(process.env.GITHUB_RUN_ID) || !ID.test(process.env.GITHUB_RUN_ATTEMPT)) throw new Error('missing hosted run identity');
    await mkdir(directory, { mode: 0o700 });
    const prefix = `${receiptKey(input_hash, Date.now())}-`;
    await appendFile(process.env.GITHUB_OUTPUT,
      `key=${prefix}${process.env.GITHUB_RUN_ID}-${process.env.GITHUB_RUN_ATTEMPT}\nprefix=${prefix}\npath=${path}\n`);
  } else if (mode === 'check') {
    let receipt; let reuse = false;
    try {
      receipt = await readReceipt(path);
      reuse = await verifyReceipt(receipt, repository, input_hash, Date.now(), getGithub);
    } catch { /* Missing, stale, corrupt or unverifiable evidence means rebuild. */ }
    await appendFile(process.env.GITHUB_OUTPUT, `reuse=${reuse}\n`);
    if (reuse) {
      await appendFile(process.env.GITHUB_STEP_SUMMARY,
        `Reusing [unsigned compilation evidence](${receipt.artifact_url}) from [run ${receipt.run_id}](https://github.com/${repository}/actions/runs/${receipt.run_id}) for native inputs \`${input_hash}\`. This is not a trusted runnable release.\n`);
    }
  } else {
    const receipt = {
      schema: 'chariox.unsigned-native-build-receipt.v1', evidence: 'unsigned-compilation-only',
      target: 'linux-x64', repository, input_hash, head_sha, completed_at_ms: Date.now(),
      run_id: process.env.GITHUB_RUN_ID, run_attempt: process.env.GITHUB_RUN_ATTEMPT,
      artifact_id: process.env.NATIVE_ARTIFACT_ID, artifact_url: process.env.NATIVE_ARTIFACT_URL,
    };
    if (!validReceipt(receipt, repository, input_hash, receipt.completed_at_ms)) throw new Error('invalid successful artifact identity');
    const bytes = `${JSON.stringify(receipt)}\n`;
    if (Buffer.byteLength(bytes) > RECEIPT_MAX_BYTES) throw new Error('receipt exceeded size bound');
    // Replace a rejected restored receipt without following its possible symlink.
    const temporary = join(directory, 'new-receipt.json');
    await writeFile(temporary, bytes, { flag: 'wx', mode: 0o600 });
    await rename(temporary, path);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main(process.argv[2]).catch(() => { process.stderr.write('native CI receipt operation failed\n'); process.exitCode = 1; });
}
