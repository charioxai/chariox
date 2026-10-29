// MP-08 / MP-09 / MP-10 / MP-11: shared real native-session proof helpers.
import assert from 'node:assert/strict';
import { writeFile, readFile, readdir } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { out, record, sha } from './workspace-runtime.mjs';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const entries = s => s.agentPanes?.[s.session.focusedAgentId] ?? [];
const variant = (reply, key) => { assert.ok(reply[key], `Expected ${key}: ${Object.keys(reply)}`); return reply[key]; };
async function state(r, id) { return variant(await r.client.send({GetSessionState: {session_id: id}}), 'SessionState'); }
async function currentRun(r, id, agentId) {
    const core = await state(r, id), agent = core.session.agents.find(a => a.id === agentId);
    const runId = agent.active_provider_run_id ?? core.agent_activity?.[agentId]?.last_completed_turn?.provider_run_id;
    assert.ok(runId, 'Real provider run identity');
    return variant(await r.client.send({GetProviderRun: {provider_run_id: runId}}), 'ProviderRun').provider_run;
}
async function accountIdentity(run) {
    assert.ok(run.structured_endpoint, 'Actual provider endpoint is required for account proof');
    const endpoint = new URL(run.structured_endpoint);
    assert.equal(endpoint.protocol, 'ws:');
    assert.equal(endpoint.hostname, '127.0.0.1');
    const socket = new WebSocket(endpoint);
    function request(id, method, params) {
        return new Promise((resolve, reject) => {
            const timer = setTimeout(() => { socket.removeEventListener('message', receive); reject(Error('Official account metadata deadline')); }, 15000);
            function receive(event) {
                const reply = JSON.parse(event.data);
                if (reply.id !== id) return;
                clearTimeout(timer); socket.removeEventListener('message', receive);
                if (reply.error) reject(Error('Official account metadata request failed'));
                else resolve(reply.result);
            }
            socket.addEventListener('message', receive);
            socket.send(JSON.stringify({id, method, params}));
        });
    }
    try {
        await new Promise((resolve, reject) => {
            const timer = setTimeout(() => reject(Error('Official account endpoint connection deadline')), 15000);
            socket.addEventListener('open', () => { clearTimeout(timer); resolve(); }, {once: true});
            socket.addEventListener('error', () => { clearTimeout(timer); reject(Error('Official account endpoint connection failed')); }, {once: true});
        });
        await request(0, 'initialize', {clientInfo: {name: 'chariox-movekernel-account-proof', version: '0.1.0'}, capabilities: {experimentalApi: true}});
        socket.send(JSON.stringify({method: 'initialized', params: {}}));
        const auth = await request(1, 'account/read', {});
        assert.ok(auth.account?.email?.trim(), 'Actual provider must report its authenticated identity');
        return sha(Buffer.from('codex:authenticated:' + auth.account.email.trim().toLowerCase()));
    } finally { socket.close(); }
}
async function captureTui(r, label) {
    const ansi = out + '/' + label + '.ansi';
    await writeFile(ansi, r.tui.output(), {mode: 0o600});
    execFileSync('python3', [new URL('../context-switch/render-pty.py', import.meta.url).pathname, ansi], {stdio: 'ignore'});
}
async function turnProgress(r, label, snapshot) {
    const agentId = snapshot.session.focusedAgentId;
    const agent = snapshot.session.agents.find(a => a.id === agentId);
    const core = await state(r, snapshot.session.id);
    const actual = core.session.agents.find(a => a.id === agentId);
    const activity = core.agent_activity?.[agentId];
    const runId = actual?.active_provider_run_id ?? activity?.last_completed_turn?.provider_run_id;
    const response = runId ? await r.client.send({GetProviderRun: {provider_run_id: runId}}) : undefined;
    const run = response?.ProviderRun?.provider_run;
    await record(label + '-progress', {
        agentState: agent?.state, isProcessing: agent?.isProcessing,
        tuiFocusedAgentId: snapshot.session.focusedAgentId,
        homeFocusedAgentId: core.session.focused_agent_id,
        tuiRemoteKernelId: agent?.remoteExecution?.workerKernelId,
        tuiAccountProfile: agent?.accountProfile,
        homeAccountProfile: actual?.account_profile,
        statusLine: snapshot.statusLine,
        fatalError: snapshot.fatalError,
        activeProviderRunId: actual?.active_provider_run_id,
        remoteKernelId: actual?.remote_execution?.worker_kernel_id,
        leasedAgentId: actual?.remote_execution?.leased_agent_id,
        notices: entries(snapshot).filter(row => row.role === 'notice' || row.role === 'error').map(row => ({id: row.id, promptId: row.promptId, text: row.text})),
        rowRoles: entries(snapshot).reduce((counts, row) => { counts[row.role] = (counts[row.role] ?? 0) + 1; return counts; }, {}),
        interactions: (snapshot.interactions ?? []).map(i => ({kind: i.kind, title: i.title, choices: i.choices?.map(c => c.id)})),
        providerRun: run && {id: run.id, state: run.state, phase: run.phase, workingDirectory: run.working_directory},
    });
}
async function assertTuiRoute(r, agentId, remoteKernelId, label) {
    let observed;
    for (let n = 0; n < 120; n++) {
        const snapshot = await r.tui.send('snapshot');
        const agent = snapshot.session.agents.find(a => a.id === agentId);
        observed = agent?.remoteExecution?.workerKernelId ?? null;
        if (agent && observed === remoteKernelId) {
            await record(label + '-tui-route', {remoteKernelId: observed, accountProfile: agent.accountProfile, statusLine: snapshot.statusLine});
            return;
        }
        await sleep(250);
    }
    await captureTui(r, label + '-stale-route');
    assert.equal(observed, remoteKernelId, 'Attached TUI must apply the recovered execution route');
}
async function turn(r, text, label, options = {}) {
    const before = options.before ?? new Set(entries(await r.tui.send('snapshot')).map(row => row.id));
    if (!options.submitted) await r.tui.send('submit_prompt', {prompt: text});
    let settled;
    for (let n = 0; n < 720; n++) {
        let snapshot = await r.tui.send('snapshot');
        if (n % 40 === 0) await turnProgress(r, label, snapshot);
        const agent = snapshot.session.agents.find(a => a.id === snapshot.session.focusedAgentId);
        const permission = snapshot.interactions?.find(i => i.kind === 'permission' && i.title === 'Command approval required');
        if (permission) {
            const choiceIndex = permission.choices.findIndex(c => c.id === 'allow_once');
            assert.ok(choiceIndex >= 0); await r.tui.send('interaction_submit', {choiceIndex});
        }
        const terminal = entries(snapshot).some(row => !before.has(row.id) && ['assistant', 'error'].includes(row.role) && row.text?.trim());
        if (!agent?.isProcessing && agent?.state !== 'Working' && terminal) {
            settled ??= Date.now();
            if (Date.now() - settled >= 1500) {
                assert.notEqual(agent.state, 'Error', 'Provider turn failed');
                const promptId = entries(snapshot).findLast(row => row.role === 'user' && row.text?.trim() === text)?.promptId;
                if (entries(snapshot).some(row => row.role === 'tool' && row.promptId === promptId && row.blobCollapsed)) await r.tui.type('\x0f');
                for (let n = 0; n < 80; n++) {
                    snapshot = await r.tui.send('snapshot');
                    if (entries(snapshot).filter(row => row.role === 'tool' && row.promptId === promptId).every(row => !row.blobCollapsed && !row.historyBlobLoading)) break;
                    await sleep(250);
                }
                await captureTui(r, label);
                return {snapshot, promptId};
            }
        } else settled = undefined;
        await sleep(250);
    }
    await turnProgress(r, label, await r.tui.send('snapshot'));
    throw Error('Real provider turn deadline: ' + label);
}
async function journal(r, generation) {
    const directory = r.path + '/home/agent-moves';
    for (const name of await readdir(directory).catch(() => [])) {
        if (!name.endsWith('.home.json')) continue;
        const value = JSON.parse(await readFile(directory + '/' + name, 'utf8'));
        if (value.identity.generation === generation) return value;
    }
}
export { sleep, entries, variant, state, currentRun, accountIdentity, captureTui, assertTuiRoute, turn, journal };
