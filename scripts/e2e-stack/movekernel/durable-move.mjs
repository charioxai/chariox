// MP-08 / MP-09 / MP-10 / MP-11: public Move, real native turns and mid-Move restarts.
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { mkdir, writeFile, readFile, readdir } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { dirname } from 'node:path';
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js';
import { root, out, kernel, binary, sha, record, runtime, enroll, project, selection, launchBrowser, close, release, restart } from './workspace-runtime.mjs';
import { fixture } from './workspace-fixture.mjs';
import { captureWeb } from './workspace-web.mjs';
import { completedWorkspaceTool } from './workspace-proof.mjs';
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
        activeProviderRunId: actual?.active_provider_run_id,
        remoteKernelId: actual?.remote_execution?.worker_kernel_id,
        leasedAgentId: actual?.remote_execution?.leased_agent_id,
        notices: entries(snapshot).filter(row => row.role === 'notice' || row.role === 'error').map(row => ({id: row.id, promptId: row.promptId, text: row.text})),
        rowRoles: entries(snapshot).reduce((counts, row) => { counts[row.role] = (counts[row.role] ?? 0) + 1; return counts; }, {}),
        interactions: (snapshot.interactions ?? []).map(i => ({kind: i.kind, title: i.title, choices: i.choices?.map(c => c.id)})),
        providerRun: run && {id: run.id, state: run.state, phase: run.phase, workingDirectory: run.working_directory},
    });
}
async function turn(r, text, label) {
    const before = new Set(entries(await r.tui.send('snapshot')).map(row => row.id));
    await r.tui.send('submit_prompt', {prompt: text});
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
async function moveWithRestart(source, target, sessionId, agentId, workspace, generation, forward) {
    const client = new LocalIpcClient(source.url, {timeoutMs: 1800000, localAuthEnvironment: source.env});
    let response;
    const request = forward ? {MoveAgentToRemote: {session_id: sessionId, agent_ref: agentId, machine_ref: target.status.daemon_id}} : {MoveAgentToLocal: {session_id: sessionId, agent_ref: agentId}};
    const pending = client.send(request).then(value => { response = value; }, error => { response = {error: String(error.message ?? error).slice(0, 700)}; });
    try {
        let captured;
        for (let n = 0; n < 60000; n++) {
            const value = await journal(source, generation);
            if (n % 400 === 0) {
                const snapshot = await source.tui.send('snapshot');
                await writeFile(out + '/move-progress-' + generation + '.json', JSON.stringify({phase: value?.phase.phase, interactions: (snapshot.interactions ?? []).map(i => ({kind: i.kind, title: i.title, choices: i.choices?.map(c => c.id)})), responseError: response?.error}) + '\n', {mode: 0o600});
            }
            if (value?.phase.phase === 'captured') { captured = value; break; }
            if (value?.phase.phase === 'rolled_back') throw Error('Move rolled back before capture: ' + value.phase.reason);
            if (value && ['materialized', 'bound', 'committed', 'complete'].includes(value.phase.phase)) throw Error('Missed mid-Move restart boundary: ' + value.phase.phase);
            if (response?.error) throw Error('Move rejected before capture: ' + response.error);
            if (response) throw Error('Move ended before the captured boundary: ' + Object.keys(response));
            await sleep(5);
        }
        assert.ok(captured, 'Move must durably capture its stopped source');
        assert.equal(captured.identity.target_kernel_id, forward ? target.status.daemon_id : source.status.daemon_id, 'Selected target identity');
        await record(forward ? 'forward-before-restart' : 'return-before-restart', {generation, operationId: captured.identity.operation_id, phase: captured.phase.phase});
        await restart(source, workspace, sessionId);
        // The old connection cannot acknowledge a request killed with its kernel.
        client.destroy();
        await pending;
        let completed;
        for (let n = 0; n < 1200; n++) {
            const value = await journal(source, generation);
            if (n % 40 === 0 || value?.phase.phase === 'rolled_back' || value?.phase.phase === 'complete') await record('move-after-restart-' + generation, {generation, phase: value?.phase.phase, reason: value?.phase.reason});
            if (value?.phase.phase === 'complete') { completed = value; break; }
            assert.notEqual(value?.phase.phase, 'rolled_back', 'Captured Move must recover its existing artifacts');
            await sleep(250);
        }
        assert.ok(completed, 'Restart must autonomously finish the same Move generation');
        return completed.phase.receipt;
    } finally { await client.close(); }
}
await mkdir(root, {recursive: false, mode: 0o700});
await mkdir(out, {recursive: true, mode: 0o700});
let failed;
try {
    assert.equal(Number(execFileSync(kernel, ['--print-local-daemon-protocol-version'], {encoding: 'utf8'}).trim()), 608);
    await record('identity', {head: process.env.CHARIOX_WORKSPACE_KERNEL_HEAD, kernelSha256: sha(await readFile(kernel)), tuiSha256: sha(await readFile(binary)), driverSha256: sha(await readFile(new URL(import.meta.url))), runtimeDriverSha256: sha(await readFile(new URL('./workspace-runtime.mjs', import.meta.url))), provider: 'codex', model: 'gpt-6-luna', effort: 'low'});
    await launchBrowser();
    const source = await runtime('source'), target = await runtime('target');
    await enroll(source); await enroll(target);
    const paths = await fixture();
    paths.workspace = dirname(execFileSync('git', ['-C', paths.primary, 'rev-parse', '--path-format=absolute', '--git-common-dir'], {encoding: 'utf8'}).trim());
    // A bounded real file keeps the public transfer in progress long enough to restart.
    await writeFile(paths.primary + '/restart-transfer-a.bin', randomBytes(12 * 1024 * 1024));
    await writeFile(paths.primary + '/restart-transfer-b.bin', randomBytes(12 * 1024 * 1024));
    const projectId = await project(source, paths);
    const approval = {primary: target.path + '/approval', notes: target.path + '/approval-notes'};
    await mkdir(approval.primary); await mkdir(approval.notes); await project(target, approval);
    await selection(source, target, projectId, paths);
    const snapshot = await source.tui.send('snapshot'), sessionId = snapshot.session.id, agentId = snapshot.session.focusedAgentId;
    const marker = 'move-' + randomBytes(10).toString('hex');
    await turn(source, 'Remember this marker for our next task: ' + marker + '. Reply only ACK. Do not use tools or create agents.', 'source-turn');
    const sourceRun = await currentRun(source, sessionId, agentId);
    const nativeId = sourceRun.provider_session_id;
    const sourceState = await state(source, sessionId), sourceAgent = sourceState.session.agents.find(a => a.id === agentId);
    await record('source-layout', {roomWorkspace: sourceState.session.workspace_id, roomWorktree: sourceState.session.worktree_id, agentWorkspace: sourceAgent.workspace_id, agentWorktree: sourceAgent.worktree_id, providerWorkingDirectory: sourceRun.working_directory, projectMembers: [paths.workspace, paths.notes]});
    assert.ok(nativeId);
    const webProof = await captureWeb(source, sessionId, agentId, 'source');
    const forward = await moveWithRestart(source, target, sessionId, agentId, paths.primary, 1, true);
    const movedState = await state(source, sessionId);
    const movedAgent = movedState.session.agents.find(a => a.id === agentId);
    assert.equal(movedAgent.remote_execution?.worker_kernel_id, target.status.daemon_id);
    assert.ok(movedAgent.remote_execution?.leased_agent_id, 'Move must continue through a leased agent');
    await record('leased-agent', {homeRoom: sessionId, homeAgent: agentId, targetKernel: target.status.daemon_id, leasedAgent: movedAgent.remote_execution.leased_agent_id});
    await captureTui(source, 'moved-idle');
    webProof.push(...await captureWeb(source, sessionId, agentId, 'moved-idle'));
    const primary = forward.repositories.find(r => r.role === 'primary').destinationPath;
    const notes = forward.repositories.find(r => r.role === 'supporting').destinationPath;
    assert.notEqual(primary, paths.primary);
    assert.equal(await readFile(primary + '/input', 'utf8'), await readFile(paths.primary + '/input', 'utf8'));
    await writeFile(primary + '/input', 'latest target work\n');
    await writeFile(notes + '/nested/document', 'latest target notes\n');
    await writeFile(paths.primary + '/input', 'divergent original home\n');
    const remoteTurn = await turn(source, 'Run a shell command to print your working directory, then repeat the marker from our previous task. Do not use any other tools or create agents.', 'target-turn');
    const targetRun = await currentRun(source, sessionId, agentId);
    assert.equal(targetRun.provider_session_id, nativeId, 'Forward Move must resume the original native thread');
    assert.equal(targetRun.working_directory, primary, 'Leased provider must use the receipt primary');
    assert.equal(targetRun.model, sourceRun.model, 'Move must preserve the provider model');
    assert.equal(targetRun.variant, sourceRun.variant, 'Move must preserve the provider effort');
    assert.ok(entries(remoteTurn.snapshot).some(row => row.role === 'assistant' && row.text?.includes(marker)), 'Native target must recall previous context');
    assert.ok(completedWorkspaceTool(entries(remoteTurn.snapshot), remoteTurn.promptId, primary), 'Target tool must execute in the receipt primary');
    await record('forward-verified', {sameRoom: remoteTurn.snapshot.session.id === sessionId, primary, directoryWorkspace: notes, markerRecalled: true, completedWorkspaceTool: true, nativeSessionId: targetRun.provider_session_id, providerWorkingDirectory: targetRun.working_directory, model: targetRun.model, effort: targetRun.variant});
    webProof.push(...await captureWeb(source, sessionId, agentId, 'target'));
    const returned = await moveWithRestart(source, target, sessionId, agentId, paths.primary, 2, false);
    await captureTui(source, 'returned-idle');
    const home = returned.repositories.find(r => r.role === 'primary').destinationPath;
    const homeNotes = returned.repositories.find(r => r.role === 'supporting').destinationPath;
    assert.notEqual(home, paths.primary); assert.notEqual(home, primary);
    assert.equal(await readFile(home + '/input', 'utf8'), 'latest target work\n');
    assert.equal(await readFile(homeNotes + '/nested/document', 'utf8'), 'latest target notes\n');
    assert.equal(await readFile(paths.primary + '/input', 'utf8'), 'divergent original home\n');
    const homeTurn = await turn(source, 'Run a shell command to print your working directory, then repeat the marker and the working directory from our previous task. Do not use any other tools or create agents.', 'home-turn');
    assert.ok(entries(homeTurn.snapshot).some(row => row.role === 'assistant' && row.text?.includes(marker) && row.text?.includes(primary)), 'Returned native context must retain the target turn');
    assert.ok(completedWorkspaceTool(entries(homeTurn.snapshot), homeTurn.promptId, home));
    assert.equal((await currentRun(source, sessionId, agentId)).provider_session_id, nativeId);
    webProof.push(...await captureWeb(source, sessionId, agentId, 'returned'));
    await record('summary', {result: webProof.every(value => value.result === 'PASS') ? 'PASS' : 'PARTIAL', nativeResult: 'PASS', web: webProof.map(({result, dpr}) => ({result, dpr})), forwardRestart: true, returnRestart: true, sameNativeId: true, sameHomeRoom: homeTurn.snapshot.session.id === sessionId, sameHomeAgent: homeTurn.snapshot.session.focusedAgentId === agentId, preservedDivergentHome: true, freshReturn: home, realProviderTurns: 3, hostedWss: true});
} catch (error) {
    failed = error; await record('summary', {result: 'FAIL', error: String(error), stack: error.stack});
} finally { await close(); release(); }
if (failed) throw failed;
