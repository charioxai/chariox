// MP-08 / MP-09 / MP-10 / MP-11: the holder's detached exec through the public lanes-kernel.mjs command.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, readFile, mkdir } from 'node:fs/promises';
import { existsSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';

const script = new URL('./lanes-kernel.mjs', import.meta.url).pathname;

async function fakeAccessCli(state, subscription = { subscription_id: 'live-grant', ended: null }) {
  await mkdir(join(state, 'bin'), { recursive: true });
  await writeFile(join(state, 'bin/chariox'), `#!${process.execPath}
import { writeFileSync } from 'node:fs';
const args = process.argv.slice(2);
if (args[0] === 'access' && args[1] === 'status') console.log(JSON.stringify({ status: 'granted' }));
else if (args[0] === 'events' && args[1] === 'subscription') console.log(JSON.stringify({ subscription: ${JSON.stringify(subscription)} }));
else if (args[0] === 'access' && args[1] === 'request') {
  writeFileSync(${JSON.stringify(join(state, 'replacement-requested'))}, 'yes');
  console.error('owner approval required'); process.exitCode = 1;
} else process.exitCode = 2;
`, { mode: 0o700 });
}

async function holder(t, extraConfig = {}, seat, launchEnvironment, legacy = false) {
  const state = await mkdtemp(join(tmpdir(), 'lanes-kernel-')); t.after(() => rm(state, { recursive: true, force: true }));
  await writeFile(join(state, 'config.json'), JSON.stringify({ toolchain: state, ...extraConfig }));
  await fakeAccessCli(state);
  if (launchEnvironment) await writeFile(join(state, 'kernel-environment.json'), JSON.stringify(launchEnvironment));
  const env = { PATH: process.env.PATH, HOME: state, LANES_KERNEL_STATE: state };
  let holderScript = script;
  if (legacy) {
    holderScript = join(state, 'lanes-kernel.mjs');
    await writeFile(holderScript, `import { createServer } from 'node:net';
import { spawn } from 'node:child_process';
import { openSync, writeFileSync } from 'node:fs';
const state = process.env.LANES_KERNEL_STATE;
createServer(socket => {
 let buffer = '';
 socket.on('data', chunk => {
  buffer += chunk; if (!buffer.includes('\\n')) return;
  const request = JSON.parse(buffer.split('\\n')[0]);
  if (!request.detach) {
    const child = spawn(request.argv[0], request.argv.slice(1), { cwd: request.cwd, env: { ...process.env, ...request.env }, stdio: ['ignore', 'pipe', 'pipe'] });
    child.stdout.on('data', data => socket.write(JSON.stringify({ out: data.toString('base64') }) + '\\n'));
    child.stderr.on('data', data => socket.write(JSON.stringify({ err: data.toString('base64') }) + '\\n'));
    child.on('close', code => socket.end(JSON.stringify({ exit: code }) + '\\n'));
    return;
  }
  // The already-running holder knows only one combined log file.
  const log = openSync(request.logFile, 'a');
  const child = spawn(request.argv[0], request.argv.slice(1), { cwd: request.cwd, env: { ...process.env, ...request.env }, stdio: ['ignore', log, log] });
  child.on('spawn', () => socket.end(JSON.stringify({ pid: child.pid, exit: 0 }) + '\\n'));
 });
}).listen(state + '/holder.sock', () => writeFileSync(state + '/holder.pid', String(process.pid)));\n`);
  }
  const child = spawn(process.execPath, [holderScript, 'holder', ...(seat ? [seat] : [])], { env, stdio: 'ignore' });
  const exited = once(child, 'exit');
  t.after(async () => { if (Number.isSafeInteger(child.pid) && child.pid > 1 && child.exitCode === null) child.kill('SIGTERM'); await exited; });
  const pidFile = join(state, seat ? `${seat}-holder.pid` : 'holder.pid');
  for (let i = 0; i < 100 && !existsSync(pidFile); i++) await new Promise(resolve => setTimeout(resolve, 50));
  assert.ok(existsSync(pidFile), 'the holder is listening');
  return { state, env, child, exec: (...args) => spawnSync(process.execPath, [script, 'exec', ...(seat ? ['--seat', seat] : []), ...args], { env, encoding: 'utf8' }) };
}

for (const seat of [undefined, 'coordinator']) for (const subscription of [null, { subscription_id: 'old-grant', ended: 'grant_ended' }]) test(`grant verifies the live event subscription before reusing a recorded approval (${seat ?? 'loopd'}, ${subscription ? 'ended' : 'missing'})`, async t => {
  const { state, env } = await holder(t, {}, seat);
  await fakeAccessCli(state, subscription);
  const result = spawnSync(process.execPath, [script, 'grant', ...(seat ? [seat] : [])], { env, encoding: 'utf8' });
  assert.notEqual(result.status, 0, 'a recorded approval with an ended inbox is not a live grant');
  assert.ok(existsSync(join(state, 'replacement-requested')), 'the existing access request path attempts replacement');
  assert.match(result.stderr, subscription ? /grant_ended/ : /no_subscription/);
  assert.match(result.stderr, /owner|approve/i);
});

test('grant reuses an approval only with a live subscription', async t => {
  const { state, env } = await holder(t);
  const result = spawnSync(process.execPath, [script, 'grant'], { env, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(JSON.parse(result.stdout).granted, 'existing');
  assert.equal(existsSync(join(state, 'replacement-requested')), false);
});

test('loopd start surfaces an ended grant instead of reporting a detached service as started', async t => {
  const { state, env } = await holder(t);
  const directory = join(state, 'run/tools/lanes/coordinator');
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, 'round.mjs'), 'process.exit(1);\n');
  await fakeAccessCli(state, { subscription_id: 'old-grant', ended: 'grant_ended' });
  const result = spawnSync(process.execPath, [script, 'loopd', 'start'], { env, encoding: 'utf8' });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /grant_ended/);
  assert.match(result.stderr, /start|grant/i);
});

