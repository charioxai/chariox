// Hosted development evidence only. Never download/execute on a user machine.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { appendFile, mkdir, mkdtemp, open, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { packageRuntime, verifyBundle } from './package-app-runtime.mjs';

export const NATIVE_RUN = '34176513092';
export const NATIVE_HEAD = 'a028881fb2a2cf487b910a439b345f2b7713ac8d';
const REPOSITORY_NAME = 'charioxai/chariox';
const WORKFLOW = '.github/workflows/app-runtime-native.yml';
const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const MAX_ARCHIVE = 512 * 1024 * 1024;
const HASH = /^sha256:[a-f0-9]{64}$/;

export async function selectArtifact(runId, get, now = Date.now()) {
  // One reviewed source input, not an arbitrary artifact/run supplied by a
  // workflow caller. Future successful native sources require an explicit edit.
  assert.equal(runId, NATIVE_RUN, 'native run is not the reviewed fixture input');
  const prefix = `/repos/${REPOSITORY_NAME}/actions`;
  const run = await get(`${prefix}/runs/${runId}`);
  assert.equal(String(run.id), runId);
  assert.equal(run.repository?.full_name, REPOSITORY_NAME);
  assert.equal(run.head_repository?.full_name, REPOSITORY_NAME);
  assert.equal(run.path, WORKFLOW);
  assert.equal(run.head_sha, NATIVE_HEAD);
  assert.equal(run.run_attempt, 1);
  assert.ok(['pull_request', 'workflow_dispatch'].includes(run.event));
  if (run.status !== 'completed') return { ready: false, runId, sourceCommit: NATIVE_HEAD };
  assert.equal(run.conclusion, 'success', 'native build did not succeed');
  const list = await get(`${prefix}/runs/${runId}/artifacts?per_page=10`);
  assert.ok(Number.isInteger(list.total_count) && list.total_count >= 1 && list.total_count <= 10);
  assert.equal(list.artifacts?.length, list.total_count);
  const matches = list.artifacts.filter(artifact => artifact.name === `UNSIGNED-NONRELEASE-linux-x64-${NATIVE_HEAD}`);
  assert.equal(matches.length, 1);
  const artifact = matches[0];
  assert.ok(Number.isSafeInteger(artifact.id) && artifact.id > 0);
  assert.equal(artifact.expired, false);
  assert.equal(artifact.workflow_run?.id, Number(runId));
  assert.equal(artifact.workflow_run?.head_sha, NATIVE_HEAD);
  assert.ok(Number.isSafeInteger(artifact.size_in_bytes) && artifact.size_in_bytes > 0 && artifact.size_in_bytes <= MAX_ARCHIVE);
  assert.ok(HASH.test(artifact.digest));
  assert.ok(Number.isFinite(Date.parse(artifact.created_at)) && Date.parse(artifact.created_at) <= now
    && Date.parse(artifact.created_at) >= Date.parse(run.run_started_at));
  assert.ok(Date.parse(artifact.expires_at) > now);
  return { ready: true, repository: REPOSITORY_NAME, workflow: WORKFLOW, runId,
    runAttempt: run.run_attempt, sourceCommit: NATIVE_HEAD, artifactId: String(artifact.id),
    size: artifact.size_in_bytes, digest: artifact.digest,
    url: `https://github.com/${REPOSITORY_NAME}/actions/runs/${runId}/artifacts/${artifact.id}` };
}

async function github(path, options = {}) {
  assert.ok(process.env.GITHUB_TOKEN);
  return fetch(`https://api.github.com${path}`, { redirect: 'manual', signal: AbortSignal.timeout(15000),
    headers: { Authorization: `Bearer ${process.env.GITHUB_TOKEN}`, Accept: 'application/vnd.github+json',
      'X-GitHub-Api-Version': '2022-11-28' }, ...options });
}

async function boundedJson(response) {
  assert.equal(response.status, 200);
  const chunks = []; let size = 0;
  try {
    for await (const chunk of response.body) {
      size += chunk.length;
      assert.ok(size <= 1024 * 1024, 'oversized GitHub metadata');
      chunks.push(chunk);
    }
    return JSON.parse(Buffer.concat(chunks).toString('utf8'));
  } finally { await response.body?.cancel().catch(() => {}); }
}

export function artifactRedirect(location) {
  const target = new URL(location);
  assert.equal(target.protocol, 'https:');
  assert.equal(target.username, ''); assert.equal(target.password, '');
  assert.ok(!target.port || target.port === '443');
  assert.ok(target.hostname.endsWith('.blob.core.windows.net') || target.hostname.endsWith('.actions.githubusercontent.com'));
  return target;
}

