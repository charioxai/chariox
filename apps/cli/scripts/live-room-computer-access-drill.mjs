// MP-08 / MP-10 / MP-11 (local 461): real kernel + real TUI in a PTY run `/access revoke all`
// then `/access grant all`; the kernel's shared grant snapshot must deny then restore every
// owned agent and the TUI must render both Room Computer notices. An omitted agent
// must be rejected while everyone remains denied. Dev-stub agents only:
// this is the protocol drill, not provider or desktop acceptance.
// Usage: node apps/cli/scripts/live-room-computer-access-drill.mjs --evidence <dir outside repo>
//          [--kernel target/debug/chariox-kernel] [--tui <compiled chariox binary>]
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn, execFileSync } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import { cp, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js';
import * as request from '../../../packages/kernel-client/dist/ipc-requests.js';
import { signalOwned } from '../../kernel/slice-linux-docker/docker/linux-owned-process.mjs';
import { verifyRoomComputerBulkAccess } from './lib/room-computer-access-drill.mjs';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const { values: args } = parseArgs({ options: { evidence: { type: 'string' }, kernel: { type: 'string' }, tui: { type: 'string' } } });
const evidence = path.resolve(args.evidence ?? '');
assert.ok(args.evidence && evidence !== repo && !evidence.startsWith(repo + path.sep), '--evidence must be outside the repository');
const kernelBinary = path.resolve(args.kernel ?? path.join(repo, 'target/debug/chariox-kernel'));
const tuiCommand = args.tui ? [path.resolve(args.tui)] : ['bun', path.join(repo, 'apps/cli/dist/index.js')];
await mkdir(evidence, { recursive: true, mode: 0o700 });

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const freePort = async () => { const s = net.createServer(); await new Promise(r => s.listen(0, '127.0.0.1', r)); const p = s.address().port; await new Promise(r => s.close(r)); return p; };
const quote = value => "'" + value.replaceAll("'", "'\\''") + "'";
async function waitFor(work, label, ms = 30000) {
  const end = Date.now() + ms; let error;
  while (Date.now() < end) { try { const value = await work(); if (value) return value; } catch (e) { error = e; } await sleep(250); }
  throw new Error(`timeout: ${label}${error ? '; ' + error.message : ''}`);
}
// Never signal 0/1/-1 or a non-child pid (coordinator process-signal rule).
async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1, 'refusing to signal an invalid pid');
  const exited = new Promise(resolve => child.once('exit', resolve));
  child.kill('SIGTERM');
  if (await Promise.race([exited.then(() => true), sleep(5000)])) return;
  child.kill('SIGKILL'); await exited;
}
// Processes carrying this run's disposable root; residue after shutdown is settled and reported.
async function runProcesses(settle = false) {
  const found = [];
  for (const pid of (await readdir('/proc')).filter(name => /^[0-9]+$/.test(name)).map(Number)) {
    if (!Number.isSafeInteger(pid) || pid <= 1 || pid === process.pid) continue;
    const environ = await readFile(`/proc/${pid}/environ`, 'latin1').catch(() => '');
    if (!environ.includes(root + path.sep)) continue;
    const stat = await readFile(`/proc/${pid}/stat`, 'latin1').catch(() => '');
    const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
    const ppid = Number(fields[1]);
    found.push({ pid, ppid, command: (await readFile(`/proc/${pid}/cmdline`, 'latin1').catch(() => '')).split('\0').slice(0, 3).join(' ') });
    if (settle) await signalOwned({ pid, parent: ppid, started: fields[19] }, 'SIGTERM');
  }
  return found;
}
function automation(socket, action, fields = {}) {
  return new Promise((resolve, reject) => {
    const c = net.createConnection(socket); let text = '';
    const timer = setTimeout(() => { c.destroy(); reject(new Error('automation timeout: ' + action)); }, 20000);
    c.once('connect', () => c.write(JSON.stringify({ id: 1, action, ...fields }) + '\n'));
    c.on('data', data => {
      text += data; if (!text.includes('\n')) return;
      clearTimeout(timer); c.end();
      try { const value = JSON.parse(text.split('\n')[0]); value.ok ? resolve(value.data) : reject(new Error(value.error)); } catch (e) { reject(e); }
    });
    c.once('error', e => { clearTimeout(timer); reject(e); });
  });
}

