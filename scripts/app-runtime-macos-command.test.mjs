import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { EventEmitter } from 'node:events';
import { mkdtemp, readFile, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import test from 'node:test';
import { createMacBuildSession } from './app-runtime-macos-build.mjs';
import { ownedGroupSignaler, runMacCommand } from './app-runtime-macos-command.mjs';
import { processGroup } from './app-runtime-macos-preflight.mjs';

const profile = JSON.parse(await readFile(new URL('../apps/app-worker/macos-build-profile.json', import.meta.url), 'utf8'));
const python = execFileSync('python3', ['-c', 'import sys;print(sys.executable)'], { encoding: 'utf8', timeout: 5000 }).trim();
const healthy = { availableMemoryBytes: 2 * 1024 ** 3, freeDiskBytes: 8 * 1024 ** 3,
  groupRssBytes: 1024, groupProcesses: 1, swapBytes: 0 };

async function observedGroup(pid) {
  const group = processGroup(execFileSync('/bin/ps', ['-axo', 'pid=,ppid=,pgid=,rss='], { encoding: 'utf8', timeout: 2000, maxBuffer: 1024 * 1024 }), pid);
  return { ...healthy, groupProcesses: group.processCount, groupRssBytes: group.rssBytes };
}

async function fixture(t) {
  const scratch = await realpath(await mkdtemp(join(tmpdir(), 'chariox-macos-command-')));
  t.after(() => rm(scratch, { recursive: true, force: true }));
  return { plan: { scratch, tools: { python }, ciProfile: profile }, session: createMacBuildSession(profile, 0),
    command: code => ({ program: process.execPath, args: ['-e', code], cwd: scratch }) };
}

test('fixed command owner acknowledges success and leaves no owned process behind', async t => {
  const f = await fixture(t);
  await runMacCommand(f.command('setTimeout(()=>{},100)'), process.env, f.plan, f.session, async () => healthy);
  assert.ok(f.session.evidence.samples >= 1);
});

test('guard cancellation terminates and reaps the retained command group', async t => {
  const f = await fixture(t);
  const before = performance.now();
  await assert.rejects(runMacCommand(f.command('setInterval(()=>{},1000)'), process.env, f.plan, f.session,
    async () => ({ ...healthy, groupRssBytes: 5 * 1024 ** 3 })), /group-memory/);
  assert.ok(performance.now() - before < 5000);
});

test('failed command descendants stop before scratch can be removed', async t => {
  const f = await fixture(t); const heartbeat = join(f.plan.scratch, 'heartbeat');
  const descendant = `const fs=require('node:fs');let i=0;setInterval(()=>fs.writeFileSync(process.argv[1],String(++i)),10);process.stdout.write('ready');`;
  const leader = `const c=require('node:child_process').spawn(process.execPath,['-e',${JSON.stringify(descendant)},${JSON.stringify(heartbeat)}],{stdio:['ignore','pipe','ignore']});c.stdout.once('data',()=>setTimeout(()=>process.exit(7),50));`;
  await assert.rejects(runMacCommand(f.command(leader), process.env, f.plan, f.session, async () => healthy), /command-failed/);
  const value = await readFile(heartbeat, 'utf8'); await delay(100);
  assert.equal(await readFile(heartbeat, 'utf8'), value);
});

test('explicit cancellation and elapsed global deadline cannot admit a command', async t => {
  const f = await fixture(t); const controller = new AbortController(); f.session.signal = controller.signal;
  const running = runMacCommand(f.command('setInterval(()=>{},1000)'), process.env, f.plan, f.session, async () => healthy);
  controller.abort(); await assert.rejects(running, /interrupted/);
  f.session.deadline = performance.now() - 1;
  await assert.rejects(runMacCommand(f.command('throw new Error("must not execute")'), process.env, f.plan, f.session, async () => healthy), /deadline/);
});

test('a successful command cannot leave a background descendant running', async t => {
  const f = await fixture(t); const heartbeat = join(f.plan.scratch, 'heartbeat');
  const descendant = `const fs=require('node:fs');let i=0;setInterval(()=>fs.writeFileSync(process.argv[1],String(++i)),10);process.stdout.write('ready');`;
  const leader = `const c=require('node:child_process').spawn(process.execPath,['-e',${JSON.stringify(descendant)},${JSON.stringify(heartbeat)}],{stdio:['ignore','pipe','ignore']});c.stdout.once('data',()=>setTimeout(()=>process.exit(0),50));`;
  await assert.rejects(runMacCommand(f.command(leader), process.env, f.plan, f.session, observedGroup), /surviving-descendants/);
  const value = await readFile(heartbeat, 'utf8'); await delay(100);
  assert.equal(await readFile(heartbeat, 'utf8'), value);
});

test('exit withdraws signal authority before delayed stdio close', () => {
  const child = new EventEmitter(); child.pid = 123;
  const calls = []; const signal = ownedGroupSignaler(child, (...args) => calls.push(args));
  signal('SIGTERM'); assert.deepEqual(calls, [[-123, 'SIGTERM']]);
  child.emit('exit', 0); signal('SIGKILL'); signal('SIGTERM');
  assert.equal(calls.length, 1);
  child.emit('close', 0); signal('SIGKILL'); assert.equal(calls.length, 1);
});

test('pre-aborted admission does not start a command that would write a file', async t => {
  const f = await fixture(t); const path = join(f.plan.scratch, 'must-not-exist');
  const controller = new AbortController(); controller.abort(); f.session.signal = controller.signal;
  await assert.rejects(runMacCommand(f.command(`require('node:fs').writeFileSync(${JSON.stringify(path)},'wrong')`),
    process.env, f.plan, f.session, async () => healthy), /interrupted/);
  await assert.rejects(readFile(path), { code: 'ENOENT' });
});
