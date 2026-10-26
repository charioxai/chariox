#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11: the lasting dev lanes kernel (executor) and its access kernel (coordinator).
// Usage: lanes-kernel.mjs setup|start|stop|status|grant|exec|env|set-claude-credential  (see docs/LANES_KERNEL.md)
import { spawn, spawnSync } from 'node:child_process';
import { createServer, connect } from 'node:net';
import { mkdirSync, writeFileSync, readFileSync, existsSync, openSync, readlinkSync, chmodSync, unlinkSync, copyFileSync, rmSync, renameSync, readdirSync } from 'node:fs';
import { randomBytes, createHash } from 'node:crypto';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { serviceUnit, sliceInvocation } from './service.mjs';
import { inspectWorker } from './worker-recovery.mjs';
import { restartWithSandbox } from './restart.mjs';
import { syncWorkerAgents, setWorkerProfileDefault } from './worker-agents.mjs';
import { probeClaudeToken, readToken, replaceClaudeCredential } from './claude-account.mjs';
import { parseCredentialArgs, storeClaudeProfile } from './claude-profile.mjs';

const STATE = process.env.LANES_KERNEL_STATE || '/root/.chariox/dev/lanes-kernel';
// Put the launcher, both kernels, holders and loopd into the shared lane cgroup together.
if (process.platform === 'linux' && ['setup', 'start', 'restart-yolo'].includes(process.argv[2])) {
  const slice = spawnSync('systemctl', ['show', 'lanes.slice', '--property=LoadState', '--value'], { encoding: 'utf8' });
  const invocation = sliceInvocation({ sliceExists: slice.status === 0 && slice.stdout.trim() === 'loaded',
    cgroup: readFileSync('/proc/self/cgroup', 'utf8'), node: process.execPath, script: process.argv[1], args: process.argv.slice(2) });
  if (invocation) {
    const result = spawnSync(invocation[0], invocation.slice(1), { stdio: 'inherit' });
    process.exit(result.status ?? 1);
  }
}
if (process.argv[2] === 'install-service') {
  const unit = serviceUnit({ node: process.execPath, script: new URL(import.meta.url).pathname, state: STATE });
  writeFileSync('/etc/systemd/system/chariox-lanes.service', unit, { mode: 0o644 });
  writeFileSync('/etc/systemd/system/lanes.slice', readFileSync(new URL('./lanes.slice', import.meta.url), 'utf8'), { mode: 0o644 });
  for (const args of [['daemon-reload'], ['enable', 'chariox-lanes.service']]) {
    const result = spawnSync('systemctl', args, { stdio: 'inherit' });
    if (result.status !== 0) throw new Error(`systemctl ${args[0]} failed; inspect chariox-lanes.service`);
  }
  console.log('Installed and enabled chariox-lanes.service and lanes.slice for boot; existing workers were not restarted.');
  process.exit(0);
}
const config = JSON.parse(readFileSync(join(STATE, 'config.json'), 'utf8'));
const ROLES = ['executor', 'coordinator'];
const dir = role => join(STATE, role);
const file = (role, name) => join(dir(role), name);
const readText = path => readFileSync(path, 'utf8').trim();
const out = value => console.log(JSON.stringify(value));

function pidAlive(pid) { try { process.kill(pid, 0); return true; } catch (error) { return error.code === 'EPERM'; } }
// Only act on a pid whose executable and kernel home both match what this script started.
function ownedPid(role, pidFile, executable) {
  if (!existsSync(pidFile)) return undefined;
  const pid = Number(readText(pidFile));
  if (!Number.isSafeInteger(pid) || pid <= 1 || !pidAlive(pid)) return undefined;
  try {
    if (readlinkSync(`/proc/${pid}/exe`) !== executable) return undefined;
    if (!readFileSync(`/proc/${pid}/environ`, 'utf8').split('\0').includes(`CHARIOX_HOME=${file(role, 'chariox')}`)) return undefined;
  } catch { return undefined; }
  return pid;
}
const kernelBinary = join(STATE, 'bin', 'chariox-kernel');
const kernelPid = role => ownedPid(role, file(role, 'kernel.pid'), kernelBinary);

