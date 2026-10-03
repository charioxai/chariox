// Explicit heavyweight CI admission. Labels are one-shot events, not standing
// permission to compile later synchronized PR heads. This grants no release trust.
import assert from 'node:assert/strict';
import { constants } from 'node:fs';
import { appendFile, open } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export const BUILD_LABELS = Object.freeze({ 'linux-x64': 'app-runtime-build-linux', 'darwin-arm64': 'app-runtime-build-macos' });
const REPOSITORY = 'charioxai/chariox';
const SHA = /^[a-f0-9]{40}$/;
const ACTOR = /^[A-Za-z0-9_-]{1,64}(?:\[bot\])?$/;

export async function admitBuild(target, eventName, event, environment, permission) {
  assert.ok(Object.hasOwn(BUILD_LABELS, target));
  assert.equal(environment.GITHUB_ACTIONS, 'true');
  assert.equal(environment.RUNNER_ENVIRONMENT, 'github-hosted');
  assert.equal(environment.GITHUB_REPOSITORY, REPOSITORY);
  assert.equal(event.repository?.full_name, REPOSITORY);
  const actor = event.sender?.login; assert.match(actor ?? '', ACTOR);
  let source;
  if (eventName === 'workflow_dispatch') {
    assert.ok(event.inputs?.confirm_build === true || event.inputs?.confirm_build === 'true');
    source = environment.GITHUB_SHA;
  } else {
    assert.equal(eventName, 'pull_request');
    assert.equal(event.action, 'labeled');
    assert.equal(event.label?.name, BUILD_LABELS[target]);
    assert.equal(event.pull_request?.head?.repo?.full_name, REPOSITORY);
    assert.equal(event.pull_request?.base?.repo?.full_name, REPOSITORY);
    source = event.pull_request.head.sha;
  }
  assert.match(source ?? '', SHA);
  // Check current effective repository permission, including on workflow reruns.
  const access = await permission(actor);
  assert.ok(['write', 'maintain', 'admin'].includes(access.permission));
  return { target, source_sha: source, actor, event: eventName === 'pull_request' ? 'pull_request:labeled' : eventName };
}

export function buildAdmission(environment, sourceCommit) {
  assert.equal(environment.CHARIOX_NATIVE_BUILD_CONFIRMED, 'true');
  assert.match(environment.NATIVE_ADMITTED_SHA ?? '', SHA);
  assert.equal(sourceCommit, environment.NATIVE_ADMITTED_SHA);
  assert.match(environment.NATIVE_ADMISSION_ACTOR ?? '', ACTOR);
  assert.ok(['workflow_dispatch', 'pull_request:labeled'].includes(environment.NATIVE_ADMISSION_EVENT));
  return { sourceCommit, actor: environment.NATIVE_ADMISSION_ACTOR, event: environment.NATIVE_ADMISSION_EVENT,
    scope: 'explicit unsigned native compilation only' };
}

async function main() {
  const file = await open(process.env.GITHUB_EVENT_PATH, constants.O_RDONLY | constants.O_NOFOLLOW);
  let event;
  try {
    const metadata = await file.stat(); assert.ok(metadata.isFile() && metadata.size <= 256 * 1024);
    const bytes = Buffer.alloc(256 * 1024 + 1); const read = await file.read(bytes, 0, bytes.length, 0);
    assert.ok(read.bytesRead <= 256 * 1024); event = JSON.parse(bytes.subarray(0, read.bytesRead).toString('utf8'));
  } finally { await file.close(); }
  const result = await admitBuild(process.argv[2], process.env.GITHUB_EVENT_NAME, event, process.env, async actor => {
    assert.ok(process.env.GITHUB_TOKEN);
    const response = await fetch(`https://api.github.com/repos/${REPOSITORY}/collaborators/${encodeURIComponent(actor)}/permission`, {
      headers: { Authorization: `Bearer ${process.env.GITHUB_TOKEN}`, Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28' },
      redirect: 'error', signal: AbortSignal.timeout(10_000),
    });
    if (!response.ok || !response.body) throw new Error('repository permission unavailable');
    const chunks = []; let length = 0;
    for await (const chunk of response.body) { length += chunk.length; if (length > 65536) throw new Error('permission response exceeded bound'); chunks.push(chunk); }
    return JSON.parse(Buffer.concat(chunks).toString('utf8'));
  });
  await appendFile(process.env.GITHUB_OUTPUT, `confirmed=true\nsource_sha=${result.source_sha}\nactor=${result.actor}\nadmission_event=${result.event}\n`);
  console.log(JSON.stringify(result));
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href)
  main().catch(() => { console.error('native_build_manual_admission_rejected'); process.exitCode = 1; });
