// Root-owned hosted fixture coordinator, never shipped or exposed to Apps.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { openSync, closeSync } from 'node:fs';
import { access, chown, readFile, readlink, realpath, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { launchRecord, expectedReadiness } from './launch-record.mjs';

const [repository, scratch, uidText, gidText, unit] = process.argv.slice(2);
assert.equal(process.platform, 'linux');
assert.equal(process.arch, 'x64');
assert.equal(process.getuid(), 0);
assert.match(unit, /^chariox-native-\d+-\d+$/);
const uid = Number(uidText); const gid = Number(gidText);
assert.ok(Number.isSafeInteger(uid) && uid > 0 && Number.isSafeInteger(gid) && gid > 0);
assert.equal((await readFile('/proc/self/cgroup', 'utf8')).trim(), `0::/system.slice/${unit}.service`);
process.setgroups([]);

const mounts = path.join(scratch, 'mounts');
const bwrap = path.join(scratch, 'bin/chariox-bwrap');
const roots = { package: '/app/package', data: '/app/data', tmp: '/app/tmp', runtime: '/runtime' };
const baselineNamespaces = Object.fromEntries(await Promise.all(['mnt', 'user', 'pid', 'net', 'ipc', 'uts', 'cgroup']
  .map(async name => [name, await readlink(`/proc/self/ns/${name}`)])));
const libc = await realpath('/lib/x86_64-linux-gnu/libc.so.6');
const interpreter = await realpath('/lib64/ld-linux-x86-64.so.2');
const active = new Set();
const outcomes = [];

function start(kind, executable = 'chariox-app-worker', bytes = launchRecord(roots)) {
  const args = [
    '--unshare-user', '--unshare-pid', '--unshare-net', '--unshare-ipc', '--unshare-uts', '--unshare-cgroup',
    '--disable-userns', '--cap-drop', 'ALL', '--new-session', '--die-with-parent', '--as-pid-1',
    '--clearenv', '--setenv', 'CX_TEST_SECRET', 'fixture-only-must-be-filtered', '--json-status-fd', '5',
    '--ro-bind', path.join(mounts, 'package'), roots.package,
    '--bind', path.join(mounts, `${kind}-data`), roots.data,
    '--bind', path.join(mounts, `${kind}-tmp`), roots.tmp,
    '--ro-bind', path.join(mounts, 'runtime'), roots.runtime,
    '--ro-bind', libc, '/lib/x86_64-linux-gnu/libc.so.6',
    '--ro-bind', interpreter, '/lib64/ld-linux-x86-64.so.2',
    '--ro-bind', awaitFalsePath, '/usr/bin/false',
    '--dev-bind', '/dev/null', '/dev/null',
    '--chdir', roots.data, '--remount-ro', '/', '--', `${roots.runtime}/${executable}`,
  ];
  const sentinel = openSync(path.join(scratch, 'secret'), 'r');
  const stdio = Array.from({ length: 32 }, () => 'ignore');
  for (const fd of [1, 2, 3, 4, 5]) stdio[fd] = 'pipe';
  stdio[31] = sentinel;
  const child = spawn(bwrap, args, { uid, gid, env: {}, cwd: '/', stdio });
  closeSync(sentinel);
  active.add(child);
  const output = { stdout: '', stderr: '', sdk: '', ready: Buffer.alloc(0), status: '' };
  let outputBytes = 0;
  const timeout = setTimeout(() => child.kill('SIGKILL'), 20000);
  for (const [fd, key] of [[1, 'stdout'], [2, 'stderr'], [3, 'sdk'], [4, 'ready'], [5, 'status']]) {
    child.stdio[fd].on('error', () => {});
    child.stdio[fd].on('data', chunk => {
      outputBytes += chunk.length;
      if (outputBytes > 65536) { child.kill('SIGKILL'); return; }
      output[key] = key === 'ready' ? Buffer.concat([output.ready, chunk]) : output[key] + chunk.toString('utf8');
    });
  }
  const completed = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => {
      clearTimeout(timeout); active.delete(child);
      resolve({ ...output, code, signal });
    });
  });
  child.stdio[3].write('PING');
  child.stdio[4].write(bytes);
  return { child, output, completed };
}