test('exec --detach exits 0 with the pid of a command that started, and keeps its log', async t => {
  const { state, exec } = await holder(t);
  const log = join(state, 'job.log');
  const result = exec('--detach', '--log', log, '--', '/bin/sh', '-c', 'echo detached-output');
  assert.equal(result.status, 0, result.stderr);
  const { pid } = JSON.parse(result.stdout);
  assert.ok(Number.isSafeInteger(pid) && pid > 1);
  for (let i = 0; i < 100 && !readFileSync(log, 'utf8').includes('detached-output'); i++) await new Promise(resolve => setTimeout(resolve, 50));
  assert.match(await readFile(log, 'utf8'), /detached-output/);
});

test('exec --detach fails with the reason when the command cannot start', async t => {
  const { exec } = await holder(t);
  const result = exec('--detach', '--', '/no/such/binary');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /ENOENT/);
  assert.equal(result.stdout.includes('"pid"'), false);
});

for (const legacy of [false, true]) test(`loopd keeps stderr out of its JSON log with ${legacy ? 'an existing' : 'a new'} holder`, async t => {
  const { state, env } = await holder(t, {}, undefined, undefined, legacy);
  const entry = join(state, 'run/tools/lanes/coordinator/round.mjs');
  await mkdir(join(state, 'run/tools/lanes/coordinator'), { recursive: true });
  await writeFile(entry, `console.log(JSON.stringify({ kind: 'started' }));
process.emitWarning('loopd warning regression');
console.error('loopd stderr regression');
setTimeout(() => console.log(JSON.stringify({ kind: 'stopped' })), 100);\n`);
  const result = spawnSync(process.execPath, [script, 'loopd', 'start'], { env, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  const log = join(state, 'loopd.log');
  for (let i = 0; i < 100 && !readFileSync(log, 'utf8').includes('stopped'); i++) await new Promise(resolve => setTimeout(resolve, 50));
  const output = await readFile(log, 'utf8');
  assert.equal(output.includes('regression'), false, 'stdout contains JSON events only');
  assert.deepEqual(output.trim().split('\n').map(JSON.parse).map(event => event.kind), ['started', 'stopped']);
  const diagnostics = await readFile(join(state, 'loopd.stderr.log'), 'utf8');
  assert.match(diagnostics, /loopd warning regression/);
  assert.match(diagnostics, /loopd stderr regression/);
});

test('a state dir with its own agents directory hands it to the commands it runs, so a drill loopd cannot read the real queue', async t => {
  const { exec } = await holder(t, { agents: '/drill/agents' });
  const result = exec('--', '/bin/sh', '-c', 'echo "$LOOPD_AGENTS"');
  assert.equal(result.stdout.trim(), '/drill/agents');
});

test('commands under the holder get the standard root PATH, sbin directories included', async t => {
  const { exec } = await holder(t);
  const result = exec('--', '/bin/sh', '-c', 'echo "$PATH"');
  const path = result.stdout.trim().split(':');
  for (const directory of ['/usr/sbin', '/sbin', '/usr/local/sbin']) assert.ok(path.includes(directory), `${directory} is on PATH`);
});

test('the coordinator seat is a second holder with its own socket, pid and LANEWAIT_HOME', async t => {
  const { state, exec } = await holder(t, {}, 'coordinator');
  const result = exec('--', '/bin/sh', '-c', 'echo "$LANEWAIT_HOME $CHARIOX_CLI_ACCESS_HOLDER_PID"');
  const [home, pid] = result.stdout.trim().split(' ');
  assert.equal(home, join(state, 'watch'));
  assert.equal(pid, readFileSync(join(state, 'coordinator-holder.pid'), 'utf8').trim());
  assert.equal(existsSync(join(state, 'holder.pid')), false, 'the loopd seat is not touched');
});

test('an unknown seat is refused by name', async t => {
  const { env } = await holder(t);
  const result = spawnSync(process.execPath, [script, 'exec', '--seat', 'nobody', '--', '/bin/true'], { env, encoding: 'utf8' });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /unknown seat nobody/);
});

test('a command whose client went away is stopped by the holder', async t => {
  const { state, env } = await holder(t, {}, 'coordinator');
  const marker = join(state, 'child.pid');
  const client = spawn(process.execPath, [script, 'exec', '--seat', 'coordinator', '--', '/bin/sh', '-c', `echo $$ > ${marker}; exec sleep 300`], { env, stdio: 'ignore' });
  for (let i = 0; i < 100 && !existsSync(marker); i++) await new Promise(resolve => setTimeout(resolve, 50));
  const pid = Number(readFileSync(marker, 'utf8'));
  assert.ok(pid > 1 && !Number.isNaN(pid));
  client.kill('SIGKILL');
  let alive = true;
  for (let i = 0; i < 100 && alive; i++) { await new Promise(resolve => setTimeout(resolve, 50)); try { process.kill(pid, 0); } catch { alive = false; } }
  assert.equal(alive, false, 'the orphaned command is gone');
});

test('only IS_SANDBOX=1 in the launch environment reaches worker processes', async t => {
  for (const value of ['1', '0', 'true', '']) {
    const { exec } = await holder(t, {}, undefined, { IS_SANDBOX: value });
    const result = exec('--', '/bin/sh', '-c', 'printf "%s" "${IS_SANDBOX-unset}"');
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, value === '1' ? '1' : 'unset');
  }
});

test('set-claude-credential refuses --profile without --from before touching any kernel', async t => {
  const state = await mkdtemp(join(tmpdir(), 'lanes-kernel-')); t.after(() => rm(state, { recursive: true, force: true }));
  await writeFile(join(state, 'config.json'), JSON.stringify({ toolchain: state }));
  const run = (...args) => spawnSync(process.execPath, [script, 'set-claude-credential', ...args], { env: { PATH: process.env.PATH, HOME: state, LANES_KERNEL_STATE: state }, encoding: 'utf8' });
  const result = run('--profile', 'lanes-claude-old');
  assert.equal(result.status, 2);
  assert.match(result.stderr, /--profile needs --from <token-file>/);
  assert.match(run('--profile', '../x', '--from', '/t').stderr, /is not a profile label/);
  assert.match(run('--from', '/t').stderr, /executor kernel is not running/, 'the unchanged form still goes to the kernel check');
});
