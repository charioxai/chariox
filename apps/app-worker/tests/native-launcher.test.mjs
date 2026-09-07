import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { spawn, spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, writeFile, symlink, rm, access } from 'node:fs/promises';
import { openSync, closeSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { launchRecord as record, expectedReadiness } from './launch-record.mjs';

const source = fileURLToPath(new URL('../src/', import.meta.url));
const probe = fileURLToPath(new URL('./native_probe.c', import.meta.url));
const enabled = process.platform === 'darwin';
let scratch;
let launcher;
let weakenedLauncher;
let deadlineProbe;
let fixtureNumber = 0;

function compile(args) {
  const result = spawnSync('/usr/bin/clang', args, {
    encoding: 'utf8', timeout: 60_000, maxBuffer: 128 * 1024,
    env: { PATH: '/usr/bin:/bin', LANG: 'C' },
  });
  assert.equal(result.status, 0, `native compile failed: ${result.stderr}`);
}

before(async () => {
  if (!enabled) return;
  const parent = path.join(homedir(), '.chariox/dev/apps-phase1/native-launcher-tests');
  await mkdir(parent, { recursive: true, mode: 0o700 });
  scratch = await mkdtemp(path.join(parent, 'run-'));
  launcher = path.join(scratch, 'chariox-app-worker');
  compile(['-std=c11', '-Wall', '-Wextra', '-Werror', '-O1', '-I', source,
    ...['launcher.c', 'launch_record.c', 'launch_process.c', 'sandbox_macos.c'].map(f => path.join(source, f)),
    '-o', launcher]);
  compile(['-std=c11', '-Wall', '-Wextra', '-Werror', '-O1', '-dynamiclib', '-I', source,
    probe, '-o', path.join(scratch, 'probe.dylib')]);
  weakenedLauncher = path.join(scratch, 'weakened-test-worker');
  compile(['-std=c11', '-Wall', '-Wextra', '-Werror', '-O1', '-I', source,
    ...['launcher.c', 'launch_record.c', 'launch_process.c'].map(f => path.join(source, f)),
    fileURLToPath(new URL('./weakened_policy.c', import.meta.url)), '-o', weakenedLauncher]);
  deadlineProbe = path.join(scratch, 'record-deadline-test');
  compile(['-std=c11', '-Wall', '-Wextra', '-Werror', '-O1', '-I', source,
    path.join(source, 'launch_record.c'), fileURLToPath(new URL('./record_deadline.c', import.meta.url)),
    '-o', deadlineProbe]);
});

after(async () => { if (scratch) await rm(scratch, { recursive: true, force: true }); });

async function fixture() {
  const parent = path.join(scratch, `fixture-${++fixtureNumber}`);
  await mkdir(parent, { mode: 0o700 });
  const roots = Object.fromEntries(['package', 'data', 'tmp', 'runtime'].map(key => [key, path.join(parent, key)]));
  for (const root of Object.values(roots)) await mkdir(root, { mode: 0o700 });
  await writeFile(path.join(parent, 'secret'), 'private supervisor fixture', { mode: 0o600 });
  await symlink(path.join(parent, 'secret'), path.join(roots.package, 'escape'));
  await writeFile(path.join(roots.package, 'fixture.bin'), Buffer.alloc(4096));
  await writeFile(path.join(roots.runtime, 'libchariox-app-runtime.dylib'), await readFile(path.join(scratch, 'probe.dylib')));
  return roots;
}

function start(bytes, args = [], executable = launcher) {
  const secret = openSync(path.join(scratch, 'probe.dylib'), 'r');
  const stdio = Array.from({ length: 32 }, () => 'ignore');
  stdio[0] = 'ignore'; stdio[1] = 'pipe'; stdio[2] = 'pipe';
  stdio[3] = 'pipe'; stdio[4] = 'pipe'; stdio[31] = secret;
  const child = spawn(executable, args, { stdio, env: { CX_TEST_SECRET: 'must-not-reach-runtime' } });
  closeSync(secret);
  let stdout = ''; let stderr = ''; let sdk = ''; let ready = Buffer.alloc(0);
  child.stdout.on('data', bytes => { stdout += bytes; });
  child.stderr.on('data', bytes => { stderr += bytes; });
  child.stdio[3].on('data', bytes => { sdk += bytes; });
  child.stdio[3].on('error', () => {});
  child.stdio[4].on('data', bytes => { ready = Buffer.concat([ready, bytes]); });
  child.stdio[4].on('error', () => {});
  const timeout = setTimeout(() => child.kill('SIGKILL'), 20_000);
  const completed = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => {
      clearTimeout(timeout);
      resolve({ code, signal, stdout, stderr, ready, sdk });
    });
  });
  child.stdio[3].write('PING');
  child.stdio[4].write(bytes);
  return { child, completed, readiness: () => ready };
}

