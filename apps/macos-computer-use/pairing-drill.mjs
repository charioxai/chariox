// macOS M1: the real kernel launches the installed ad-hoc helper through
// LaunchServices, pairs, reports health, then Stop, crash, local stop, kernel
// exit and identity denial are each observed and cleaned up. Identity mode only:
// no ScreenCaptureKit, AX, CGEvent or permission API runs.
// Usage: node apps/macos-computer-use/pairing-drill.mjs --kernel <chariox-kernel>
//          --root <disposable dir> --evidence <dir outside repo>
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../..');
const { values: args } = parseArgs({ options: { kernel: { type: 'string' }, root: { type: 'string' }, evidence: { type: 'string' } } });
for (const dir of [args.root, args.evidence]) assert.ok(dir && path.isAbsolute(dir) && !dir.startsWith(repo + path.sep), 'absolute dirs outside the repository');
const kernelBinary = path.resolve(args.kernel);
const root = args.root, home = path.join(root, 'home'), app = path.join(root, 'install', 'Chariox Computer Helper.app');
mkdirSync(home, { recursive: true, mode: 0o700 }); mkdirSync(args.evidence, { recursive: true, mode: 0o700 });
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const signing = target => execFileSync('/bin/sh', ['-c', 'codesign -dvvv "$1" 2>&1', 'sh', target], { encoding: 'utf8' });
const hashOf = target => signing(target).match(/^CDHash=([0-9a-f]{40})$/m)[1];
const freePort = async () => { const s = net.createServer(); await new Promise(r => s.listen(0, '127.0.0.1', r)); const p = s.address().port; await new Promise(r => s.close(r)); return p; };
const alive = pid => { try { process.kill(pid, 0); return true; } catch { return false; } };
const parentOf = pid => Number(execFileSync('ps', ['-o', 'ppid=', '-p', String(pid)], { encoding: 'utf8' }).trim());
const helperPids = () => { try { return execFileSync('pgrep', ['-f', app + '/Contents/MacOS/helper'], { encoding: 'utf8' }).trim().split('\n').filter(Boolean).map(Number); } catch { return []; } };
async function until(check, label, ms = 8000) { const end = Date.now() + ms; while (Date.now() < end) { if (await check()) return; await sleep(50); } throw new Error('timeout: ' + label); }

const kernelHash = hashOf(kernelBinary);
execFileSync('bash', [path.join(here, 'build.sh')], { env: { ...process.env, CUMAC_BUILD_DIR: path.join(root, 'install'), CHARIOX_DRILL_KERNEL_CDHASH: kernelHash }, stdio: 'ignore' });
const helperHash = hashOf(app);
const report = { items: ['macOS-M1'], kernel: kernelBinary, kernel_cdhash: kernelHash, helper: app, helper_cdhash: helperHash,
  source_commit: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repo, encoding: 'utf8' }).trim(), checks: [], status: 'FAIL', started_at: new Date().toISOString() };
let kernel, socket;