const awaitFalsePath = await realpath('/usr/bin/false');

async function waitReady(running) {
  const deadline = Date.now() + 10000;
  while ((running.output.ready.length < expectedReadiness.length || !running.output.status.includes('\n')) && Date.now() < deadline) {
    if (running.child.exitCode !== null || running.child.signalCode !== null) break;
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  assert.deepEqual(running.output.ready, expectedReadiness, JSON.stringify(running.output));
}

async function inspectWorker(running) {
  const statusLine = running.output.status.split('\n').filter(Boolean).map(line => JSON.parse(line))
    .find(value => Number.isSafeInteger(value['child-pid']));
  assert.ok(statusLine, 'bundled bwrap must identify its exact PID1 worker');
  const pid = statusLine['child-pid'];
  const status = await readFile(`/proc/${pid}/status`, 'utf8');
  assert.match(status, /^CapEff:\s+0000000000000000$/m);
  assert.match(status, /^CapPrm:\s+0000000000000000$/m);
  assert.match(status, /^CapInh:\s+0000000000000000$/m);
  assert.match(status, /^NoNewPrivs:\s+1$/m);
  assert.match(status, /^Seccomp:\s+2$/m);
  const uids = status.match(/^Uid:\s+(.+)$/m)[1].trim().split(/\s+/).map(Number);
  assert.deepEqual(uids, [uid, uid, uid, uid]);
  const gids = status.match(/^Gid:\s+(.+)$/m)[1].trim().split(/\s+/).map(Number);
  assert.deepEqual(gids, [gid, gid, gid, gid]);
  assert.match(status, /^Groups:[ \t]*$/m);
  const namespaces = Object.fromEntries(await Promise.all(Object.keys(baselineNamespaces)
    .map(async name => [name, await readlink(`/proc/${pid}/ns/${name}`)])));
  for (const [name, value] of Object.entries(namespaces))
    assert.notEqual(value, baselineNamespaces[name], `${name} namespace must differ`);
  assert.equal((await readFile(`/proc/${pid}/cgroup`, 'utf8')).trim(), `0::/system.slice/${unit}.service`);
  const metadata = await readFile(`/proc/${pid}/mountinfo`, 'utf8');
  assert.ok(Buffer.byteLength(metadata) < 65536);
  const observedMounts = {};
  for (const root of ['/', ...Object.values(roots)]) {
    const line = metadata.split('\n').find(line => line.split(' ')[4] === root);
    assert.ok(line, `missing mount ${root}`);
    const options = line.split(' ')[5].split(',');
    observedMounts[root] = options;
    if (root === '/' || root === roots.package || root === roots.runtime) assert.ok(options.includes('ro'));
    if (root !== '/') {
      assert.ok(options.includes('nodev') && options.includes('nosuid'));
      if (root !== roots.runtime) assert.ok(options.includes('noexec'));
    }
  }
  await assert.rejects(access(`/proc/${pid}/root/proc`));
  await assert.rejects(access(`/proc/${pid}/root/etc/passwd`));
  const executable = await stat(`/proc/${pid}/exe`);
  const expected = await stat(path.join(mounts, 'runtime/chariox-app-worker'));
  assert.equal(executable.ino, expected.ino);
  assert.equal(executable.dev, expected.dev);
  return { namespaces, mounts: observedMounts, uids, gids, supplementaryGroups: [], noNewPrivileges: true, capabilities: 'none', seccompFilter: true };
}

try {
  const running = start('good');
  await waitReady(running);
  await assert.rejects(access(path.join(mounts, 'good-data/constructor-ran')));
  const observed = await inspectWorker(running);
  running.child.stdio[4].write('CXAWGO01');
  const result = await running.completed;
  assert.equal(result.code, 0, JSON.stringify(result));
  assert.equal(result.signal, null);
  assert.equal(result.sdk, 'PONG');
  const checks = Object.fromEntries(result.stdout.trim().split('\n').map(line => line.split(':')));
  for (const name of [
    'constructor_already_confined', 'environment_filtered', 'ambient_descriptor_closed', 'constructor_host_read_denied',
    'trusted_bootstrap', 'inherited_sdk_bidirectional', 'native_thread_create_join',
    'private_data_write', 'private_tmp_write', 'package_write_denied', 'parent_traversal_denied',
    'unrelated_host_read_denied', 'package_read', 'package_executable_mapping_denied',
    'package_mapping_cannot_become_executable', 'anonymous_memory', 'raw_network_denied',
    'fork_denied', 'foreign_signal_denied', 'native_limit_query', 'native_limit_mutation_denied',
    'foreign_limit_query_denied', 'private_descriptor_copy', 'private_file_watch', 'private_file_watch_event',
    'escaped_file_watch_denied', 'private_file_watch_remove', 'exec_denied',
  ]) assert.equal(checks[name], 'ok', JSON.stringify(result));
  assert.ok(!result.stdout.includes(':FAIL'));
  outcomes.push({ name: 'production-native-boundary', passed: true, observed, checks });

  const bad = start('bad');
  const badResult = await bad.completed;
  assert.equal(badResult.code, 104, JSON.stringify(badResult));
  assert.equal(badResult.ready.length, 0);
  await assert.rejects(access(path.join(mounts, 'bad-data/constructor-ran')));
  outcomes.push({ name: 'executable-data-mount-rejected-before-readiness', passed: true });

  const weakened = start('weak', 'weakened-test-worker');
  await waitReady(weakened);
  weakened.child.stdio[4].write('CXAWGO01');
  const weakResult = await weakened.completed;
  assert.equal(weakResult.code, 1, JSON.stringify(weakResult));
  assert.match(weakResult.stdout, /raw_network_denied:FAIL/);
  assert.match(weakResult.stdout, /fork_denied:FAIL/);
  outcomes.push({ name: 'removed-native-policy-detected', passed: true });
} finally {
  for (const child of active) child.kill('SIGKILL');
}

const hashes = {};
for (const relative of [
  'bin/chariox-bwrap', 'bin/chariox-app-worker', 'bin/weakened-test-worker', 'bin/libchariox-app-runtime.so',
]) hashes[relative] = createHash('sha256').update(await readFile(path.join(scratch, relative))).digest('hex');
const evidence = {
  schema: 'chariox.native-linux-fixture.v1',
  sourceCommit: execFileSync('/usr/bin/git', ['-c', `safe.directory=${repository}`, '-C', repository, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
  os: await readFile('/etc/os-release', 'utf8'), kernel: execFileSync('/usr/bin/uname', ['-r'], { encoding: 'utf8' }).trim(),
  compiler: execFileSync('/usr/bin/gcc', ['--version'], { encoding: 'utf8' }).split('\n')[0],
  bubblewrap: JSON.parse(await readFile(path.join(repository, 'apps/app-worker/sandbox.lock.json'), 'utf8')).bubblewrap,
  hashes, outcomes,
  budget: { memoryBytes: 1073741824, swapBytes: 0, cpuPercent: 100, tasks: 64, timeoutSeconds: 120 },
  scope: 'Hosted Linux x64 native libc fixture and explicit root provisioning; not an installer, Node/libuv/V8 compatibility, signing, or full resource-quota acceptance.',
};
const evidencePath = path.join(scratch, 'evidence/native-linux.json');
await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 });
await chown(evidencePath, uid, gid);
console.log(JSON.stringify(evidence));
