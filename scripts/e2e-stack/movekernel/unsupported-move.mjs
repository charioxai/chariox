// MP-08 / MP-09 / MP-10 / MP-11: real Claude history must refuse Move before teardown.
import assert from 'node:assert/strict';
import {mkdir, readFile, readdir, writeFile} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {root, out, kernel, binary, sha, record, runtime, enroll, project, selection, launchBrowser, close, release} from './workspace-runtime.mjs';
import {fixture} from './workspace-fixture.mjs';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const variant = (reply, key) => {assert.ok(reply[key], `Expected ${key}`); return reply[key];};
const entries = snapshot => snapshot.agentPanes?.[snapshot.session.focusedAgentId] ?? [];
async function turn(source, prompt, label) {
    const before = new Set(entries(await source.tui.send('snapshot')).map(row => row.id));
    await source.tui.send('submit_prompt', {prompt});
    let settled;
    for (let n = 0; n < 720; n++) {
        const snapshot = await source.tui.send('snapshot');
        const agent = snapshot.session.agents.find(a => a.id === snapshot.session.focusedAgentId);
        const reply = entries(snapshot).some(row => !before.has(row.id) && ['assistant', 'error'].includes(row.role) && row.text?.trim());
        if (!agent?.isProcessing && agent?.state !== 'Working' && reply) {
            settled ??= Date.now();
            if (Date.now() - settled > 1500) {
                assert.notEqual(agent.state, 'Error', 'Claude turn must complete');
                const ansi = out + '/' + label + '.ansi';
                await writeFile(ansi, source.tui.output(), {mode: 0o600});
                execFileSync('python3', [new URL('../context-switch/render-pty.py', import.meta.url).pathname, ansi], {stdio: 'ignore'});
                return snapshot;
            }
        } else settled = undefined;
        await sleep(250);
    }
    throw Error('Real Claude turn deadline: ' + label);
}
async function run(source, sessionId, agentId) {
    const core = variant(await source.client.send({GetSessionState: {session_id: sessionId}}), 'SessionState');
    const agent = core.session.agents.find(a => a.id === agentId);
    const id = agent.active_provider_run_id ?? core.agent_activity?.[agentId]?.last_completed_turn?.provider_run_id;
    assert.ok(id, 'A real official provider run is required');
    return variant(await source.client.send({GetProviderRun: {provider_run_id: id}}), 'ProviderRun').provider_run;
}
await mkdir(root, {recursive: false, mode: 0o700});
await mkdir(out, {recursive: true, mode: 0o700});
let failed;
try {
    await record('identity', {head: process.env.CHARIOX_WORKSPACE_KERNEL_HEAD, kernelSha256: sha(await readFile(kernel)), tuiSha256: sha(await readFile(binary)), provider: 'claude-p', model: 'claude-haiku-5-5'});
    await launchBrowser();
    const source = await runtime('source'), target = await runtime('target');
    await enroll(source); await enroll(target);
    const paths = await fixture();
    const projectId = await project(source, paths);
    const approval = {primary: target.path + '/approval', notes: target.path + '/approval-notes'};
    await mkdir(approval.primary); await mkdir(approval.notes); await project(target, approval);
    await selection(source, target, projectId, paths);
    const initial = await source.tui.send('snapshot'), sessionId = initial.session.id, agentId = initial.session.focusedAgentId;
    const profile = variant(await source.client.send({CreateProviderAccountProfile: {provider: 'claude', label: 'Move refusal'}}), 'ProviderAccountProfile').profile;
    const chunks = [];
    for await (const chunk of process.stdin) chunks.push(chunk);
    const value = Buffer.concat(chunks).toString('utf8').trim();
    assert.ok(value, 'Existing product setup credential must be supplied on stdin');
    await source.client.send({SetProviderAccountCredential: {provider: 'claude', account_profile: profile.profile_id, value, run: false, overwrite: false}});
    await source.client.send({RefreshProviderAccountProfile: {provider: 'claude', account_profile: profile.profile_id}});
    await source.client.send({UpdateAgentProfile: {session_id: sessionId, agent_id: agentId, provider: 'claude-p', account_profile: profile.profile_id, model: 'claude-haiku-5-5', clear_effort: true}});
    const marker = 'unsupported-move-history';
    await turn(source, 'Remember this marker and reply with it: ' + marker + '. Do not use tools or create agents.', 'claude-before-move');
    const before = await run(source, sessionId, agentId);
    assert.ok(before.provider_session_id, 'Claude native history must exist');
    assert.ok(before.model.startsWith('claude-haiku-5-5'), 'Resolved validation model must be Haiku5.5');
    let reason;
    try {await source.client.send({MoveAgentToRemote: {session_id: sessionId, agent_ref: agentId, machine_ref: target.status.machine_id}});}
    catch (error) {reason = String(error);}
    assert.ok(reason?.includes('native checkpoint support'), 'Unsupported history must refuse precisely');
    const after = await run(source, sessionId, agentId);
    assert.equal(after.id, before.id, 'Refusal must retain the provider run');
    assert.equal(after.state, before.state, 'Refusal must happen before stopping the provider');
    assert.equal(after.provider_session_id, before.provider_session_id);
    const operations = (await readdir(source.path + '/home/agent-moves')).filter(name => name.endsWith('.home.json'));
    assert.equal(operations.length, 0, 'Unsupported history must be refused before any Move generation is fenced');
    const resumed = await turn(source, 'Repeat the marker from our previous task. Do not use tools or create agents.', 'claude-after-refusal');
    assert.ok(entries(resumed).some(row => row.role === 'assistant' && row.text?.includes(marker)), 'Source remains usable with its original native history');
    await record('summary', {result: 'PASS', refusalBeforeStop: true, sameProviderRun: true, sameNativeSession: true, model: after.model, realProviderTurns: 2, hostedWss: true});
} catch (error) {failed = error; await record('failure', {message: String(error)});}
finally {await close(); release();}
if (failed) throw failed;