async function startKernel(allowlist) {
  const port = await freePort();
  const runtime = path.join(home, 'runtime');
  const env = { PATH: process.env.PATH, HOME: home, TMPDIR: root, CHARIOX_HOME: runtime, CHARIOX_LOG_DIR: path.join(root, 'logs'),
    CHARIOX_DAEMON_SOCKET: path.join(root, 'kernel.sock'), CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_PORT: String(await freePort()),
    CHARIOX_CODEX_PORT: String(await freePort()), CHARIOX_OPENCODE_PORT: String(await freePort()), CHARIOX_RELAY_URL: `ws://127.0.0.1:${await freePort()}`,
    CHARIOX_RELAY_TOKEN: 'disposable-cumac-m1', CHARIOX_DAEMON_ID: `cumac-m1-${process.pid}`, CHARIOX_DAEMON_ALIAS: 'cumac-m1', CHARIOX_PROVIDER_DEV_STUB: '1',
    CHARIOX_MACOS_COMPUTER_HELPER: app, CHARIOX_MACOS_COMPUTER_HELPER_DRILL_CDHASH: allowlist };
  kernel = spawn(kernelBinary, [], { cwd: home, env, stdio: 'ignore' });
  const tokenFile = path.join(runtime, 'state', 'kernel-local-auth', `${port}.token`);
  await until(() => existsSync(tokenFile), 'kernel auth token', 60000);
  await until(async () => { try { socket = await connect(port, readFileSync(tokenFile, 'utf8').trim()); return true; } catch { return false; } }, 'kernel socket', 30000);
  const temp = execFileSync('getconf', ['DARWIN_USER_TEMP_DIR'], { encoding: 'utf8' }).trim();
  const roots = () => readdirSync(temp).filter(name => name.startsWith('chariox-computer-')).map(name => path.join(temp, name));
  const before = new Set(roots());
  return () => roots().filter(dir => !before.has(dir));
}
function connect(port, token) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}`, { headers: { authorization: `Bearer ${token}` } });
    ws.pending = new Map(); ws.next = 0;
    ws.onmessage = event => { const frame = JSON.parse(event.data); ws.pending.get(frame.request_id)?.(frame); };
    ws.onopen = () => resolve(ws); ws.onerror = () => reject(new Error('connect failed'));
  });
}
function computer(command) {
  const id = `m1-${++socket.next}`;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('request timeout')), 30000);
    socket.pending.set(id, frame => { clearTimeout(timer); frame.response ? resolve(frame.response.KernelBrowser.result) : reject(new Error(JSON.stringify(frame.error ?? frame).slice(0, 300))); });
    const request = command.op === 'revoke_grants' ? { KernelBrowser: { command } } : { KernelBrowser: { command: { op: 'computer', command } } };
    socket.send(JSON.stringify({ type: 'request', request_id: id, command_id: id, request }));
  });
}
async function stopKernel() {
  socket?.close();
  if (kernel && kernel.exitCode === null) { const exited = new Promise(r => kernel.once('exit', r)); kernel.kill('SIGTERM'); await Promise.race([exited, sleep(10000)]); if (kernel.exitCode === null) kernel.kill('SIGKILL'); }
}
const check = (name, detail = {}) => report.checks.push({ name, ...detail });
const refused = async (command, pattern, label) => {
  const error = await computer(command).then(() => null, e => e.message);
  assert.ok(error && pattern.test(error), `${label}: ${error}`);
  return error;
};

try {
  let rendezvous = await startKernel(helperHash);
  await refused({ op: 'state' }, /explicitly start/, 'observation never starts the helper');
  assert.deepEqual(helperPids(), []); check('state-before-start-refused-no-launch');

  const first = await computer({ op: 'start' });
  assert.equal(first.mode, 'identity'); assert.equal(first.capture, 'unavailable');
  assert.ok(alive(first.pid) && parentOf(first.pid) === 1, 'LaunchServices helper outside the kernel tree');
  const [seatRoot] = rendezvous(); const [dir] = readdirSync(seatRoot);
  assert.deepEqual(readdirSync(path.join(seatRoot, dir)), [], 'one-use bootstrap and socket path removed');
  assert.equal(statSync(seatRoot).mode & 0o777, 0o700);
  assert.equal((await computer({ op: 'state' })).generation, first.generation);
  check('paired-health', { generation: first.generation, pid: first.pid, parent: 1, helper_signature: signing(app).match(/^Signature=.*$/m)?.[0] });
  await sleep(3000); assert.ok(alive(first.pid), 'heartbeat holds the lease'); check('lease-held-3s');

  await computer({ op: 'revoke_grants', agent_id: null });
  await until(() => !alive(first.pid), 'Stop ends the helper');
  await until(() => rendezvous().length === 0, 'Stop removes rendezvous');
  await refused({ op: 'state' }, /explicitly start/, 'stopped seat refuses observation'); check('kernel-stop-fences-and-cleans');

  const second = await computer({ op: 'start' });
  assert.notEqual(second.generation, first.generation);
  const stale = await refused({ op: 'snapshot', target: { surface_id: first.surface_id, generation: first.generation } }, /stale native desktop/, 'old epoch is stale');
  const unsupported = await refused({ op: 'snapshot', target: { surface_id: second.surface_id, generation: second.generation } }, /macOS Computer helper refused/, 'identity mode has no capture');
  assert.ok(alive(second.pid), 'refusals do not end the seat');
  check('fresh-epoch-and-stale-refusal', { generation: second.generation, stale, unsupported });

  process.kill(second.pid, 'SIGKILL');
  await until(() => rendezvous().length === 0, 'crash observed and rendezvous removed', 5000);
  await refused({ op: 'state' }, /explicitly start/, 'crashed seat needs explicit restart'); assert.deepEqual(helperPids(), []); check('helper-crash-observed-and-cleaned');

  const third = await computer({ op: 'start' });
  process.kill(third.pid, 'SIGTERM');
  await until(() => rendezvous().length === 0, 'local stop observed', 5000);
  await refused({ op: 'state' }, /explicitly start/, 'locally stopped seat'); check('local-helper-stop-observed');

  const fourth = await computer({ op: 'start' });
  await stopKernel();
  await until(() => !alive(fourth.pid), 'helper exits with its kernel', 5000); check('kernel-exit-ends-helper');

  // Denial: a kernel allowlisting another helper build refuses the real helper,
  // and the refused helper exits instead of lingering.
  rendezvous = await startKernel('0'.repeat(40));
  const denied = await refused({ op: 'start' }, /pairing refused/, 'foreign helper identity refused');
  await until(() => helperPids().length === 0, 'refused helper exits', 5000);
  assert.equal(rendezvous().length, 0); check('pairing-denied-for-foreign-helper-identity', { denied });
  report.status = 'PASS';
} catch (error) {
  report.failure = String(error?.message ?? error).slice(0, 2000); process.exitCode = 1;
} finally {
  await stopKernel();
  report.residue = helperPids();
  for (const pid of report.residue) process.kill(pid, 'SIGKILL');
  if (report.residue.length) { report.status = 'FAIL'; process.exitCode = 1; }
  rmSync(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  report.disposable_root_removed = !existsSync(root);
  report.finished_at = new Date().toISOString();
  writeFileSync(path.join(args.evidence, 'result.json'), JSON.stringify(report, null, 2) + '\n', { mode: 0o600 });
  console.log(`${report.status} macOS M1 helper pairing drill` + (report.failure ? `: ${report.failure}` : ''));
}