export async function saveArchive(response, path, evidence) {
  assert.equal(response.status, 200);
  assert.ok(evidence.size > 0 && evidence.size <= MAX_ARCHIVE && HASH.test(evidence.digest));
  const output = await open(path, 'wx', 0o600);
  const hash = createHash('sha256'); let size = 0;
  try {
    for await (const chunk of response.body) {
      size += chunk.length;
      assert.ok(size <= evidence.size && size <= MAX_ARCHIVE, 'native archive exceeded its byte ceiling');
      hash.update(chunk);
      await output.writeFile(chunk);
    }
    assert.equal(size, evidence.size);
    assert.equal(`sha256:${hash.digest('hex')}`, evidence.digest);
    await output.sync();
  } catch (error) { await rm(path, { force: true }); throw error; }
  finally { await output.close(); await response.body?.cancel().catch(() => {}); }
}

let stage = 'environment';
async function prepare() {
  assert.equal(process.env.GITHUB_ACTIONS, 'true');
  assert.equal(process.env.RUNNER_ENVIRONMENT, 'github-hosted');
  assert.equal(process.platform, 'linux'); assert.equal(process.arch, 'x64');
  assert.equal(process.env.GITHUB_REPOSITORY, REPOSITORY_NAME);
  const scratch = await mkdtemp(join(await realpath(process.env.RUNNER_TEMP), 'chariox-embedded.'));
  await mkdir(join(scratch, 'evidence'), { mode: 0o700 });
  await appendFile(process.env.GITHUB_OUTPUT, `scratch=${scratch}\nevidence=${scratch}/evidence\n`);
  stage = 'native_run';
  const selected = await selectArtifact(process.env.NATIVE_RUN ?? NATIVE_RUN, async path => boundedJson(await github(path)));
  await writeFile(join(scratch, 'evidence/native-source.json'), `${JSON.stringify(selected, null, 2)}\n`, { mode: 0o600 });
  stage = selected.ready ? 'download' : 'native_pending';
  assert.equal(selected.ready, true, 'native artifact not ready; embedded execution remains unverified');
  const redirect = await github(`/repos/${REPOSITORY_NAME}/actions/artifacts/${selected.artifactId}/zip`);
  assert.equal(redirect.status, 302);
  const target = artifactRedirect(redirect.headers.get('location'));
  await redirect.body?.cancel();
  // The short-lived storage URL gets no GitHub credential or other headers.
  const archive = join(scratch, 'native.zip');
  await saveArchive(await fetch(target, { redirect: 'error', signal: AbortSignal.timeout(120000) }), archive, selected);
  stage = 'zip';
  execFileSync('/usr/bin/python3', [join(REPOSITORY, 'scripts/app-runtime-extract-zip.py'), '--archive', archive,
    '--destination', join(scratch, 'native'), '--sha256', selected.digest.slice(7)],
  { timeout: 60000, maxBuffer: 65536, stdio: ['ignore', 'pipe', 'pipe'], env: { PATH: '/usr/bin:/bin', LANG: 'C.UTF-8' } });
  const native = JSON.parse(await readFile(join(scratch, 'native/artifact-manifest.json'), 'utf8'));
  assert.equal(native.sourceCommit, selected.sourceCommit);
  stage = 'bundle';
  const bundle = await packageRuntime({ nativeDirectory: join(scratch, 'native'), output: join(scratch, 'bundle'),
    target: 'linux-x64', allowHistorical: true });
  await verifyBundle(join(scratch, 'bundle'));
  // verifyBundle retains either exact current native inputs or explicit historical
  // provenance. Neither classification grants signing or runtime enrollment.
  assert.equal(bundle.native.sourceCommit, selected.sourceCommit);
  assert.equal(bundle.signing.status, 'unsigned');
  assert.equal(bundle.signing.enrollment, 'not-performed');
  await writeFile(join(scratch, 'evidence/bundle-identity.json'), `${JSON.stringify({
    digest: bundle.bundleDigest, native: bundle.native, signing: bundle.signing,
  }, null, 2)}\n`, { mode: 0o600 });
  await rm(archive);
  await rm(join(scratch, 'native'), { recursive: true });
  await appendFile(process.env.GITHUB_OUTPUT, 'ready=true\n');
  console.log(JSON.stringify({ artifactId: selected.artifactId, sourceCommit: selected.sourceCommit, bundleDigest: bundle.bundleDigest,
    scope: 'Reviewed unsigned runtime prepared; embedded execution not yet performed' }));
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  prepare().catch(() => { console.error(`embedded_runtime_artifact_preparation_failed:${stage}`); process.exitCode = 1; });
}
