// MP-08 / MP-10 / MP-11: owned real kernels, hosted enrollment and TUI/CLI controls.
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { createWriteStream } from 'node:fs';
import { mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import net from 'node:net';
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js';
import { openTui } from '../context-switch/tui.mjs';
import { spawnOwned, signalOwnedProcessGroup, signalOwnedPid, descendants, processSnapshot } from '../context-switch/owned.mjs';
import { registerTerminationCleanup } from '../context-switch/termination.mjs';
import { checkpointDiagnostics } from './workspace-diagnostics.mjs';
if (process.argv.includes('--help')) {
    console.log('Run with absolute CHARIOX_WORKSPACE_KERNEL, CHARIOX_WORKSPACE_TUI, CHARIOX_WORKSPACE_PROFILE, CHARIOX_WORKSPACE_STATE and CHARIOX_WORKSPACE_EVIDENCE paths.');
    process.exit(0);
}
const required = name => {
    const value = process.env[name];
    if (!value?.startsWith('/'))
        throw Error(`${name} requires an absolute path`);
    return value;
};
const root = required('CHARIOX_WORKSPACE_STATE'), out = required('CHARIOX_WORKSPACE_EVIDENCE');
const kernel = required('CHARIOX_WORKSPACE_KERNEL'), binary = required('CHARIOX_WORKSPACE_TUI');
const origin = process.env.CHARIOX_WORKSPACE_ORIGIN ?? 'https://val.51-255-87-178.sslip.io';
const relay = process.env.CHARIOX_WORKSPACE_RELAY ?? 'wss://valrelay.51-255-87-178.sslip.io';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const variant = (reply, name) => {
    assert.ok(reply[name], `Expected ${name}, got ${Object.keys(reply)}`);
    return reply[name];
};
async function port() {
    return await new Promise((resolve, reject) => {
        const server = net.createServer();
        server.on('error', reject);
        server.listen(0, '127.0.0.1', () => {
            const value = server.address().port;
            server.close(() => resolve(value));
        });
    });
}
async function record(name, data) {
    await writeFile(out + '/' + name + '.json', JSON.stringify({ MP: ['MP-08', 'MP-10', 'MP-11'], at: new Date().toISOString(), ...data }, null, 2) + '\n', { mode: 0o600 });
    console.log(name);
}
const runtimes = [];
let browser, closed = false;
async function stop(child, signal = 'SIGINT') {
    if (!child)
        return;
    const owned = descendants(child.pid);
    if (child.exitCode === null && child.signalCode === null)
        signalOwnedProcessGroup(child, signal);
    for (let n = 0; n < 60; n++) {
        if (!processSnapshot().some(p => p.state !== 'Z' && owned.some(x => p.pid === x.pid && p.start === x.start)))
            return;
        await sleep(250);
    }
    for (const p of processSnapshot().filter(p => p.state !== 'Z' && owned.some(x => p.pid === x.pid && p.start === x.start)))
        signalOwnedPid(p.pid, 'SIGKILL');
}
async function close() {
    if (closed)
        return;
    closed = true;
    const owned = runtimes.flatMap(r => [...descendants(r.child.pid), ...(r.tui ? descendants(r.tui.child.pid) : [])]);
    for (const r of [...runtimes].reverse()) {
        await r.tui?.close().catch(() => {
        });
        await r.client?.send({ DisconnectCloudRelay: null }).catch(() => {
        });
        await r.client?.close().catch(() => {
        });
        await stop(r.child);
    }
    await browser?.close();
    await sleep(500);
    await record('checkpoint-diagnostics', {kernels: await Promise.all(runtimes.map(async r => ({
        name: r.name, probes: await checkpointDiagnostics(r.path + '/home'),
    })))});
    const remaining = processSnapshot().filter(p => p.state !== 'Z' && owned.some(x => p.pid === x.pid && p.start === x.start));
    await record('cleanup', { remaining: remaining.map(p => ({ pid: p.pid, start: p.start })), kernels: runtimes.map(r => ({ pid: r.child.pid, exitCode: r.child.exitCode, signal: r.child.signalCode })) });
    assert.equal(remaining.length, 0);
    await rm(root, { recursive: true, force: true });
}
const release = registerTerminationCleanup(close);
async function runtime(name) {
    const path = root + '/' + name;
    for (const dir of ['home', 'os-home', 'tmp', 'config'])
        await mkdir(path + '/' + dir, { recursive: true, mode: 0o700 });
    await writeFile(path + '/home/config.toml', 'version = 1\n[state]\npath = "' + path + '/home/state.db"\n[history.archive]\nmode = "disabled"\n[credential_vault]\nbackend = "chariox_encrypted"\npath = "' + path + '/home/private-vault.json"\nunlock_policy = "kernel_init"\nagent_management = "allow"\n', { mode: 0o600 });
    const ports = await Promise.all(Array.from({ length: 4 }, port));
    const env = { PATH: process.env.PATH, LANG: 'C.UTF-8', HOME: path + '/os-home', CHARIOX_HOME: path + '/home', TMPDIR: path + '/tmp', XDG_CONFIG_HOME: path + '/config', XDG_DATA_HOME: path + '/data', XDG_STATE_HOME: path + '/state', XDG_CACHE_HOME: path + '/cache', IS_SANDBOX: '1', CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_CODEX_PORT: String(ports[2]), CHARIOX_OPENCODE_PORT: String(ports[3]), CHARIOX_DAEMON_SOCKET: path + '/daemon.sock', RUST_MIN_STACK: '16777216' };
    const url = `ws://127.0.0.1:${ports[0]}/kernel`;
    const child = spawnOwned(kernel, [], { cwd: path, env, stdio: ['ignore', 'pipe', 'pipe'], detached: true });
    const stream = createWriteStream(path + '/kernel.private.log', { mode: 0o600 });
    child.stdout.pipe(stream);
    child.stderr.pipe(stream);
    const r = { name, path, env, url, child };
    runtimes.push(r);
    await record(name + '-process', { pid: child.pid, kernelSha256: sha(await readFile(kernel)) });
    for (let n = 0; n < 120; n++) {
        const client = new LocalIpcClient(url, { timeoutMs: 10000, localAuthEnvironment: env });
        try {
            await client.send({ ListSessions: null });
            r.client = client;
            return r;
        }
        catch {
            await client.close();
            if (child.exitCode !== null)
                throw Error(name + ' kernel exited');
            await sleep(500);
        }
    }
    throw Error(name + ' readiness timeout');
}
// Restart only this harness's kernel and reconnect the same real TUI Room.
async function restart(r, workspace, sessionId) {
    // Interrupt the captured transaction before closing its client or TUI.
    const exited = new Promise(resolve => r.child.once('exit', resolve));
    await stop(r.child, 'SIGKILL');
    if (r.child.exitCode === null && r.child.signalCode === null) await exited;
    assert.equal(r.child.signalCode, 'SIGKILL', `Own kernel must be interrupted mid-Move (exit=${r.child.exitCode}, signal=${r.child.signalCode})`);
    await r.tui?.close();
    await r.client?.close();
    r.child = spawnOwned(kernel, [], { cwd: r.path, env: r.env, stdio: ['ignore', 'pipe', 'pipe'], detached: true });
    const stream = createWriteStream(r.path + '/kernel-restart.private.log', { mode: 0o600 });
    r.child.stdout.pipe(stream); r.child.stderr.pipe(stream);
    let ready = false;
    for (let n = 0; n < 120; n++) {
        const client = new LocalIpcClient(r.url, { timeoutMs: 10000, localAuthEnvironment: r.env });
        try { await client.send({ ListSessions: null }); r.client = client; ready = true; break; }
        catch { await client.close(); if (r.child.exitCode !== null) throw Error('Restarted kernel exited'); await sleep(250); }
    }
    assert.ok(ready, 'Restarted kernel readiness');
    r.tui = await openTui({ tree: workspace, env: r.env, url: r.url, profile: r.accountProfile.profile_id, root: r.path, binary, provider: 'codex', model: 'gpt-6-luna', session: sessionId, clientId: 'movekernel-restarted-' + r.name });
    await unlock(r);
    await record(r.name + '-restart', { pid: r.child.pid, sameRoom: sessionId, previousKernelSignal: 'SIGKILL' });
}
async function enroll(r) {
    const context = await browser.newContext(), page = await context.newPage();
    try {
        assert.equal((await page.goto(origin + '/validation/ready')).status(), 200);
        const before = variant(await r.client.send({ RelayStatus: null }), 'RelayStatus').status;
        const login = variant(await r.client.send({ StartCloudRelayLogin: { api_url: origin, client_id: 'movekernel-r6-' + r.name + '-' + Date.now(), client_alias: 'Move workspace ' + r.name, machine_id: before.machine_id, machine_alias: 'movekernel-r6-' + r.name } }), 'CloudRelayLoginStarted').login;
        assert.equal(new URL(login.verification_url).origin, origin);
        await page.goto(login.verification_url);
        const button = page.getByRole('button', { name: /approve|allow|connect/i });
        if (await button.count() === 1)
            await button.click();
        let approved = false;
        for (let n = 0; n < 24; n++) {
            await sleep(Math.max(5000, (login.interval_seconds ?? 5) * 1000));
            const result = variant(await r.client.send({ PollCloudRelayLogin: { api_url: origin, device_code: login.device_code } }), 'CloudRelayLoginPolled').result;
            if (result.status === 'approved') {
                approved = true;
                break;
            }
            assert.notEqual(result.status, 'denied');
        }
        assert.ok(approved);
        await r.client.send({ ConnectCloudRelay: null });
        for (let n = 0; n < 120; n++) {
            r.status = variant(await r.client.send({ RelayStatus: null }), 'RelayStatus').status;
            if (r.status.connected)
                break;
            await sleep(500);
        }
        assert.ok(r.status.connected);
        assert.equal(r.status.relay_url, relay);
        let target;
        for (let n = 0; n < 60; n++) {
            const dashboard = await (await context.request.get(origin + '/dashboard')).json();
            target = (dashboard.relayTargets ?? []).find(t => t.machineId === r.status.machine_id && t.daemonId === r.status.daemon_id);
            if (target?.status === 'ONLINE')
                break;
            await sleep(500);
        }
        assert.equal(target?.status, 'ONLINE', 'Own kernel must gain fresh hosted presence');
        r.profile = variant(await r.client.send({ CloudRelayStatus: null }), 'CloudRelayStatus').profile;
        await record(r.name + '-enrollment', { origin, relay, machineId: r.status.machine_id, kernelId: r.status.daemon_id, connected: true, dashboardStatus: target.status });
    }
    finally {
        await context.close();
    }
}
async function unlock(r) {
    let status = variant(await r.client.send({ GetCredentialVaultStatus: null }), 'CredentialVaultStatus').status;
    if (status.unlocked)
        return;
    r.passphrase ??= randomBytes(24).toString('base64url');
    await r.tui.type('\x15/credential vault manage');
    await r.tui.type('\r');
    for (let n = 0; n < 240; n++) {
        const snapshot = await r.tui.send('snapshot');
        for (const interaction of snapshot.interactions ?? []) {
            assert.match(interaction.title, /vault|passphrase|password|encrypt|protect|unlock|credential/i);
            const custom = interaction.customChoice;
            if (custom?.id === 'passphrase' || custom?.inputKind === 'secret' || custom?.input_kind === 'secret') {
                await r.tui.send('interaction_custom_reply', { interactionId: interaction.id, reply: r.passphrase, editing: true });
                await r.tui.send('interaction_submit', { choiceIndex: interaction.choices.length });
            }
            else
                await r.tui.send('interaction_submit', { choiceIndex: 0 });
        }
        status = variant(await r.client.send({ GetCredentialVaultStatus: null }), 'CredentialVaultStatus').status;
        if (status.unlocked) {
            await record(r.name + '-vault-ready', { unlocked: true, path: 'real source TUI credential vault manage' });
            return;
        }
        await sleep(250);
    }
    throw Error(r.name + ' Vault setup timeout');
}
async function project(r, paths, projectId) {
    r.accountProfile ??= variant(await r.client.send({ LinkProviderAccountProfile: { provider: 'codex', label: 'Workspace drill', path: required('CHARIOX_WORKSPACE_PROFILE') } }), 'ProviderAccountProfile').profile;
    const profile = r.accountProfile;
    await r.client.send({ RefreshProviderAccountProfile: { provider: 'codex', account_profile: profile.profile_id } });
    const created = variant(await r.client.send({ CreateSession: { workspace_id: paths.workspace ?? paths.primary, worktree_id: paths.primary, alias: 'Workspace ' + r.name, project_selection: projectId ? { kind: 'existing', project_id: projectId } : { kind: 'new', name: 'Move workspace ' + r.name }, agent_defaults: { provider: 'codex', account_profile: profile.profile_id, model: 'gpt-6-luna', effort: 'low', permission_level: 'required' } } }), 'SessionCreated');
    await r.tui?.close();
    r.tui = await openTui({ tree: paths.primary, env: r.env, url: r.url, profile: profile.profile_id, root: r.path, binary, provider: 'codex', model: 'gpt-6-luna', session: created.session.id, clientId: 'movekernel-r6-' + r.name });
    let snap;
    for (let n = 0; n < 120; n++) {
        snap = await r.tui.send('snapshot');
        if (snap.session?.agentCount > 0 && snap.attachmentId)
            break;
        await sleep(500);
    }
    assert.ok(snap.attachmentId);
    await unlock(r);
    const state = variant(await r.client.send({ GetSessionState: { session_id: snap.session.id } }), 'SessionState');
    const id = state.session.project_id;
    assert.ok(id);
    await r.client.send({ UpdateProjectWorkspaces: { project_id: id, workspace_ids: [paths.workspace ?? paths.primary, paths.notes] } });
    return id;
}
async function selection(source, target, id, paths) {
    const reply = variant(await source.client.send({ QueryFreshRemoteMachineKernels: { machine_ref: target.status.machine_id } }), 'FreshRemoteMachineKernelsObserved');
    const presence = reply.kernels.find(k => (k.kernel_id ?? k.daemon_id) === target.status.daemon_id);
    assert.ok(presence?.public_key, 'Fresh target key required');
    return { target: { relayRealmId: source.profile.realm_id, machineId: target.status.machine_id, kernelId: target.status.daemon_id, relayPublicKey: presence.public_key, keyThumbprint: sha(presence.public_key) }, contextSelection: { kernelContext: 'empty', developmentSetup: { kind: 'source_project', projectId: id, repositories: [{ role: 'primary', workspaceId: paths.primary, worktreeId: paths.primary }, { role: 'supporting', workspaceId: paths.notes, worktreeId: null }] } } };
}
async function cli(r, args) {
    const child = spawnOwned(binary, [...args, '--kernel-url', r.url], { cwd: r.path, env: r.env, stdio: ['ignore', 'pipe', 'pipe'], detached: true });
    let output = '', error = '';
    child.stdout.on('data', v => output += v);
    child.stderr.on('data', v => error += v);
    let timer, result;
    try {
        result = await Promise.race([new Promise((resolve, reject) => {
            child.once('error', reject);
            child.once('exit', (code, signal) => resolve({ code, signal }));
        }), new Promise(resolve => {
            timer = setTimeout(() => resolve({ timeout: true }), 180000);
            timer.unref();
        })]);
    } finally {
        clearTimeout(timer);
    }
    if (result.timeout) {
        await stop(child);
        throw Error('CLI context command timeout');
    }
    assert.equal(result.code, 0, 'Real context CLI failed: ' + error.slice(-1000));
    return JSON.parse(output.trim());
}
async function approve(source, label) {
    const snapshot = await source.tui.send('snapshot');
    for (const interaction of snapshot.interactions ?? []) {
        assert.match(interaction.title, /Ready to move/);
        const index = interaction.choices.findIndex(c => (c.id ?? c.value) === 'continue' || /continue/i.test(c.label ?? c.title ?? ''));
        assert.ok(index >= 0, 'Move review must offer Continue');
        const ansi = source.tui.output();
        assert.ok(!source.passphrase || !ansi.includes(source.passphrase), 'Vault input must not appear in captured PTY output');
        const capture = out + '/' + label + '-review.ansi';
        await writeFile(capture, ansi, { mode: 0o600 });
        execFileSync('python3', [new URL('../context-switch/render-pty.py', import.meta.url).pathname, capture], { stdio: 'ignore' });
        await record(label + '-review', { title: interaction.title, choices: interaction.choices.map(c => ({ id: c.id, label: c.label })), attachmentId: snapshot.attachmentId });
        await source.tui.send('interaction_submit', { choiceIndex: index });
    }
}
async function copy(source, selected, label) {
    const path = source.path + '/' + label + '.json';
    await writeFile(path, JSON.stringify(selected), { mode: 0o600 });
    let result, error, settled = false;
    const pending = cli(source, ['context', 'copy', path]).then(r => {
        result = r;
        settled = true;
    }, e => {
        error = e;
        settled = true;
    });
    for (let n = 0; n < 600 && !settled; n++) {
        await approve(source, label);
        await sleep(250);
    }
    await pending;
    if (error)
        throw error;
    const status = variant(result, 'ManagedContextTransferStarted').status;
    for (let n = 0; n < 600; n++) {
        await approve(source, label);
        const current = variant(await source.client.send({ GetManagedContextTransferStatus: { contextId: status.contextId } }), 'ManagedContextTransferStatus').status;
        if (['completed', 'failed'].includes(current.phase)) {
            await record(label + '-result', { status: current });
            return current;
        }
        await sleep(500);
    }
    throw Error(label + ' transfer timeout');
}
async function launchTarget(source, status) {
    return variant(await cli(source, ['context', 'launch-target', status.contextId, status.planDigest]), 'ManagedContextLaunchTarget').target.development;
}
async function launchBrowser() {
    const { chromium } = await import('/w/valenv/tools/node_modules/playwright-core/index.mjs');
    browser = await chromium.launch({ headless: true, executablePath: '/opt/google/chrome/chrome', args: ['--no-sandbox', '--disable-dev-shm-usage'] });
}
async function webContext(options) { assert.ok(browser, "Launch the viewer before opening a Web context"); return browser.newContext(options); }
export { webContext, root, out, kernel, binary, origin, relay, sha, record, runtime, enroll, project, selection, copy, launchTarget, launchBrowser, close, release, restart, cli };
