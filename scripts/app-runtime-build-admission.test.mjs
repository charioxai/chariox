import assert from 'node:assert/strict';
import test from 'node:test';
import { admitBuild, buildAdmission } from './app-runtime-build-admission.mjs';
const repo = { full_name: 'charioxai/chariox' };
const source = 'a'.repeat(40);
const environment = { GITHUB_ACTIONS: 'true', RUNNER_ENVIRONMENT: 'github-hosted', GITHUB_REPOSITORY: repo.full_name, GITHUB_SHA: source };
const event = { action: 'labeled', repository: repo, sender: { login: 'builder' }, label: { name: 'app-runtime-build-macos' },
  pull_request: { head: { repo, sha: source }, base: { repo }, labels: [{ name: 'app-runtime-build-macos' }] } };

test('only the exact labeled same-repository head and an effective writer admit a pre-merge build', async () => {
  for (const permission of ['write', 'maintain', 'admin']) {
    const result = await admitBuild('darwin-arm64', 'pull_request', event, environment, async actor => {
      assert.equal(actor, 'builder'); return { permission };
    });
    assert.equal(result.source_sha, source); assert.equal(result.event, 'pull_request:labeled');
  }
  for (const change of [e => { e.action = 'synchronize'; }, e => { e.action = 'opened'; },
    e => { e.label.name = 'app-runtime-build-linux'; }, e => { delete e.label; },
    e => { e.pull_request.head.repo.full_name = 'fork/repo'; }, e => { e.pull_request.head.sha = 'main'; }]) {
    const altered = structuredClone(event); change(altered);
    await assert.rejects(admitBuild('darwin-arm64', 'pull_request', altered, environment, async () => ({ permission: 'admin' })));
  }
  for (const permission of ['read', 'triage', 'none', undefined])
    await assert.rejects(admitBuild('darwin-arm64', 'pull_request', event, environment, async () => ({ permission })));
  await assert.rejects(admitBuild('darwin-arm64', 'pull_request', event, environment, async () => { throw new Error('unavailable'); }));
});

test('post-merge dispatch still requires explicit confirmation and current writer permission', async () => {
  const dispatch = { repository: repo, sender: { login: 'builder' }, inputs: { confirm_build: true } };
  const result = await admitBuild('linux-x64', 'workflow_dispatch', dispatch, environment, async () => ({ permission: 'write' }));
  assert.equal(result.source_sha, source); assert.equal(result.event, 'workflow_dispatch');
  await assert.rejects(admitBuild('linux-x64', 'workflow_dispatch', { ...dispatch, inputs: { confirm_build: false } }, environment, async () => ({ permission: 'admin' })));
  await assert.rejects(admitBuild('linux-x64', 'push', dispatch, environment, async () => ({ permission: 'admin' })));
});

test('artifact admission is bound to the checked-out source and cannot carry arbitrary metadata', () => {
  const env = { CHARIOX_NATIVE_BUILD_CONFIRMED: 'true', NATIVE_ADMITTED_SHA: source,
    NATIVE_ADMISSION_ACTOR: 'builder', NATIVE_ADMISSION_EVENT: 'pull_request:labeled' };
  assert.equal(buildAdmission(env, source).sourceCommit, source);
  assert.throws(() => buildAdmission(env, 'b'.repeat(40)));
  assert.throws(() => buildAdmission({ ...env, NATIVE_ADMISSION_ACTOR: 'bad\nactor' }, source));
  assert.throws(() => buildAdmission({ ...env, NATIVE_ADMISSION_EVENT: 'pull_request:synchronize' }, source));
});
