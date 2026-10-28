// MP-08 / MP-09 / MP-10 / MP-11: a divergent home thread leaves Return cancellable.
import assert from 'node:assert/strict';
import {randomBytes} from 'node:crypto';
import {mkdir, readFile, writeFile} from 'node:fs/promises';
import {root, out, kernel, binary, sha, record, runtime, enroll, project, selection, launchBrowser, close, release} from './workspace-runtime.mjs';
import {fixture} from './workspace-fixture.mjs';
import {turn, currentRun, journal, state, entries, variant} from './native-move-proof.mjs';

async function nativePath(run) {
    const endpoint = new URL(run.structured_endpoint);
    assert.equal(endpoint.protocol, 'ws:');
    assert.equal(endpoint.hostname, '127.0.0.1');
    const socket = new WebSocket(endpoint);
    async function request(id, method, params) {
        return await new Promise((resolve, reject) => {
            const timer = setTimeout(() => { socket.removeEventListener('message', receive); reject(Error('Native metadata deadline')); }, 15000);
            function receive(event) {
                const reply = JSON.parse(event.data);
                if (reply.id !== id) return;
                clearTimeout(timer); socket.removeEventListener('message', receive);
                if (reply.error) reject(Error('Native metadata request failed')); else resolve(reply.result);
            }
            socket.addEventListener('message', receive);
            socket.send(JSON.stringify({id, method, params}));
        });
    }
    try {
        await new Promise((resolve, reject) => {
            socket.addEventListener('open', resolve, {once: true});
            socket.addEventListener('error', () => reject(Error('Native metadata connection failed')), {once: true});
        });
        await request(1, 'initialize', {clientInfo: {name: 'return-continuity', version: '1'}});
        const result = await request(2, 'thread/read', {threadId: run.provider_session_id, includeTurns: false});
        assert.equal(result.thread.id, run.provider_session_id);
        const path = result.thread.path;
        assert.ok(path?.includes('/sessions/') && path.endsWith('-' + run.provider_session_id + '.jsonl'));
        return path;
    } finally { socket.close(); }
}

await mkdir(root, {recursive: false, mode: 0o700});
await mkdir(out, {recursive: true, mode: 0o700});
let failed;
try {
    await record('identity', {head: process.env.CHARIOX_WORKSPACE_KERNEL_HEAD, kernelSha256: sha(await readFile(kernel)), tuiSha256: sha(await readFile(binary))});
    await launchBrowser();
    const source = await runtime('source'), target = await runtime('target');
    await enroll(source); await enroll(target);
    const paths = await fixture(), projectId = await project(source, paths);
    const approval = {primary: target.path + '/approval', notes: target.path + '/approval-notes'};
    await mkdir(approval.primary); await mkdir(approval.notes); await project(target, approval);
    const targetSession = (await target.tui.send('snapshot')).session.id;
    await target.tui.close(); target.tui = undefined;
    await target.client.send({DeleteSession: {session_ref: targetSession, workspace_id: null}});
    await target.client.send({RemoveProviderAccountProfile: {provider: 'codex', account_profile: target.accountProfile.profile_id}});
    await selection(source, target, projectId, paths);
    const snapshot = await source.tui.send('snapshot'), sessionId = snapshot.session.id, agentId = snapshot.session.focusedAgentId;
    const marker = 'return-' + randomBytes(10).toString('hex');
    await turn(source, 'Remember this marker: ' + marker + '. Reply only ACK. Do not use tools or create agents.', 'home-turn');
    const initial = await currentRun(source, sessionId, agentId), homePath = await nativePath(initial);
    await source.client.send({MoveAgentToRemote: {session_id: sessionId, agent_ref: agentId, machine_ref: target.status.daemon_id}});
    await turn(source, 'Repeat our marker. Do not use tools or create agents.', 'remote-turn');
    const remote = await currentRun(source, sessionId, agentId);
    const original = (await state(source, sessionId)).session.agents.find(a => a.id === agentId);
    assert.equal(remote.provider_session_id, initial.provider_session_id);
    // The fixture advances only the native thread created above. No credential or
    // unrelated provider file is read or changed; both divergent histories must survive.
    const homeHistory = Buffer.concat([await readFile(homePath), Buffer.from(JSON.stringify({timestamp: new Date().toISOString(), type: 'event_msg', payload: {type: 'user_message', message: 'independent home continuation', images: [], local_images: []}}) + '\n')]);
    await writeFile(homePath, homeHistory);
    const leasedPrefix = 'leased:' + original.remote_execution.leased_agent_id + ':';
    const worker = variant(await target.client.send({GetProviderRun: {provider_run_id: remote.id.slice(leasedPrefix.length)}}), 'ProviderRun').provider_run;
    const remotePath = await nativePath(worker), remoteHistory = await readFile(remotePath);
    assert.ok(!homeHistory.equals(remoteHistory), 'The two native histories must diverge before Return');
    const refused = await source.client.send({MoveAgentToLocal: {session_id: sessionId, agent_ref: agentId}}).then(() => null, error => String(error));
    assert.match(refused ?? '', /divergent state/);
    assert.equal((await journal(source, 2)).phase.phase, 'bound');
    await source.client.send({MoveAgentToLocal: {session_id: sessionId, agent_ref: agentId, abort_pending: true}});
    assert.equal((await journal(source, 2)).phase.phase, 'rolled_back');
    const restored = (await state(source, sessionId)).session.agents.find(a => a.id === agentId);
    assert.ok(JSON.stringify(restored.remote_execution) === JSON.stringify(original.remote_execution), 'Cancellation must restore the source route');
    assert.equal(restored.workspace_id, original.workspace_id);
    assert.equal(restored.worktree_id, original.worktree_id);
    assert.ok((await readFile(homePath)).equals(homeHistory), 'Cancellation must preserve divergent home history');
    assert.ok((await readFile(remotePath)).equals(remoteHistory), 'Cancellation must preserve the stopped remote source history');
    const continued = await turn(source, 'Repeat our marker. Do not use tools or create agents.', 'remote-after-cancellation');
    assert.equal((await currentRun(source, sessionId, agentId)).provider_session_id, initial.provider_session_id);
    assert.ok(entries(continued.snapshot).some(row => row.role === 'assistant' && row.text?.includes(marker)));
    await record('return-conflict-cancelled', {result: 'PASS', homeHistorySha256: sha(homeHistory), sourceHistorySha256: sha(remoteHistory), sourceTurnAccepted: true, sameNativeSession: true, routeRestored: true});
} catch (error) { failed = error; await record('failure', {message: String(error.message ?? error).slice(0, 800)}); }
finally { await close(); release(); }
if (failed) throw failed;
