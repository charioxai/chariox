// Disposable root coordinator; starts only a non-root confined worker.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream, openSync, closeSync } from 'node:fs';
import { access, chown, readFile, realpath, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { FrameDecoder, encodeFrame, validateMessage } from '../../../packages/app-sdk/src/protocol.js';
import { launchRecord, launchString } from './launch-record.mjs';
import { boundedText, inspectWorker, namespaces, systemLibraryMounts } from './embedded-platform.mjs';

const [repository, scratch, uidText, gidText, unit] = process.argv.slice(2);
assert.equal(process.platform, 'linux'); assert.equal(process.arch, 'x64'); assert.equal(process.getuid(), 0);
assert.match(unit, /^chariox-embedded-\d+-\d+$/);
const uid = Number(uidText); const gid = Number(gidText);
assert.ok(Number.isSafeInteger(uid) && uid > 0 && Number.isSafeInteger(gid) && gid > 0);
assert.equal((await boundedText('/proc/self/cgroup')).trim(), `0::/system.slice/${unit}.service`);
process.setgroups([]);

const mounts = join(scratch, 'mounts');
const roots = { package: '/app/package', data: '/app/data', tmp: '/app/tmp', runtime: '/runtime' };
const bwrap = join(scratch, 'bin/chariox-bwrap');
const launcher = join(scratch, 'bin/chariox-app-worker');
const baseline = await namespaces();
const libraries = await systemLibraryMounts(join(scratch, 'bundle'), launcher);
// Synthetic fixture identity, not a verified/signed .cxapp release digest.
const identity = createHash('sha256');
for (const name of ['main.mjs', 'stalled.mjs', 'value.cjs', 'value.mjs'])
  identity.update(launchString(name)).update(launchString(await readFile(join(mounts, 'package/runtime', name))));
const releaseDigest = identity.digest('hex');
const readiness = Buffer.concat([Buffer.from('CXAWR001'), ...['1', 'probe-install', releaseDigest].map(launchString)]);
const active = new Map();
const results = [];

function start(kind, entry = 'runtime/main.mjs', startupTimeoutMs = 10000) {
  const config = { version: 1, entry, declarations: { tools: ['probe'], incomingEvents: [] }, startupTimeoutMs };
  if (entry !== 'runtime/main.mjs') config.declarations.tools = [];
  const bootstrap = `require('node:module').createRequire('/runtime/bootstrap.cjs')('/runtime/bootstrap.cjs').start(${JSON.stringify(config)});`;
  const args = ['--unshare-user', '--unshare-pid', '--unshare-net', '--unshare-ipc', '--unshare-uts', '--unshare-cgroup',
    '--disable-userns', '--cap-drop', 'ALL', '--new-session', '--die-with-parent', '--as-pid-1',
    '--clearenv', '--setenv', 'CX_TEST_SECRET', 'fixture-only', '--json-status-fd', '5',
    '--ro-bind', join(mounts, 'package'), roots.package,
    '--bind', join(mounts, `${kind}-data`), roots.data, '--bind', join(mounts, `${kind}-tmp`), roots.tmp,
    '--ro-bind', join(mounts, 'runtime'), roots.runtime,
    '--ro-bind', launcher, '/usr/libexec/chariox-app-worker',
    ...libraries.flatMap(({ source, target }) => ['--ro-bind', source, target]),
    '--ro-bind', falsePath, '/usr/bin/false', '--dev-bind', '/dev/null', '/dev/null',
    '--chdir', roots.data, '--remount-ro', '/', '--', '/usr/libexec/chariox-app-worker'];
  const sentinel = openSync(join(scratch, 'secret'), 'r');
  const stdio = Array.from({ length: 32 }, () => 'ignore');
  for (const fd of [1, 2, 3, 4, 5]) stdio[fd] = 'pipe';
  stdio[31] = sentinel;
  const child = spawn(bwrap, args, { uid, gid, env: {}, cwd: '/', stdio });
  closeSync(sentinel);
  const output = { stdout: '', stderr: '', status: '', ready: Buffer.alloc(0), messages: [] };
  let received = 0;
  const timer = setTimeout(() => child.kill('SIGKILL'), 25000);
  const decoder = new FrameDecoder(message => {
    validateMessage(message, 'worker');
    assert.equal(message.generation, '1'); assert.ok(output.messages.length < 32);
    output.messages.push(message);
  });
  for (const [fd, key] of [[1, 'stdout'], [2, 'stderr'], [3, 'messages'], [4, 'ready'], [5, 'status']]) {
    child.stdio[fd].on('error', () => {});
    child.stdio[fd].on('data', bytes => {
      received += bytes.length;
      if (received > 2 * 1024 * 1024) { child.kill('SIGKILL'); return; }
      try {
        if (fd === 3) decoder.push(bytes);
        else if (fd === 4) { assert.ok(output.ready.length + bytes.length < 1024); output.ready = Buffer.concat([output.ready, bytes]); }
        else { assert.ok(Buffer.byteLength(output[key]) + bytes.length <= 65536); output[key] += bytes.toString('utf8'); }
      } catch { output.invalid = true; child.kill('SIGKILL'); }
    });
  }
  const completed = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => {
      clearTimeout(timer); active.delete(child);
      try { decoder.end(); } catch { output.invalid = true; }
      resolve({ ...output, code, signal });
    });
  });
  // Attach a rejection handler immediately, including failures before readiness.
  active.set(child, completed.catch(() => {}));
  child.stdio[4].write(launchRecord(roots, { digest: releaseDigest, bootstrap }));
  return { child, output, completed,
    send: message => child.stdio[3].write(encodeFrame({ version: 1, generation: '1', ...message })) };
}