test('native constructors and operations remain confined after exact supervisor handshake', { skip: !enabled }, async context => {
  const roots = await fixture();
  const running = start(record(roots));
  const expected = expectedReadiness;
  const end = Date.now() + 5000;
  while (running.readiness().length < expected.length && Date.now() < end)
    await new Promise(resolve => setTimeout(resolve, 10));
  assert.deepEqual(running.readiness(), expected);
  await assert.rejects(access(path.join(roots.data, 'constructor-ran')));
  running.child.stdio[4].write('CXAWGO01');
  const result = await running.completed;
  assert.equal(result.code, 0, JSON.stringify(result));
  assert.equal(result.signal, null);
  assert.ok(!result.stdout.includes(':FAIL'), result.stdout);
  assert.equal(result.stdout.trim().split('\n').length, 21, result.stdout);
  assert.equal(result.sdk, 'PONG');
  assert.match(result.stdout, /constructor_host_read_denied:ok/);
  assert.match(result.stdout, /package_executable_mapping_denied:ok/);
  assert.match(result.stdout, /fork_denied:ok/);
  assert.match(result.stdout, /raw_network_denied:ok/);
  assert.match(result.stdout, /exec_denied:ok/);
  context.diagnostic(JSON.stringify({ scope: 'Development macOS libc probe; not signed/hardened Node acceptance', checks: result.stdout.trim().split('\n') }));
});

test('malformed authority records cannot reach readiness or load a runtime', { skip: !enabled }, async () => {
  const roots = await fixture();
  for (const overrides of [
    { generation: '01' }, { generation: '9223372036854775808' },
    { generation: '0' }, { digest: 'A'.repeat(64) }, { installation: 'bad/id' },
    { bootstrap: 'a\0b' }, { bootstrap: Buffer.from([0xc0, 0x80]) },
  ]) {
    const result = await start(record(roots, overrides)).completed;
    assert.equal(result.code, 100, JSON.stringify(result));
    assert.equal(result.ready.length, 0);
  }
  await assert.rejects(access(path.join(roots.data, 'constructor-ran')));
});

test('native probe detects deliberately removed OS policy', { skip: !enabled }, async () => {
  const roots = await fixture();
  const running = start(record(roots), [], weakenedLauncher);
  running.child.stdio[4].write('CXAWGO01');
  const result = await running.completed;
  assert.equal(result.code, 1, JSON.stringify(result));
  assert.match(result.stdout, /constructor_host_read_denied:FAIL/);
  assert.match(result.stdout, /package_write_denied:FAIL/);
  assert.match(result.stdout, /fork_denied:FAIL/);
});

test('idle startup control stream has a finite monotonic deadline', { skip: !enabled }, () => {
  const result = spawnSync(deadlineProbe, [], { timeout: 2000, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /bounded_monotonic_record_deadline:ok/);
});

test('overlapping roots and symlink roots are rejected before readiness', { skip: !enabled }, async () => {
  const roots = await fixture();
  const alias = path.join(path.dirname(roots.data), 'alias');
  await symlink(roots.data, alias);
  for (const changed of [{ ...roots, tmp: roots.data }, { ...roots, data: alias }]) {
    const result = await start(record(changed)).completed;
    assert.equal(result.code, 101, JSON.stringify(result));
    assert.equal(result.ready.length, 0);
  }
});

test('oversized and trailing launch payloads cannot reach readiness', { skip: !enabled }, async () => {
  const roots = await fixture();
  const oversized = record(roots).subarray(0, 12);
  oversized.writeUInt32BE(280001, 8);
  const trailing = Buffer.concat([record(roots), Buffer.from('extra')]);
  trailing.writeUInt32BE(trailing.length - 12, 8);
  for (const bytes of [oversized, trailing]) {
    const result = await start(bytes).completed;
    assert.equal(result.code, 100, JSON.stringify(result));
    assert.equal(result.ready.length, 0);
  }
});

test('App-supplied CLI switches and invalid continuation cannot load runtime', { skip: !enabled }, async () => {
  const roots = await fixture();
  assert.equal((await start(record(roots), ['--no-sandbox']).completed).code, 100);
  const running = start(record(roots));
  running.child.stdio[4].write('CXAWBAD!');
  const result = await running.completed;
  assert.equal(result.code, 105, JSON.stringify(result));
  await assert.rejects(access(path.join(roots.data, 'constructor-ran')));
});