function roleEnv(role) {
  const root = dir(role);
  const launchEnvironment = join(STATE, 'kernel-environment.json');
  const sandbox = existsSync(launchEnvironment) ? JSON.parse(readFileSync(launchEnvironment, 'utf8')).IS_SANDBOX : process.env.IS_SANDBOX;
  return {
    // The sbin directories are part of the standard root PATH: workers run ufw, ip, iptables and the like.
    PATH: `${config.toolchain}/cargo/bin:/root/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`,
    HOME: join(root, 'home'), SHELL: '/bin/bash', TMPDIR: join(root, 'tmp'),
    CARGO_HOME: `${config.toolchain}/cargo`, RUSTUP_HOME: `${config.toolchain}/rustup`,
    XDG_CONFIG_HOME: join(root, 'xdg-config'), XDG_STATE_HOME: join(root, 'xdg-state'), XDG_CACHE_HOME: join(root, 'xdg-cache'),
    CHARIOX_HOME: file(role, 'chariox'), CHARIOX_LOG_DIR: join(root, 'logs'), CHARIOX_SESSION_HISTORY_DIR: join(root, 'history'),
    CHARIOX_DAEMON_SOCKET: join(root, 'daemon.sock'), CHARIOX_DAEMON_ALIAS: `lanes-${role}`, CHARIOX_MACHINE_ALIAS: `lanes-${role}`,
    NODE_OPTIONS: '--max-old-space-size=4096',
    ...(sandbox === '1' && { IS_SANDBOX: '1' }),
  };
}

let ipc;
async function client(role) {
  ipc ??= await import(join(config.runtime.worktree, 'packages/kernel-client/dist/ipc.js'));
  const port = config.ports[role];
  return new ipc.LocalIpcClient(`ws://127.0.0.1:${port}/kernel`, { localAuthEnvironment: roleEnv(role) });
}
async function rpc(role, request) {
  const c = await client(role);
  try {
    const result = await c.send(request);
    if (result.Error) throw new Error(`kernel refused ${Object.keys(request)[0]}: ${JSON.stringify(result.Error).slice(0, 160)}`);
    return result;
  } finally { await c.close?.().catch?.(() => {}); }
}
async function relayStatus(role, attempts = 60) {
  for (let i = 0; i < attempts; i++) {
    try { return (await rpc(role, { RelayStatus: null })).RelayStatus.status; } catch { await delay(500); }
  }
  throw new Error(`${role} kernel is not reachable`);
}