const root = await mkdtemp(path.join(os.tmpdir(), 'chariox-room-computer-access-'));
const home = path.join(root, 'home'), workspace = path.join(root, 'workspace'), socket = path.join(root, 'tui.sock');
const ptyLog = path.join(evidence, 'tui-pty.log');
let kernel, tui, client, kernelLog, tuiLog;
const hashFile = async file => createHash('sha256').update(await readFile(file)).digest('hex');
const report = { items: ['MP-08', 'MP-10', 'MP-11'], kernel: kernelBinary, tui: tuiCommand.join(' '),
  status: 'FAIL', started_at: new Date().toISOString(), resources: [] };
const resources = () => ({ at: new Date().toISOString(),
  memory_available_bytes: Number(execFileSync('awk', ['/MemAvailable/ {print $2}', '/proc/meminfo'], { encoding: 'utf8' }).trim()) * 1024,
  disk_available_bytes: Number(execFileSync('df', ['-B1', '--output=avail', '/'], { encoding: 'utf8' }).trim().split('\n').at(-1)) });
try {
  report.source_commit = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repo, encoding: 'utf8' }).trim();
  report.source_patch_sha256 = createHash('sha256').update(execFileSync('git', ['diff', 'HEAD'], { cwd: repo })).digest('hex');
  assert.equal(process.platform, 'linux', 'MP-08 / MP-10 / MP-11: Linux-only drill (requires /proc and util-linux script -c)');
  report.kernel_sha256 = await hashFile(kernelBinary);
  report.tui_sha256 = await hashFile(args.tui ? tuiCommand[0] : tuiCommand[1]);
  // Probe required tools before the host-dependent resource floor.
  execFileSync('script', ['-q', '-c', 'true', '/dev/null']);
  report.resources.push(resources());
  assert.ok(report.resources[0].memory_available_bytes >= 9 * 2 ** 30 && report.resources[0].disk_available_bytes >= 10 * 2 ** 30, 'MP-11 resource floor');
  await mkdir(home, { mode: 0o700 }); await mkdir(workspace);
  const port = await freePort();
  const env = {
    PATH: process.env.PATH, LANG: 'C.UTF-8', TERM: 'xterm-256color', HOME: home, TMPDIR: root,
    CHARIOX_HOME: path.join(home, 'runtime'), CHARIOX_LOG_DIR: path.join(root, 'logs'), CHARIOX_DAEMON_SOCKET: path.join(root, 'kernel.sock'),
    CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_PORT: String(await freePort()), CHARIOX_CODEX_PORT: String(await freePort()),
    CHARIOX_OPENCODE_PORT: String(await freePort()), CHARIOX_RELAY_URL: `ws://127.0.0.1:${await freePort()}`, CHARIOX_RELAY_TOKEN: 'disposable-room-computer-access',
    CHARIOX_DAEMON_ID: `room-computer-access-${process.pid}`, CHARIOX_DAEMON_ALIAS: 'room-computer-access', CHARIOX_PROVIDER_DEV_STUB: '1',
  };
  kernelLog = createWriteStream(path.join(evidence, 'kernel.log'), { mode: 0o600 });
  tuiLog = createWriteStream(ptyLog, { mode: 0o600 });
  kernel = spawn(kernelBinary, [], { cwd: workspace, env, stdio: ['ignore', 'pipe', 'pipe'] });
  kernel.stdout.pipe(kernelLog, { end: false }); kernel.stderr.pipe(kernelLog, { end: false });
  client = await waitFor(async () => {
    const c = new LocalIpcClient(`ws://127.0.0.1:${port}`, { localAuthEnvironment: env });
    try { await c.send({ ListSessions: null }); return c; } catch (e) { await c.close(); throw e; }
  }, 'kernel readiness');
  const one = (response, key) => { assert.ok(response?.[key], `MP-08 missing ${key}`); return response[key]; };
  const created = one(await client.send(request.createSessionRequest(workspace, workspace, 'room-computer-access', { provider: 'dev-stub', model: 'room-computer-access' })), 'SessionCreated');
  one(await client.send(request.spawnAgentRequest(created.session.id, 'dev-stub', 'second', 'room-computer-access')), 'AgentSpawned');
  const cli = [...tuiCommand, '--kernel-url', `ws://127.0.0.1:${port}`, '--session', created.session.id, '--provider', 'dev-stub', '--model', 'room-computer-access', '--automation-socket', socket,
    '--client-id', 'room-computer-access-drill', '--workspace', workspace, '--worktree', workspace];
  tui = spawn('script', ['-q', '-c', cli.map(quote).join(' '), '/dev/null'], { cwd: workspace, env, stdio: ['pipe', 'pipe', 'pipe'] });
  tui.stdout.pipe(tuiLog, { end: false }); tui.stderr.pipe(tuiLog, { end: false });
  await waitFor(() => automation(socket, 'wait_for', { sessionId: created.session.id, timeoutMs: 1000 }), 'TUI attach');
  const list = async () => one(await client.send(request.kernelBrowserRequest({ op: 'list_grants' })), 'KernelBrowser').result;
  // Access notices are TUI output, not transcript entries: read the PTY stream with ANSI stripped.
  const readTuiText = async () => (await readFile(ptyLog, 'utf8')).replace(/\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07|\x1b[()][A-Z0-9]/g, '').replace(/\s+/g, ' ');
  report.checks = await verifyRoomComputerBulkAccess({
    submitCommand: prompt => automation(socket, 'submit_prompt', { prompt }),
    snapshot: list, readTuiText,
    captureEvidence: async (label, result) => {
      await writeFile(path.join(evidence, label + '-grants.json'), JSON.stringify(result, null, 2) + '\n', { mode: 0o600 });
      if (label === 'bulk-revoke') {
        // MP-11: malformed input cannot undo the owner's real TUI revocation.
        await assert.rejects(
          client.send(request.kernelBrowserRequest({ op: 'grant_room_computer' })),
          error => error.name === 'LocalIpcError' && error.code === 'invalid_request'
            && /invalid request: missing field `agent_id`/.test(error.message),
          'MP-11 missing agent must be rejected at request decoding',
        );
        const after = await list();
        assert.deepEqual(after.room_computer, result.room_computer, 'MP-11 malformed restore preserves denials');
        report.missing_agent_rejected = true;
      }
    },
    waitFor,
  });
  report.status = 'PASS';
} catch (error) {
  report.failure = error.message.slice(0, 2000);
  process.exitCode = 1;
} finally {
  if (tui) await automation(socket, 'exit').catch(() => {});
  await waitFor(async () => tui?.exitCode !== null, 'TUI exit', 5000).catch(() => {});
  await stop(tui);
  // The kernel may prewarm provider servers; tear them down through the product before stopping it.
  await client?.send(request.teardownProviderProcessesRequest(null, true)).catch(() => {});
  report.processes_before_kernel_stop = kernel || tui ? await runProcesses() : [];
  await client?.close().catch(() => {});
  await stop(kernel);
  kernelLog?.end(); tuiLog?.end();
  // Known kernel issue: a codex app-server can start during kernel shutdown and outlive it.
  report.residue_settled = kernel || tui ? await runProcesses(true) : [];
  if (kernel || tui) await sleep(2000);
  report.residue_after_settle = kernel || tui ? await runProcesses() : [];
  if (report.residue_after_settle.length) { report.status = 'FAIL'; report.failure ??= 'drill processes survived cleanup'; process.exitCode = 1; }
  await cp(path.join(root, 'logs'), path.join(evidence, 'kernel-logs'), { recursive: true }).catch(() => {});
  await rm(root, { recursive: true, force: true });
  report.cleanup = kernel || tui
    ? 'TUI exited through automation; providers torn down; kernel stopped; residue audited; disposable state removed.'
    : 'Preflight failed before runtime launch; disposable state removed.';
  if (process.platform === 'linux') {
    try { report.resources.push(resources()); } catch (error) { report.resource_sample_failure = error.message.slice(0, 2000); }
  }
  report.finished_at = new Date().toISOString();
  await writeFile(path.join(evidence, 'result.json'), JSON.stringify(report, null, 2) + '\n', { mode: 0o600 });
  console.log(`${report.status} room computer bulk access drill` + (report.failure ? `: ${report.failure}` : '')
    + (report.residue_settled.length ? `; settled ${report.residue_settled.length} leftover process(es)` : ''));
}
