// MP-08 / MP-09 / MP-10 / MP-11: real Return failures, cancellation and queued admission.
import assert from 'node:assert/strict';
import {mkdir, writeFile, readFile, rm, symlink, unlink} from 'node:fs/promises';
import {LocalIpcClient} from '../../../packages/kernel-client/dist/ipc.js';
import {record, restart, sha} from './workspace-runtime.mjs';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const refusal = async (client, request) => {try {return await client.send(request);} catch (error) {return {error: String(error)};}};
const entries = snapshot => snapshot.agentPanes?.[snapshot.session.focusedAgentId] ?? [];

export async function exerciseReturnFailures({source, target, sessionId, agentId, primary, approval, nativeId, marker, turn, currentRun, journal}) {
    const request = {MoveAgentToLocal: {session_id: sessionId, agent_ref: agentId}};
    const approvalSession = (await target.tui.send('snapshot')).session.id;
    await restart(target, approval.primary, approvalSession);
    const unavailable = await refusal(source.client, request);
    assert.ok(unavailable.error?.includes('after restart'), 'Return must explain the unavailable restarted provider');
    const afterRestart = await journal(source, 2);
    assert.ok(!afterRestart || afterRestart.phase.phase === 'rolled_back', 'A Return from a restarted post-turn target must leave source admission open');
    await record('return-restarted-source-refusal', {result: 'PASS', reason: 'source run unavailable after restart', pendingFence: false});
    await turn(source, 'Repeat the marker from our previous task. Do not use tools or create agents.', 'restart-source-usable');
    assert.equal((await currentRun(source, sessionId, agentId)).provider_session_id, nativeId);

    const outside = target.path + '/outside-move-project';
    const link = primary + '/invalid-return-link';
    await writeFile(outside, 'preserve outside Project\n');
    await symlink(outside, link);
    const failed = await refusal(source.client, request);
    assert.ok(failed.error?.includes('symbolic links'), 'Snapshot must fail after fencing on its actual source path');
    const rolledBack = await journal(source, 2);
    assert.equal(rolledBack.phase.phase, 'rolled_back', 'Failed Return must durably roll back its fence');
    assert.equal(await readFile(outside, 'utf8'), 'preserve outside Project\n');
    await unlink(link);
    await turn(source, 'Repeat the marker from our previous task. Do not use tools or create agents.', 'rollback-source-usable');
    assert.equal((await currentRun(source, sessionId, agentId)).provider_session_id, nativeId);
    await record('return-fenced-failure-rollback', {result: 'PASS', phase: rolledBack.phase.phase, sourceTurnAccepted: true, sameNativeSession: true});

    const client = new LocalIpcClient(source.url, {timeoutMs: 1800000, localAuthEnvironment: source.env});
    let response;
    const pending = client.send(request).then(value => {response = value;}, error => {response = {error: String(error)};});
    let captured;
    for (let n = 0; n < 6000; n++) {
        const value = await journal(source, 3);
        if (value?.phase.phase === 'captured') {captured = value; break;}
        assert.ok(!response, 'Return must reach capture before target publication');
        await sleep(5);
    }
    assert.ok(captured, 'Real Return must capture the stopped native source');
    const destination = source.env.HOME + '/chariox-moves/move-' + sha(Buffer.from(captured.identity.operation_id));
    await mkdir(destination, {recursive: true});
    await writeFile(destination + '/preserve-marker', marker);
    await pending;
    assert.ok(response.error, 'Occupied Return destination must refuse publication');
    assert.equal((await journal(source, 3)).phase.phase, 'captured');
    const prompt = 'Repeat the marker from our previous task. Do not use tools or create agents.';
    const before = new Set(entries(await source.tui.send('snapshot')).map(row => row.id));
    await source.tui.send('submit_prompt', {prompt});
    const aborted = await source.client.send({MoveAgentToLocal: {session_id: sessionId, agent_ref: agentId, abort_pending: true}});
    assert.ok(aborted.AgentMoveAborted, 'Explicit abort must restore captured source ownership');
    assert.equal((await journal(source, 3)).phase.phase, 'rolled_back');
    assert.equal(await readFile(destination + '/preserve-marker', 'utf8'), marker, 'Abort must preserve the conflicting directory');
    await turn(source, prompt, 'abort-queued-source-usable', {submitted: true, before});
    assert.equal((await currentRun(source, sessionId, agentId)).provider_session_id, nativeId);
    await record('return-captured-abort-and-queued-prompt', {result: 'PASS', sourceTurnAccepted: true, queuedDuringMove: true, sameNativeSession: true, independentConflictPreserved: true});
    await rm(destination, {recursive: true});
    await client.close();
    return {generation: 4, extraTurns: 3};
}
