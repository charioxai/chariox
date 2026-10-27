// MP-08 / MP-09 / MP-10 / MP-11: real account, packages, Project setup and native Move.
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { mkdir, writeFile, readFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { root, out, kernel, binary, sha, record, runtime, enroll, project, selection, launchBrowser, close, release } from './workspace-runtime.mjs';
import { fixture } from './workspace-fixture.mjs';
import { sleep, entries, variant, state, currentRun, accountIdentity, turn, journal, assertTuiRoute } from './native-move-proof.mjs';
import { completedWorkspaceTool } from './workspace-proof.mjs';

async function installProofPackages(source, agentId) {
    const server = root + '/proof-mcp.mjs';
    await writeFile(server, `import readline from 'node:readline';
const input=readline.createInterface({input:process.stdin});
input.on('line',line=>{const q=JSON.parse(line);if(q.id===undefined)return;
let result={};
if(q.method==='initialize')result={protocolVersion:q.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:'mk04-proof',version:'1'}};
else if(q.method==='tools/list')result={tools:[{name:'mk04_ping',description:'Return the supplied marker and the execution kernel home',inputSchema:{type:'object',properties:{marker:{type:'string'}},required:['marker']}}]};
else if(q.method==='tools/call')result={content:[{type:'text',text:JSON.stringify({marker:q.params.arguments.marker,executionHome:process.env.CHARIOX_HOME})}]};
process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:q.id,result})+'\\n');});\n`);
    await source.client.send({InstallMcpServer: {workspace_id: null, config: {name: 'mk04-proof', transport: {type: 'stdio', command: process.execPath, args: [server]}, enabled: true, required: true}}});
    const skill = root + '/proof-skill';
    await mkdir(skill);
    await writeFile(skill + '/SKILL.md', '---\nname: mk04-proof\ndescription: Execute the requested Move skill proof\n---\nWhen asked for the MK04 proof, run a shell command that writes exactly mk04-skill-executed to mk04-skill-proof.txt in the current working directory.\n');
    const installed = variant(await source.client.send({InstallSkill: {workspace_id: null, source_path: skill}}), 'SkillInstalled');
    for (const kind of ['mcp', 'skill']) await source.client.send({GrantAgentExtension: {workspace_id: null, agent_ref: agentId, kind, name: 'mk04-proof'}});
    await record('packages-installed', {skill: installed.skill.name, homeMcp: 'mk04-proof', grants: ['mcp', 'skill']});
}
async function setupProject(source, projectId, sessionId, agentId) {
    const operationId = 'mk04-source-setup';
    const definition = {schema_version: 1, origin: 'user_authored', source: 'commands', target_platform: 'linux-x86_64', source_path: null, inputs: [], path_entries: [], setup_steps: [{kind: 'command', command: 'pwd > .mk04-env-ready'}], validation_commands: ['test -s .mk04-env-ready']};
    await source.client.send({StartProjectEnvironmentSetup: {operationId, projectId, sessionId, agentId, targetWorkerId: source.status.machine_id, targetPlatform: definition.target_platform, definition, validationCommands: []}});
    for (let n = 0; n < 960; n++) {
        const status = variant(await source.client.send({GetProjectEnvironmentSetupStatus: {operationId}}), 'ProjectEnvironmentSetupStatus').status;
        if (status.phase === 'ready') { await record('source-setup', {operationId, phase: status.phase}); return; }
        assert.ok(!['failed', 'cancelled'].includes(status.phase), 'Source Project setup must be ready: ' + status.failure_code);
        await sleep(250);
    }
    throw Error('Source Project setup deadline');
}
await mkdir(root, {recursive: false, mode: 0o700});
await mkdir(out, {recursive: true, mode: 0o700});
let failed;
try {
    const protocol = Number(process.env.CHARIOX_WORKSPACE_PROTOCOL ?? 634);
    assert.equal(Number(execFileSync(kernel, ['--print-local-daemon-protocol-version'], {encoding: 'utf8'}).trim()), protocol);
    await record('identity', {head: process.env.CHARIOX_WORKSPACE_KERNEL_HEAD, kernelSha256: sha(await readFile(kernel)), tuiSha256: sha(await readFile(binary)), protocol, provider: 'codex', model: 'gpt-6-luna', effort: 'low'});
    await launchBrowser();
    const source = await runtime('source'), target = await runtime('target');
    await enroll(source); await enroll(target);
    const paths = await fixture();
    await writeFile(paths.primary + '/.gitignore', 'mk04-skill-proof.txt\n');
    const projectId = await project(source, paths);
    const approval = {primary: target.path + '/approval', notes: target.path + '/approval-notes'};
    await mkdir(approval.primary); await mkdir(approval.notes); await project(target, approval);
    const targetSession = (await target.tui.send('snapshot')).session.id;
    await target.tui.close(); target.tui = undefined;
    await target.client.send({DeleteSession: {session_ref: targetSession, workspace_id: null}});
    await target.client.send({RemoveProviderAccountProfile: {provider: 'codex', account_profile: target.accountProfile.profile_id}});
    const beforeProfiles = variant(await target.client.send({ListProviderAccountProfiles: {provider: 'codex'}}), 'ProviderAccountProfilesListed').profiles;
    assert.ok(!beforeProfiles.some(p => p.auth_state === 'authenticated' || p.profile_id === source.accountProfile.profile_id), 'Target must begin without the selected authenticated Codex account');
    await selection(source, target, projectId, paths);
    const snapshot = await source.tui.send('snapshot'), sessionId = snapshot.session.id, agentId = snapshot.session.focusedAgentId;
    await installProofPackages(source, agentId);
    const marker = 'mk04-' + randomBytes(10).toString('hex');
    await turn(source, 'Remember this marker for the next task: ' + marker + '. Reply only ACK. Do not use tools or create agents.', 'source-turn');
    const sourceRun = await currentRun(source, sessionId, agentId), nativeId = sourceRun.provider_session_id;
    assert.ok(nativeId, 'Source must have a durable native session');
    const account = await accountIdentity(sourceRun);
    await setupProject(source, projectId, sessionId, agentId);
    await source.client.send({MoveAgentToRemote: {session_id: sessionId, agent_ref: agentId, machine_ref: target.status.machine_id}});
    await assertTuiRoute(source, agentId, target.status.daemon_id, 'composed-move');
    const operation = await journal(source, 1), receipt = operation.phase.receipt;
    assert.equal(operation.phase.phase, 'complete');
    assert.deepEqual(operation.composition.commit.readiness, operation.composition.readiness);
    assert.ok(operation.composition.readiness.environment_setup_operation_id, 'Target-native Project setup receipt is required');
    const primary = receipt.repositories.find(r => r.repository_id === receipt.primary_repository_id).destination_path;
    assert.equal((await readFile(primary + '/.mk04-env-ready', 'utf8')).trim(), primary, 'Source setup output must have been rewritten by target setup');
    const profiles = variant(await target.client.send({ListProviderAccountProfiles: {provider: 'codex'}}), 'ProviderAccountProfilesListed').profiles;
    assert.equal(profiles.filter(p => p.auth_state === 'authenticated').length, 1, 'Move must transfer exactly its selected authenticated account');
    await record('resource-receipts', {requirements: operation.composition.requirements, readiness: operation.composition.readiness, commit: operation.composition.commit, accountTransferred: true, targetNativeSetup: true});
    const result = await turn(source, 'Perform the MK04 proof using the granted mk04-proof skill, call the mk04_ping tool with our remembered marker, then run pwd. Reply with the remembered marker. Do not create agents.', 'target-turn');
    assert.equal(await readFile(primary + '/mk04-skill-proof.txt', 'utf8'), 'mk04-skill-executed');
    assert.ok(completedWorkspaceTool(entries(result.snapshot), result.promptId, primary));
    const toolRows = entries(result.snapshot).filter(row => row.role === 'tool' && row.promptId === result.promptId);
    assert.ok(toolRows.some(row => row.text?.includes('mk04_ping') && row.text.includes(marker) && row.text.includes(source.path + '/home')), 'Granted MCP must execute through home forwarding');
    const moved = (await state(source, sessionId)).session.agents.find(a => a.id === agentId);
    const projected = await currentRun(source, sessionId, agentId);
    const prefix = 'leased:' + moved.remote_execution.leased_agent_id + ':';
    assert.ok(projected.id.startsWith(prefix), 'Projected run must belong to the selected Move lease');
    const worker = variant(await target.client.send({GetProviderRun: {provider_run_id: projected.id.slice(prefix.length)}}), 'ProviderRun').provider_run;
    assert.equal(worker.provider_session_id, nativeId);
    assert.equal(await accountIdentity(worker), account);
    assert.equal(worker.model, 'gpt-6-luna'); assert.equal(worker.variant, 'low');
    await record('summary', {result: 'PASS', sameNativeSession: true, sameAuthenticatedAccount: true, transferredSelectedAccount: true, grantedSkillExecuted: true, homeMcpExecuted: true, targetNativeSetup: true, sameHomeRoom: result.snapshot.session.id === sessionId, sameHomeAgent: result.snapshot.session.focusedAgentId === agentId, realProviderTurns: 2, targetProviderTurns: 1, hostedWss: true});
} catch (error) { failed = error; await record('summary', {result: 'FAIL', error: String(error), stack: error.stack}); }
finally { await close(); release(); }
if (failed) throw failed;