function prepare(role) {
  for (const name of ['chariox', 'home', 'xdg-config', 'xdg-state', 'xdg-cache', 'logs', 'history', 'tmp']) mkdirSync(file(role, name), { recursive: true, mode: 0o700 });
  const home = file(role, 'chariox');
  if (!existsSync(join(home, 'config.toml'))) {
    writeFileSync(join(home, 'config.toml'), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${join(home, 'vault.json')}"\nunlock_policy = "kernel_init"\n[kernel_access]\nrequest_timeout_minutes = 10\n`, { mode: 0o600 });
  }
  if (!existsSync(file(role, 'passphrase'))) writeFileSync(file(role, 'passphrase'), randomBytes(24).toString('hex'), { mode: 0o600 });
  const gitconfig = join(dir(role), 'home', '.gitconfig');
  if (role === 'executor' && !existsSync(gitconfig) && existsSync('/root/.gitconfig')) copyFileSync('/root/.gitconfig', gitconfig);
}

async function startKernel(role) {
  prepare(role);
  const existing = kernelPid(role);
  if (existing) return { role, pid: existing, started: false };
  if (!existsSync(kernelBinary)) throw new Error('kernel binary missing; see docs/LANES_KERNEL.md');
  const port = config.ports[role];
  const log = openSync(file(role, 'logs/kernel.out'), 'a');
  const child = spawn(kernelBinary, [], {
    env: { ...roleEnv(role), CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_PORT: String(port + 1), CHARIOX_OPENCODE_PORT: String(port + 2), CHARIOX_CODEX_PORT: String(port + 3) },
    detached: true, stdio: ['ignore', log, log],
  });
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error('invalid kernel pid');
  writeFileSync(file(role, 'kernel.pid'), String(child.pid));
  writeFileSync(file(role, 'kernel-started-at'), String(Date.now())); child.unref();
  const status = await relayStatus(role);
  writeFileSync(file(role, 'ids.json'), JSON.stringify({ kernel_id: status.daemon_id, machine_id: status.machine_id }));
  return { role, pid: child.pid, started: true, kernel_id: status.daemon_id };
}

async function stopKernel(role) {
  const pid = kernelPid(role);
  if (!pid) return { role, stopped: false, reason: 'not running' };
  process.kill(pid, 'SIGTERM');
  for (let i = 0; i < 60 && pidAlive(pid); i++) await delay(500);
  if (pidAlive(pid)) throw new Error(`${role} kernel ${pid} did not exit after SIGTERM; inspect it before stopping it by hand`);
  return { role, stopped: true, pid };
}

// The Vault relocks on every kernel restart; its disposable passphrase never leaves this state directory.
async function ensureVault(role) {
  const status = (await rpc(role, { GetCredentialVaultStatus: null }));
  const body = status.CredentialVaultStatus?.status ?? status.CredentialVaultStatus ?? status;
  if (body.unlocked === true || body.state === 'unlocked' || body.locked === false) return { role, vault: 'unlocked' };
  // A restored session cannot answer a new unlock decision; each unlock asks in a fresh owner session.
  const workspace = join(dir(role), 'workspace');
  mkdirSync(workspace, { recursive: true, mode: 0o700 });
  const made = (await rpc(role, { CreateSession: { workspace_id: workspace, worktree_id: workspace, alias: `lanes-${role}-unlock-${Date.now()}` } })).SessionCreated;
  const { session, agent } = made;
  if (!existsSync(file(role, 'session.id'))) { writeFileSync(file(role, 'session.id'), session.id); writeFileSync(file(role, 'agent.id'), agent.id); }
  const pending = rpc(role, { ManageCredentialVault: { session_id: session.id, agent_id: agent.id } });
  pending.catch(() => {});
  let prompt;
  for (let i = 0; i < 80 && !prompt; i++) {
    const state = (await rpc(role, { GetSessionState: { session_id: session.id } })).SessionState.session;
    prompt = state.active_interactions?.find(item => item.id.startsWith('vault-unlock-'));
    if (!prompt) await delay(100);
  }
  if (!prompt) throw new Error('vault prompt missing');
  await rpc(role, { RespondToInteraction: { session_id: session.id, interaction_id: prompt.id, choice_id: 'passphrase', custom_reply: readText(file(role, 'passphrase')) } });
  await pending;
  return { role, vault: 'unlocked-now' };
}

async function enroll(role) {
  const status = await relayStatus(role);
  const meta = JSON.parse(readFileSync(`/w/valenv/prs/${config.stack}/metadata.json`, 'utf8'));
  const origin = `https://${meta.hostname}`, relayUrl = `wss://${meta.relay_hostname}`;
  if (status.connected && status.relay_url === relayUrl) return { role, relay: 'connected' };
  if (!status.connected && existsSync(file(role, 'enrolled'))) {
    await rpc(role, { ConnectCloudRelay: null });
  } else {
    const { chromium } = await import('/w/valenv/tools/node_modules/playwright-core/index.mjs');
    const login = (await rpc(role, { StartCloudRelayLogin: { api_url: origin, client_id: 'lanes-kernel', client_alias: `lanes-${role}`, machine_id: status.machine_id, machine_alias: `lanes-${role}` } })).CloudRelayLoginStarted.login;
    if (new URL(login.verification_url).origin !== origin) throw new Error('verification origin mismatch');
    const browser = await chromium.launch({ executablePath: '/opt/google/chrome/chrome', headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
    try {
      const page = await (await browser.newContext()).newPage();
      await page.goto(login.verification_url);
      let approved = false;
      for (let i = 0; i < 30 && !approved; i++) {
        const button = page.getByRole('button', { name: /approve|allow|connect/i });
        if (await button.count() === 1 && await button.isVisible()) await button.click();
        await delay(Math.max(5000, (login.interval_seconds ?? 5) * 1000));
        approved = (await rpc(role, { PollCloudRelayLogin: { api_url: origin, device_code: login.device_code } })).CloudRelayLoginPolled.result.status === 'approved';
      }
      if (!approved) throw new Error('device login was not approved');
    } finally { await browser.close(); }
    await rpc(role, { ConnectCloudRelay: null });
    writeFileSync(file(role, 'enrolled'), new Date().toISOString());
  }
  for (let i = 0; i < 120; i++) {
    const s = await relayStatus(role, 1).catch(() => ({}));
    if (s.connected && s.relay_url === relayUrl) return { role, relay: 'connected', kernel_id: s.daemon_id };
    await delay(500);
  }
  throw new Error(`${role} relay did not connect over ${relayUrl}`);
}

// The existing lanes Claude setup token is stored through the product, fed from its file and never printed.
async function ensureClaudeProfile() {
  const profiles = async () => (await rpc('executor', { ListProviderAccountProfiles: { provider: 'claude' } })).ProviderAccountProfilesListed.profiles;
  let profile = (await profiles()).find(p => p.label === 'lanes-claude' && p.origin === 'chariox_created');
  if (!profile) {
    await rpc('executor', { CreateProviderAccountProfile: { provider: 'claude', label: 'lanes-claude' } });
    profile = (await profiles()).find(p => p.label === 'lanes-claude' && p.origin === 'chariox_created');
  }
  if (!profile) throw new Error('lanes-claude profile was not created');
  const authStatus = async () => (await rpc('executor', { GetProviderAuthStatus: { provider: 'claude', account_profile: profile.profile_id } }));
  let auth = JSON.stringify(await authStatus());
  if (!/authenticated/.test(auth) || /unauthenticated/.test(auth)) {
    await replaceClaudeCredential({ rpc: request => rpc('executor', request), profileId: profile.profile_id, token: readToken(config.claudeToken) });
    auth = JSON.stringify(await authStatus());
  }
  await setWorkerProfileDefault({ rpc: request => rpc('executor', request), profileId: profile.profile_id });
  writeFileSync(file('executor', 'claude-profile.id'), profile.profile_id);
  // Workers use the profile's config directory as their HOME for Claude: the shared subagent definitions go there.
  const profileConfig = file('executor', `chariox/state/provider-accounts/local/claude/${profile.profile_id}/claude`);
  const agents = existsSync(profileConfig) ? syncWorkerAgents({ source: config.agentDefinitions || '/root/.claude/agents', target: profileConfig, names: config.workerAgents }) : { reason: 'profile config directory not found' };
  return { claudeProfile: profile.profile_id, agents, auth: (auth.match(/"(?:auth_)?(?:state|status)":"[a-z_]+"/g) || []).slice(0, 2) };
}

// Re-stores the running executor's lanes-claude credential in place (no kernel restart, no worker stopped). A Claude process takes the
// credential when it is launched, so a worker already running keeps the one it started with until its process is relaunched.
// With a label, another Claude account lives in its own profile of that label (created when missing) and only queue entries that
// name it (`account`) launch on it; lanes-claude and the kernel default stay as they are.
async function setClaudeCredential({ from, profile: label }) {
  if (!kernelPid('executor')) throw new Error('the executor kernel is not running; start it first');
  if (label) {
    const token = readToken(from);
    const stored = await storeClaudeProfile({
      rpc: request => rpc('executor', request), label, token,
      configDirectory: id => { const directory = file('executor', `chariox/state/provider-accounts/local/claude/${id}/claude`); return existsSync(directory) ? directory : undefined; },
      agentSource: config.agentDefinitions || '/root/.claude/agents', agentNames: config.workerAgents,
    });
    const probe = await probeClaudeToken(token);
    return { label, ...stored, org: probe.org, probe: probe.usable ? 'usable' : probe.reason };
  }
  const profileFile = file('executor', 'claude-profile.id');
  if (!existsSync(profileFile)) throw new Error('no lanes-claude profile yet: run start first');
  const token = readToken(from ?? config.claudeToken);
  const auth = await replaceClaudeCredential({ rpc: request => rpc('executor', request), profileId: readText(profileFile), token });
  const probe = await probeClaudeToken(token);
  return { claudeProfile: readText(profileFile), auth, org: probe.org, probe: probe.usable ? 'usable' : probe.reason };
}

// Codex lanes run on the account the lanes already use. The product links that provider home in place
// (no copy, no login or logout); if the kernel cannot run it, Claude keeps working and the reason is reported.
async function ensureCodexProfile() {
  if (!config.codexHome) return { codexProfile: null, reason: 'config.codexHome is not set' };
  const profiles = async () => (await rpc('executor', { ListProviderAccountProfiles: { provider: 'codex' } })).ProviderAccountProfilesListed.profiles;
  try {
    let profile = (await profiles()).find(p => p.label === 'lanes-codex');
    if (!profile) {
      await rpc('executor', { LinkProviderAccountProfile: { provider: 'codex', label: 'lanes-codex', path: config.codexHome } });
      profile = (await profiles()).find(p => p.label === 'lanes-codex');
    }
    if (!profile) throw new Error('lanes-codex profile was not created');
    const refreshed = JSON.stringify(await rpc('executor', { RefreshProviderAccountProfile: { provider: 'codex', account_profile: profile.profile_id } }));
    if (!/"auth_state":"authenticated"/.test(refreshed)) throw new Error('lanes-codex profile is not authenticated; ask the coordinator to refresh the linked Codex account');
    writeFileSync(file('executor', 'codex-profile.id'), profile.profile_id);
    return { codexProfile: profile.profile_id };
  } catch (error) { return { codexProfile: null, reason: String(error.message).slice(0, 240) }; }
}

// The loop's own non-secret config: which profile each queue provider uses. Lane entries add their own model and effort.
function writeLoopConfig() {
  const path = join(STATE, 'loop', 'config.json');
  const loop = existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')) : {};
  if (existsSync(file('executor', 'kernel-started-at'))) loop.kernelStartedAtMs = Number(readText(file('executor', 'kernel-started-at')));
  loop.kernel = JSON.parse(readFileSync(file('executor', 'ids.json'), 'utf8')).kernel_id;
  loop.repository ??= join(STATE, 'loop', 'imported.git');
  loop.providers ??= {};
  const claude = file('executor', 'claude-profile.id'), codex = file('executor', 'codex-profile.id');
  // Use the running kernel's environment, never a caller's env added after it started.
  const pid = kernelPid('executor');
  const sandbox = pid ? readFileSync(`/proc/${pid}/environ`, 'utf8').split('\0').includes('IS_SANDBOX=1') : roleEnv('executor').IS_SANDBOX === '1';
  if (existsSync(claude)) loop.providers.claude = { profile: readText(claude), model: 'claude-sonnet-5-5', effort: 'high', permission: sandbox ? 'yolo' : 'required' };
  if (existsSync(codex)) loop.providers.codex = { profile: readText(codex), model: 'gpt-6.1-sol', effort: 'medium', permission: 'yolo' };
  mkdirSync(join(STATE, 'loop'), { recursive: true, mode: 0o700 });
  writeFileSync(path, JSON.stringify(loop, null, 2) + '\n', { mode: 0o600 });
}

// Two holders: loopd's (the round service) and the coordinator's own. Each has its own until-exit access grant, so each has its
// own event inbox: the coordinator's session waits on worker events without taking them from loopd.
const SEATS = {
  loopd: { socket: 'holder.sock', pidFile: 'holder.pid', log: 'holder.log', argv: ['holder'] },
  coordinator: { socket: 'coordinator-holder.sock', pidFile: 'coordinator-holder.pid', log: 'coordinator-holder.log', argv: ['holder', 'coordinator'] },
};
const seatOf = name => SEATS[name ?? 'loopd'] ?? (() => { throw new Error(`unknown seat ${name}; use ${Object.keys(SEATS).join(' or ')}`); })();
const holderPid = seat => {
  const { pidFile, argv } = seatOf(seat);
  if (!existsSync(join(STATE, pidFile))) return undefined;
  const pid = Number(readText(join(STATE, pidFile)));
  if (!Number.isSafeInteger(pid) || pid <= 1 || !pidAlive(pid)) return undefined;
  try {
    const args = readFileSync(`/proc/${pid}/cmdline`, 'utf8').split('\0'); args.pop();
    return args[1]?.endsWith('lanes-kernel.mjs') && args.slice(2).join(' ') === argv.join(' ') ? pid : undefined;
  } catch { return undefined; }
};

// The holder is the long-lived ancestor an until-exit access grant binds to; exec runs commands beneath it.
function runHolder(seatName) {
  const seat = seatOf(seatName);
  const socketPath = join(STATE, seat.socket);
  const env = { ...roleEnv('coordinator'), CHARIOX_BIN: join(STATE, 'bin/chariox'), CHARIOX_COORDINATOR_HOME: join(STATE, 'loop'), CHARIOX_CLI_ACCESS_HOLDER_PID: String(process.pid), ...(config.agents && { LOOPD_AGENTS: config.agents }), ...(seatName === 'coordinator' && { LANEWAIT_HOME: join(STATE, 'watch') }) };
  try { unlinkSync(socketPath); } catch {}
  const server = createServer(socket => {
    let buffer = '';
    socket.on('error', () => {});
    socket.on('data', chunk => {
      buffer += chunk;
      if (!buffer.includes('\n')) return;
      const request = JSON.parse(buffer.split('\n')[0]); buffer = '';
      const child = spawn(request.argv[0], request.argv.slice(1), { cwd: request.cwd, env: { ...process.env, ...env, ...request.env }, stdio: ['ignore', request.logFile ? openSync(request.logFile, 'a') : 'pipe', request.logFile ? openSync(request.logFile, 'a') : 'pipe'] });
      child.on('error', error => socket.end(JSON.stringify({ err: Buffer.from(String(error.message)).toString('base64'), exit: 127 }) + '\n'));
      // A detached command is acknowledged once it has really started; a failed spawn answers through the error above.
      if (request.detach) { child.on('spawn', () => { child.unref(); socket.end(JSON.stringify({ pid: child.pid, exit: 0 }) + '\n'); }); return; }
      child.stdout.on('data', data => socket.write(JSON.stringify({ out: data.toString('base64') }) + '\n'));
      child.stderr.on('data', data => socket.write(JSON.stringify({ err: data.toString('base64') }) + '\n'));
      child.on('close', code => socket.end(JSON.stringify({ exit: code ?? 1 }) + '\n'));
      // A command whose client is gone (a dropped ssh session) must not go on consuming events nobody reads.
      socket.on('close', () => { if (child.exitCode === null && child.signalCode === null && Number.isSafeInteger(child.pid) && child.pid > 1) child.kill('SIGTERM'); });
    });
  });
  server.listen(socketPath, () => { chmodSync(socketPath, 0o600); writeFileSync(join(STATE, seat.pidFile), String(process.pid)); });
  for (const signal of ['SIGTERM', 'SIGINT']) process.on(signal, () => { server.close(); try { unlinkSync(socketPath); } catch {} process.exit(0); });
  setInterval(() => {}, 1 << 30);
}

function ensureHolder(seatName) {
  const existing = holderPid(seatName);
  if (existing) return existing;
  const seat = seatOf(seatName);
  try { unlinkSync(join(STATE, seat.socket)); } catch {}
  const log = openSync(join(STATE, seat.log), 'a');
  const child = spawn(process.execPath, [new URL(import.meta.url).pathname, ...seat.argv], { env: { PATH: process.env.PATH, HOME: process.env.HOME, LANES_KERNEL_STATE: STATE }, detached: true, stdio: ['ignore', log, log] });
  child.unref();
  return child.pid;
}

function execInHolder(request, seatName) {
  return new Promise((resolve, reject) => {
    const socket = connect(join(STATE, seatOf(seatName).socket));
    const result = { code: 1, stdout: [], stderr: [] };
    let buffer = '';
    socket.on('connect', () => socket.write(JSON.stringify(request) + '\n'));
    socket.on('data', chunk => {
      buffer += chunk;
      let index;
      while ((index = buffer.indexOf('\n')) >= 0) {
        const message = JSON.parse(buffer.slice(0, index)); buffer = buffer.slice(index + 1);
        if (message.out) result.stdout.push(Buffer.from(message.out, 'base64'));
        if (message.err) result.stderr.push(Buffer.from(message.err, 'base64'));
        if (message.pid) result.pid = message.pid;
        if ('exit' in message) result.code = message.exit;
      }
    });
    socket.on('close', () => resolve({ code: result.code, pid: result.pid, stdout: Buffer.concat(result.stdout).toString(), stderr: Buffer.concat(result.stderr).toString() }));
    socket.on('error', reject);
  });
}
const chariox = join(STATE, 'bin/chariox');

// loopd: the standing round service. It runs beneath the holder from a frozen copy of the adapters, so
// editing the checkout cannot change a live round; `loopd install` refreshes the copy between rounds.
const loopdPidFile = join(STATE, 'loopd.pid');
const loopdEntry = join(STATE, 'run', 'tools/lanes/coordinator/round.mjs');
const loopdPid = () => {
  if (!existsSync(loopdPidFile)) return undefined;
  const pid = Number(readText(loopdPidFile));
  return Number.isSafeInteger(pid) && pid > 1 && pidAlive(pid) && readFileSync(`/proc/${pid}/cmdline`, 'utf8').split('\0').slice(1, 3).join(' ') === `${loopdEntry} serve` ? pid : undefined;
};
async function startLoopd() {
  const existing = loopdPid();
  if (existing) return { loopd: existing, started: false };
  if (!existsSync(loopdEntry)) throw new Error('no frozen adapters in run/; run: lanes-kernel.mjs loopd install');
  if (!holderPid()) throw new Error('the holder is not running; run start first');
  const started = await execInHolder({ argv: [process.execPath, loopdEntry, 'serve'], cwd: STATE, detach: true, logFile: join(STATE, 'loopd.log') });
  if (!Number.isSafeInteger(started.pid) || started.pid <= 1) throw new Error('loopd did not start');
  writeFileSync(loopdPidFile, String(started.pid));
  return { loopd: started.pid, started: true };
}
async function stopLoopd() {
  const pid = loopdPid();
  if (!pid) return { loopd: null, stopped: false };
  process.kill(pid, 'SIGTERM');
  for (let i = 0; i < 120 && pidAlive(pid); i++) await delay(500);
  if (pidAlive(pid)) throw new Error(`loopd ${pid} did not stop after SIGTERM; it finishes its current step first, inspect loopd.log before stopping it by hand`);
  return { loopd: pid, stopped: true };
}
async function installLoopd() {
  const root = new URL('../../..', import.meta.url).pathname;
  const next = join(STATE, 'run.next');
  rmSync(next, { recursive: true, force: true }); mkdirSync(next, { recursive: true });
  const archive = spawnSync('git', ['-C', root, 'archive', 'HEAD', 'tools/lanes'], { maxBuffer: 64 << 20 });
  if (archive.status !== 0) throw new Error('git archive of tools/lanes failed');
  const untar = spawnSync('tar', ['-x', '-C', next], { input: archive.stdout });
  if (untar.status !== 0) throw new Error('could not unpack the adapters copy');
  const stopped = await stopLoopd();
  rmSync(join(STATE, 'run'), { recursive: true, force: true }); renameSync(next, join(STATE, 'run'));
  return { ...stopped, installedFrom: spawnSync('git', ['-C', root, 'rev-parse', 'HEAD']).stdout.toString().trim() };
}
const queueCount = name => { try { return readdirSync(join(config.agents || '/root/.chariox/dev/browser-resume-20260930/agents', '_loop', 'queue', name)).filter(f => f.endsWith('.json')).length; } catch { return 0; } };

// The owner approves the until-exit grant in the real web popup, typing the coordinator Vault passphrase.
async function approveInPopup() {
  const meta = JSON.parse(readFileSync(`/w/valenv/prs/${config.stack}/metadata.json`, 'utf8'));
  const ids = JSON.parse(readFileSync(file('coordinator', 'ids.json'), 'utf8'));
  const { chromium } = await import('/w/valenv/tools/node_modules/playwright-core/index.mjs');
  const browser = await chromium.launch({ executablePath: '/opt/google/chrome/chrome', headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  try {
    const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 })).newPage();
    await page.goto(`https://${meta.hostname}/terminal?kernel=${ids.kernel_id}&session=${readText(file('coordinator', 'session.id'))}`);
    const input = page.locator('[data-passkey-popup-input]').first();
    await input.waitFor({ state: 'visible', timeout: 120_000 });
    const lifetime = page.locator('[data-passkey-popup-lifetime]');
    if (await lifetime.count()) await lifetime.selectOption('until_exit');
    await input.click();
    await page.keyboard.type(readText(file('coordinator', 'passphrase')), { delay: 10 });
    await page.locator('[data-passkey-popup-approve]').click();
    await input.waitFor({ state: 'hidden', timeout: 60_000 });
  } finally { await browser.close(); }
}

async function grantAccess(seat) {
  const pid = ensureHolder(seat);
  for (let i = 0; i < 40 && !existsSync(join(STATE, seatOf(seat).socket)); i++) await delay(250);
  const holder = holderPid(seat) ?? pid;
  const check = async () => {
    const result = await execInHolder({ argv: [chariox, 'access', 'status', '--holder-pid', String(holder)], cwd: STATE }, seat);
    try { return result.code === 0 ? JSON.parse(result.stdout).status : `exit_${result.code}`; } catch { return 'unparsed'; }
  };
  const current = await check();
  if (current === 'granted' || current === 'approved' || current === 'active') return { holder, granted: 'existing', status: current };
  const request = execInHolder({ argv: [chariox, 'access', 'request', '--until-exit', '--holder-pid', String(holder)], cwd: STATE }, seat);
  const [approved] = await Promise.all([request, approveInPopup()]);
  if (approved.code !== 0) throw new Error(`access request exited ${approved.code}: ${approved.stderr.slice(0, 200)}`);
  return { holder, granted: 'now', status: await check() };
}

async function status() {
  const result = { state: STATE, runtime: config.runtime.head, stack: config.stack };
  for (const role of ROLES) {
    const pid = kernelPid(role);
    const entry = { pid: pid ?? null };
    if (pid) {
      try {
        const s = await relayStatus(role, 3);
        entry.relay_connected = s.connected; entry.kernel_id = s.daemon_id; entry.protocol = s.protocol_version ?? null;
        const vault = await rpc(role, { GetCredentialVaultStatus: null });
        entry.vault = vault.CredentialVaultStatus?.status ?? vault.CredentialVaultStatus ?? 'unknown';
      } catch (error) { entry.error = String(error.message).slice(0, 160); }
    }
    result[role] = entry;
  }
  result.holder = holderPid() ?? null;
  result.coordinatorHolder = holderPid('coordinator') ?? null;
  result.loopd = { pid: loopdPid() ?? null, queued: queueCount(''), done: queueCount('done'), rejected: queueCount('rejected') };
  const url = `https://${JSON.parse(readFileSync(`/w/valenv/prs/${config.stack}/metadata.json`, 'utf8')).hostname}/validation/ready`;
  result.stack_ready = await fetch(url).then(r => r.ok, () => false);
  return result;
}

async function startLanes() {
  const started = [];
  for (const role of ['executor', 'coordinator']) started.push(await startKernel(role));
  const steps = [];
  for (const role of ROLES) steps.push(await ensureVault(role));
  for (const role of ROLES) steps.push(await enroll(role));
  steps.push(await ensureClaudeProfile());
  steps.push(await ensureCodexProfile());
  writeLoopConfig();
  // One popup at a time: the loopd seat first, then the coordinator's own.
  steps.push({ grant: await grantAccess() });
  steps.push({ coordinatorGrant: await grantAccess('coordinator') });
  steps.push(await startLoopd());
  return { started, steps };
}
async function stopLanes() {
  const results = [];
  results.push(await stopLoopd());
  for (const seat of Object.keys(SEATS)) {
    const holder = holderPid(seat);
    if (holder) { process.kill(holder, 'SIGTERM'); results.push({ holder, seat, stopped: true }); }
  }
  for (const role of ROLES) results.push(await stopKernel(role));
  return results;
}

const [command, ...rest] = process.argv.slice(2);
try {
  if (command === 'holder') runHolder(rest[0]);
  else if (command === 'start' || command === 'setup') {
    out(await startLanes());
  } else if (command === 'stop') {
    out(await stopLanes());
  } else if (command === 'restart-yolo') {
    let stoppingLoopdPid;
    const check = await restartWithSandbox({
      state: STATE, agents: config.agents || '/root/.chariox/dev/browser-resume-20260930/agents',
      preflight: async () => {
        const runtime = spawnSync('git', ['-C', config.runtime.worktree, 'merge-base', '--is-ancestor', 'af52049173a502fd30fa21a32e039c874fe0b9eb', config.runtime.head]);
        if (runtime.status !== 0) throw new Error('restart requires the af52049173 carry-over runtime or a descendant');
        const provenance = JSON.parse(readFileSync(join(STATE, 'PROVENANCE.json'), 'utf8'));
        if (provenance.head !== config.runtime.head || provenance.sha256?.['chariox-kernel'] !== createHash('sha256').update(readFileSync(kernelBinary)).digest('hex')) throw new Error('kernel binary does not match the recorded carry-over runtime; no restart performed');
        for (const role of ROLES) if (!kernelPid(role)) throw new Error(`${role} kernel is not running; refusing a partial restart`);
        const probe = spawnSync(process.env.CLAUDE_ACCT || '/root/.chariox/dev/coordinator/claude-acct.sh', ['probe'], { encoding: 'utf8', timeout: 100_000, maxBuffer: 64 << 10 });
        if (probe.status !== 0) throw new Error('claude-acct.sh probe failed; no restart performed');
        const accounts = probe.stdout.split('\n').filter(line => /^oauth-token(?:\.[a-z-]+)? org=/.test(line));
        const active = accounts.find(line => line.startsWith('oauth-token '));
        const org = active?.match(/org=…([a-f0-9]+)/)?.[1];
        if (!org || !active.includes('http=200') || !/status=allowed(?:_warning)?(?: |$)/.test(active)) throw new Error('active Claude account is not usable; no restart performed');
        const named = accounts.find(line => !line.startsWith('oauth-token ') && line.includes(`org=…${org} `));
        if (!named) throw new Error('active Claude account name could not be resolved; no restart performed');
        return `${named.split(' ')[0].replace('oauth-token.', '')}(org=…${org})`;
      },
      loopdRunning: async () => Boolean(loopdPid()),
      stopLoopd: async () => { stoppingLoopdPid = loopdPid(); return stopLoopd(); },
      startLoopd: async () => {
        // A timeout can leave loopd finishing its current collection with SIGTERM pending.
        // Wait for that exact process before starting its replacement, or it could exit just after this returns.
        for (let i = 0; stoppingLoopdPid && pidAlive(stoppingLoopdPid) && i < 600; i++) await delay(500);
        if (stoppingLoopdPid && pidAlive(stoppingLoopdPid)) throw new Error(`loopd ${stoppingLoopdPid} is still stopping; kernels left running`);
        return startLoopd();
      },
      stop: stopLanes, start: startLanes,
      inspect: run => inspectWorker(request => rpc('executor', request), run),
    });
    out(check);
  } else if (command === 'status') out(await status());
  else if (command === 'loopd') {
    const action = rest[0];
    if (action === 'install') { const done = await installLoopd(); out({ ...done, ...(holderPid() ? await startLoopd() : {}) }); }
    else if (action === 'start') out(await startLoopd());
    else if (action === 'stop') out(await stopLoopd());
    else { console.error('usage: lanes-kernel.mjs loopd install|start|stop'); process.exitCode = 2; }
  }
  else if (command === 'set-claude-credential') {
    const options = parseCredentialArgs(rest);
    if (options.error) { console.error(`${options.error}\nusage: lanes-kernel.mjs set-claude-credential [--profile <label> --from <token-file> | --from <token-file>]`); process.exitCode = 2; }
    else out(await setClaudeCredential(options));
  }
  else if (command === 'grant') out(await grantAccess(rest[0]));
  else if (command === 'exec') {
    const seat = rest[0] === '--seat' ? (rest.shift(), rest.shift()) : undefined;
    const detach = rest[0] === '--detach' ? rest.shift() : undefined;
    const log = detach && rest[0] === '--log' ? (rest.shift(), rest.shift()) : undefined;
    if (rest[0] === '--') rest.shift();
    const result = await execInHolder({ argv: rest, cwd: process.cwd(), detach: Boolean(detach), logFile: log }, seat);
    process.stdout.write(result.stdout); process.stderr.write(result.stderr);
    if (result.pid) out({ pid: result.pid });
    process.exitCode = result.code;
  } else if (command === 'env') {
    const env = roleEnv('coordinator');
    for (const key of ['CHARIOX_HOME', 'CHARIOX_DAEMON_SOCKET']) console.log(`${key}=${env[key]}`);
    console.log(`CHARIOX_BIN=${join(STATE, 'bin/chariox')}`, `CHARIOX_COORDINATOR_HOME=${join(STATE, 'loop')}`);
  } else { console.error('usage: lanes-kernel.mjs setup|start|stop|status|grant [seat]|loopd|exec [--seat coordinator]|env|set-claude-credential [--profile label] [--from file]'); process.exitCode = 2; }
} catch (error) {
  console.error(JSON.stringify({ error: String(error.message).replace(/(?:Bearer\s+|token[=:]\s*)\S+/gi, '[redacted]').slice(0, 300) }));
  process.exitCode = 1;
}