const falsePath = await realpath('/usr/bin/false');

async function until(running, predicate) {
  const end = Date.now() + 12000;
  while (!predicate() && Date.now() < end && running.child.exitCode === null && running.child.signalCode === null)
    await new Promise(resolve => setTimeout(resolve, 10));
  assert.ok(predicate(), JSON.stringify(running.output));
}
async function confinedStart(kind, entry, timeout) {
  const running = start(kind, entry, timeout);
  await until(running, () => running.output.ready.length === readiness.length && running.output.status.includes('\n'));
  assert.deepEqual(running.output.ready, readiness);
  const status = running.output.status.split('\n').filter(Boolean).map(line => JSON.parse(line)).find(value => Number.isSafeInteger(value['child-pid']));
  assert.ok(status);
  const observed = await inspectWorker(status['child-pid'], { baseline, unit, uid, gid, launcher, roots });
  await assert.rejects(access(join(mounts, `${kind}-data/app-imported`)));
  assert.equal(running.output.messages.length, 0, 'App SDK cannot run before continuation');
  running.child.stdio[4].write('CXAWGO01');
  return { running, observed };
}
async function ready(running) {
  const selected = () => running.output.messages.find(message => message.kind === 'request' && message.method === 'worker.ready');
  await until(running, selected);
  const message = selected();
  assert.deepEqual(message.params, { tools: ['probe'], events: [], lifecycle: ['shutdown'] });
  running.send({ kind: 'response', id: message.id, result: null });
}
async function invoke(running, id, method, params) {
  running.send({ kind: 'request', id, method, params, deadline_ms: Date.now() + 5000 });
  const selected = () => running.output.messages.find(message => message.kind === 'response' && message.id === id);
  await until(running, selected);
  assert.ok(!Object.hasOwn(selected(), 'error'), JSON.stringify(selected()));
  return selected().result;
}
async function fileHash(path) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path, { highWaterMark: 65536 })) hash.update(chunk);
  return hash.digest('hex');
}

try {
  const first = await confinedStart('run');
  await ready(first.running);
  const checks = await invoke(first.running, 'probe', 'tools.invoke', { name: 'probe', input: {} });
  assert.equal(checks.nodeVersion, '24.20.0'); assert.equal(checks.moduleAbi, '137');
  for (const name of ['esmAndCjs', 'sdkFrozen', 'privateIo', 'privateWatch', 'crypto', 'environmentFiltered', 'ambientFdDenied',
    'hostReadDenied', 'escapedReadDenied', 'packageWriteDenied', 'childDenied', 'addonDenied', 'tcpDenied', 'udpDenied'])
    assert.equal(checks[name], true, JSON.stringify(checks));
  const shutdown = await invoke(first.running, 'shutdown', 'lifecycle.dispatch', { event: 'shutdown' });
  assert.equal(shutdown, 'shutdown response drained');
  const ended = await first.running.completed;
  assert.equal(ended.code, 0, JSON.stringify(ended)); assert.equal(ended.signal, null); assert.ok(!ended.invalid);
  results.push({ name: 'confined-embedded-sdk-io-and-denials', passed: true, observed: first.observed, checks });

  const second = await confinedStart('disconnect'); await ready(second.running);
  second.running.child.stdio[3].destroy();
  const disconnected = await second.running.completed;
  assert.equal(disconnected.code, 133, JSON.stringify(disconnected));
  results.push({ name: 'kernel-channel-disconnect-terminates-worker', passed: true });

  const third = await confinedStart('deadline', 'runtime/stalled.mjs', 500);
  const timedOut = await third.running.completed;
  assert.equal(timedOut.code, 132, JSON.stringify(timedOut)); assert.equal(timedOut.messages.length, 0);
  results.push({ name: 'stalled-registration-startup-deadline', passed: true });
} finally {
  const remaining = [...active];
  for (const [child] of remaining) child.kill('SIGKILL');
  if (remaining.length) {
    let cleanupTimer;
    await Promise.race([Promise.allSettled(remaining.map(([, completion]) => completion)),
      new Promise(resolve => { cleanupTimer = setTimeout(resolve, 2000); })]);
    clearTimeout(cleanupTimer);
  }
  const evidence = {
    schema: 'chariox.embedded-linux-fixture.v1', results,
    nativeSource: JSON.parse(await readFile(join(scratch, 'evidence/native-source.json'), 'utf8')),
    bundle: JSON.parse(await readFile(join(scratch, 'evidence/bundle-identity.json'), 'utf8')),
    launcherSha256: await fileHash(launcher), bubblewrapSha256: await fileHash(bwrap),
    systemLibraries: await Promise.all(libraries.map(async file => ({ target: file.target, sha256: await fileHash(file.source) }))),
    budget: { memoryBytes: 2147483648, swapBytes: 0, cpuPercent: 100, tasks: 128, timeoutSeconds: 180 },
    scope: 'Historical unsigned Linux x64 embedded execution fixture; not runtime enrollment, an installer, macOS, or complete release validation.',
  };
  const path = join(scratch, 'evidence/embedded-linux.json');
  await writeFile(path, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 }); await chown(path, uid, gid);
  console.log(JSON.stringify(evidence));
}
