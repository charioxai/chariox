// MP-08 / MP-09 / MP-10 / MP-11: regression checks, supplementary to live drills.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm, chmod, mkdir, copyFile, access, readlink } from 'node:fs/promises';
import { execFileSync, spawn } from 'node:child_process';
import { once } from 'node:events';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { run, validateAcceptance, verifyOwnedPid, LaneError, execute } from './lanes.mjs';
import { enforceCoordinatorDeadline } from './coordinator-deadline.mjs';
import { waitForIdle } from './watch.mjs';

test('acceptance rows require full ids, known status and boolean new', () => {
  const row = { id: 'external-agents-v2/EA2-16R2', status: 'PASS', head: 'a'.repeat(40), date: '2026-10-23', evidence: '/evidence', note: 'MP-11', new: true };
  assert.deepEqual(validateAcceptance({ rows: [row] }), { rows: [row] });
  for (const bad of [{ id: 'EA2-16R2' }, { status: 'green' }, { new: 'true' }, { status: undefined }, { result: 'PASS', status: undefined }, { head: 'short' }]) {
    assert.throws(() => validateAcceptance({ rows: [{ ...row, ...bad }] }));
  }
});

test('steer transmits the expected prompt fence without submitting a new prompt', async () => {
  const calls = [];
  const execute = async (argv, stdin) => { calls.push({ argv, request: JSON.parse(stdin) }); return { PromptSteered: { accepted: true } }; };
  await run('steer', { kernel: 'b', taskId: 'lane', sessionId: 's', attachmentId: 't', agentId: 'a', expectedPromptId: 'p', steerId: 'steer-1', prompt: 'MP-11: steer' }, { execute });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].request.SteerActivePrompt.expected_active_prompt_id, 'p');
  assert.ok(calls[0].argv.includes('--command-id'));
  await assert.rejects(run('steer', { kernel: 'b', taskId: 'lane', sessionId: 's', attachmentId: 't', agentId: 'a', prompt: 'x' }, { execute }));
});

test('launch persists partial ownership, refreshes, uploads brief and arms before prompt', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-launch-'));
  try {
    const calls = [];
    const execute = async (argv, stdin) => {
      if (argv[0] === 'files') { calls.push('files put'); return { sha256: 'a'.repeat(64) }; }
      const req = JSON.parse(stdin); const op = Object.keys(req)[0]; calls.push(op);
      return {
        RelayStatus: { RelayStatus: { status: { capabilities: ['agent_deadline_v1'] } } }, GetAgentDeadline: { AgentDeadline: { deadline: { state: 'armed' } } }, CreateSession: { SessionCreated: { session: { id: 's' } } },
        AttachToSession: { SessionAttached: { attachment: { id: 't' } } },
        RefreshProviderAccountProfile: { ProviderAccountProfile: { profile: { auth_state: 'authenticated' } } },
        SpawnAgent: { AgentSpawned: { agent: { id: 'a' } } },
        SetAgentDeadline: { AgentDeadline: { deadline: { state: 'armed' } } },
        SubmitPrompt: { PromptSubmitted: { outcome: { Started: { prompt: { id: 'p' } } } } },
      }[op];
    };
    const brief = join(dir, 'brief.md'); await writeFile(brief, 'MP-11: task');
    const stateFile = join(dir, 'launch.json');
    const result = await run('launch', { kernel: 'b', taskId: 'lane', alias: 'lane', worktree: '/repo', provider: 'codex', profile: 'profile', model: 'gpt-6-luna', effort: 'low', permission: 'required', brief, remoteBrief: '/repo/BRIEF.md', stateFile, deadlineMs: Date.now() + 60_000 }, { execute });
    assert.equal(result.agentId, 'a');
    assert.deepEqual(calls, ['CreateSession', 'AttachToSession', 'RefreshProviderAccountProfile', 'SpawnAgent', 'files put', 'RelayStatus', 'SetAgentDeadline', 'GetAgentDeadline', 'SubmitPrompt']);
    assert.equal(JSON.parse(await readFile(stateFile, 'utf8')).agentId, 'a');
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('slot rejects a timed-out wait and never reports a running command as passed', async () => {
  let closed = false;
  const execute = async argv => {
    if (argv[1] === 'open') return { terminal: { terminal_id: 't' } };
    if (argv[1] === 'close') { closed = true; return {}; }
    return { fired: null };
  };
  await assert.rejects(run('slot', { kernel: 'b', taskId: 'lane', sessionId: 's', worktree: '/repo', slotScript: '/agents/cargo-slot.sh', argv: ['cargo', 'test'], timeout: 1 }, { execute }), { code: 'terminal_wait_timeout' });
  assert.equal(closed, true);
});

test('PID cleanup refuses special ids, missing ownership and recycled identities', async () => {
  for (const pid of [undefined, NaN, -1, 0, 1, '2']) await assert.rejects(verifyOwnedPid({ pid }));
  await assert.rejects(verifyOwnedPid({ pid: process.pid, startTime: 'wrong', bootId: 'wrong', owner: 'other' }, 'lane'));
});

test('bundle refuses invalid heads before opening any terminal', async () => {
  const execute = async () => { throw Error('must not execute'); };
  await assert.rejects(run('bundle', { kernel: 'b', taskId: 'lane', sessionId: 's', worktree: '/repo', base: 'main;touch /bad', head: 'a'.repeat(40) }, { execute }));
});

test('CLI failures never echo untrusted provider diagnostics', async () => {
  await assert.rejects(execute(['-e', 'process.stderr.write(JSON.stringify({error:{code:"provider_profile_unverified",message:"PRIVATE-CANARY"}}));process.exit(1)'], '', process.execPath), error => error.code === 'provider_profile_unverified' && !error.message.includes('PRIVATE-CANARY'));
});

test('a transport retry uses the same command id and body exactly once', async () => {
  const calls = [];
  const input = { kernel: 'b', taskId: 'lane', sessionId: 's', attachmentId: 't', agentId: 'a', expectedPromptId: 'p', steerId: 'steer-1', prompt: 'MP-11: steer' };
  await run('steer', input, { execute: async (argv, body) => { calls.push([argv, body]); if (calls.length === 1) throw new LaneError('kernel_connection_lost'); return {}; } });
  assert.deepEqual(calls[0], calls[1]);
  assert.equal(calls.length, 2);
  await assert.rejects(run('steer', input, { execute: async () => { throw new LaneError('prompt_steer_rejected'); } }), { code: 'prompt_steer_rejected' });
});

test('status selects the target agent and never borrows another agents prompt', async () => {
  const result = await run('status', { kernel: 'b', taskId: 'lane', sessionId: 's', agentId: 'a' }, { execute: async (_, body) => 'RelayStatus' in JSON.parse(body) ? { RelayStatus: { status: { capabilities: ['agent_deadline_v1'] } } } : JSON.parse(body).GetSessionState ? { SessionState: { agents: [{ id: 'a', is_processing: false, last_activity_at_ms: 123 }], session: { active_prompt: { id: 'other', target_agent_id: 'b' } } } } : { AgentDeadline: { deadline: { state: 'reached' } } } });
  assert.equal(result.currentPrompt, null);
  assert.equal(result.state, 'idle');
  assert.equal(result.lastActivityMs, 123);
  assert.deepEqual(result.capabilities, ['agent_deadline_v1']);
});

test('stop destroys an idle agent after typed no-active-prompt refusal', async () => {
  const calls = [];
  const result = await run('stop', { kernel: 'b', taskId: 'lane', sessionId: 's', agentId: 'a', attachmentId: 't' }, { execute: async (_, body) => { const name = Object.keys(JSON.parse(body))[0]; calls.push(name); if (name === 'CancelActivePrompt') throw new LaneError('no_active_prompt'); return { AgentsListed: { agents: [] } }; } });
  assert.deepEqual(calls, ['CancelActivePrompt', 'DestroyAgent', 'ListAgents']);
  assert.equal(result.stopped, true);
});

test('fetch downloads all three artifacts and names each malformed acceptance field', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-fetch-'));
  try {
    const execute = async argv => { await writeFile(argv.at(-1), argv.at(-2).endsWith('.json') ? '{"rows":[{"id":"SHORT","status":"PASS"}]}' : 'MP-11: notes'); return {}; };
    const files = await run('fetch', { kernel: 'b', taskId: 'lane', worktree: '/repo', directory: dir }, { execute });
    assert.equal(files['ACCEPTANCE_UPDATE.json'].acceptance.valid, false);
    assert.deepEqual(files['ACCEPTANCE_UPDATE.json'].acceptance.issues.map(i => i.field), ['rows[0].id', 'rows[0].head', 'rows[0].date']);
    assert.match(await readFile(join(dir, 'PUSH_READY.md'), 'utf8'), /MP-11/);
    const broken = async argv => { await writeFile(argv.at(-1), argv.at(-2).endsWith('.json') ? '{not json' : 'notes'); return {}; };
    const bad = await run('fetch', { kernel: 'b', taskId: 'lane', worktree: '/repo', directory: dir }, { execute: broken });
    assert.deepEqual(bad['ACCEPTANCE_UPDATE.json'].acceptance.issues, [{ field: '$', problem: 'is not valid JSON' }]);
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('fetch keeps going when a stopped worker never wrote its handoff files', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-fetch-'));
  try {
    const execute = async argv => { if (argv.at(-2).endsWith('PUSH_READY.md')) throw new LaneError('file_not_found'); await writeFile(argv.at(-1), argv.at(-2).endsWith('.json') ? '{"rows":[]}' : 'notes'); return {}; };
    const files = await run('fetch', { kernel: 'b', taskId: 'lane', worktree: '/repo', directory: dir }, { execute });
    assert.deepEqual(files['PUSH_READY.md'], { missing: true, reason: 'file_not_found' });
    assert.ok(files['LANE_STATUS.md'].sha256);
  } finally { await rm(dir, { recursive: true, force: true }); }
});

// The two files the workers really wrote in dogfood r9 (amint r19, dictation r2): both ended finished_collect_failed.
for (const [lane, field] of [['amint', 'rows[0].head'], ['dictation', 'rows[0].head']]) {
  test(`fetch keeps the real ${lane} acceptance file and names the head it cannot use`, async () => {
    const dir = await mkdtemp(join(tmpdir(), 'dogfood-fetch-'));
    try {
      const source = new URL(`./fixtures/r9-${lane}-ACCEPTANCE_UPDATE.json`, import.meta.url).pathname;
      const execute = async argv => { if (argv.at(-2).endsWith('.json')) await copyFile(source, argv.at(-1)); else await writeFile(argv.at(-1), 'notes'); return {}; };
      const files = await run('fetch', { kernel: 'b', taskId: 'lane', worktree: '/repo', directory: dir }, { execute });
      const { acceptance } = files['ACCEPTANCE_UPDATE.json'];
      assert.equal(acceptance.valid, false);
      assert.equal(acceptance.issues[0].field, field);
      assert.match(acceptance.issues[0].problem, /40-character/);
      assert.equal(JSON.parse(await readFile(join(dir, 'ACCEPTANCE_UPDATE.json'), 'utf8')).rows.length > 0, true);
    } finally { await rm(dir, { recursive: true, force: true }); }
  });
}

test('status lists the decisions the target agent is waiting on, and only its own', async () => {
  const active = [{ id: 'perm-1', agent_id: 'a', kind: 'permission', level: 'warning', title: 'Approve Claude Code Write?', message: 'secret body', choices: [{ id: 'allow_once', label: 'Allow once', reply: 'x' }, { id: 'allow_session', label: 'Allow for session', reply: 'y' }, { id: 'deny', label: 'Deny', reply: 'z' }] }, { id: 'perm-2', agent_id: 'b', kind: 'permission', title: 'Approve Claude Code Read?' }];
  const result = await run('status', { kernel: 'b', taskId: 'lane', sessionId: 's', agentId: 'a' }, { execute: async (_, body) => 'RelayStatus' in JSON.parse(body) ? { RelayStatus: { status: { capabilities: ['agent_deadline_v1'] } } } : JSON.parse(body).GetSessionState ? { SessionState: { agents: [{ id: 'a', is_processing: true }], session: { active_interactions: active } } } : { AgentDeadline: { deadline: { state: 'armed' } } } });
  assert.deepEqual(result.interactions, [{ id: 'perm-1', kind: 'permission', level: 'warning', ownerOnly: false, title: 'Approve Claude Code Write?', choices: ['allow_once', 'allow_session', 'deny'] }]);
});

test('status waits on events for the expected turn and ignores earlier idle', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-watch-')); const previous = process.env.CHARIOX_BIN;
  try {
    const executable = join(dir, 'chariox');
    const frames = [
      { event_id: 1, event: { agent_activity: { a: { status: 'idle', active_prompt_count: 0 } } } },
      { event_id: 2, event: { agent_activity: { a: { status: 'working', active_prompt_count: 1, active_turn: { prompt_id: 'p' } } } } },
      { event_id: 3, event: { agent_activity: { a: { status: 'idle', active_prompt_count: 0 } } } },
    ];
    await writeFile(executable, `#!/usr/bin/env node
for(const frame of ${JSON.stringify(frames)}) process.stdout.write(JSON.stringify(frame)+'\\n');
`);
    assert.equal((await readFile(executable, 'utf8')).split('\n')[0], '#!/usr/bin/env node');
    await chmod(executable, 0o700); process.env.CHARIOX_BIN = executable;
    assert.deepEqual(execFileSync(executable, { encoding: 'utf8' }).trim().split('\n').map(line => JSON.parse(line)), frames);
    assert.deepEqual(await waitForIdle({ kernel: 'b', sessionId: 's', attachmentId: 't', agentId: 'a', expectedPromptId: 'p', timeout: 3 }), { eventId: 3, promptId: 'p', state: 'idle' });
  } finally { if (previous === undefined) delete process.env.CHARIOX_BIN; else process.env.CHARIOX_BIN = previous; await rm(dir, { recursive: true, force: true }); }
});

test('bundle hash mismatch refuses local fetch', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-bundle-'));
  try {
    const execute = async (argv, body) => {
      if (argv[0] === 'request') return { KernelFileStat: { stat: { sha256: '0'.repeat(64) } } };
      if (argv[0] === 'files') { await writeFile(argv.at(-1), 'changed bytes'); return {}; }
      if (argv[1] === 'open') return { terminal: { terminal_id: 't' } };
      return { fired: { kind: 'finished', exit_code: 0 } };
    };
    await assert.rejects(run('bundle', { kernel: 'b', taskId: 'lane', sessionId: 's', worktree: '/repo', base: 'a'.repeat(40), head: 'b'.repeat(40), remoteBundle: '/repo/lane.bundle', localBundle: join(dir, 'lane.bundle'), repository: dir }, { execute, git: async () => { throw Error('must not fetch'); } }), { code: 'bundle_hash_mismatch' });
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('bundle verifies real git history and imports into an empty local repository', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-git-'));
  try {
    const source = join(dir, "source 'quoted'"), local = join(dir, 'local');
    await mkdir(source); await mkdir(local);
    const git = (cwd, ...argv) => execFileSync('git', ['-C', cwd, ...argv], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
    git(source, 'init', '-b', 'main'); git(local, 'init', '-b', 'main');
    await writeFile(join(source, 'sum.cjs'), 'module.exports=(a,b)=>a+b;\n'); git(source, 'add', 'sum.cjs');
    git(source, '-c', 'user.name=Lane Test', '-c', 'user.email=lane@example.invalid', 'commit', '-m', 'Add sum [skip ci]'); const base = git(source, 'rev-parse', 'HEAD');
    await writeFile(join(source, 'sum.test.cjs'), 'require("node:assert/strict").equal(require("./sum.cjs")(2,3),5);\n'); git(source, 'add', 'sum.test.cjs');
    git(source, '-c', 'user.name=Lane Test', '-c', 'user.email=lane@example.invalid', 'commit', '-m', 'Test sum [skip ci]'); const head = git(source, 'rev-parse', 'HEAD');
    const remote = join(source, 'lane.bundle'), downloaded = join(dir, 'download.bundle');
    const remoteCli = async (argv, body) => {
      if (argv[0] === 'request') return { KernelFileStat: { stat: { sha256: createHash('sha256').update(await readFile(remote)).digest('hex') } } };
      if (argv[0] === 'files') { await copyFile(remote, downloaded); return {}; }
      if (argv[1] === 'open') { const command = argv.slice(argv.indexOf('--') + 1); execFileSync(command[0], command.slice(1), { stdio: ['ignore', 'pipe', 'pipe'] }); return { terminal: { terminal_id: 't' } }; }
      return { fired: { kind: 'finished', exit_code: 0 } };
    };
    const result = await run('bundle', { kernel: 'b', taskId: 'lane', sessionId: 's', worktree: source, base, head, remoteBundle: remote, localBundle: downloaded, repository: local }, { execute: remoteCli });
    assert.equal(result.head, head); assert.equal(git(local, 'rev-parse', 'refs/remotes/lanes/lane'), head);
    assert.equal(git(local, 'merge-base', base, head), base);
    await assert.rejects(access(join(local, 'sum.cjs')));
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('launch resumes after deadline refusal without creating another agent or upload', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-resume-'));
  try {
    const brief = join(dir, 'brief.md'); await writeFile(brief, 'MP-11: task');
    const input = { kernel: 'b', taskId: 'lane', alias: 'lane', worktree: '/repo', provider: 'codex', profile: 'profile', model: 'gpt-6-luna', effort: 'low', permission: 'required', brief, remoteBrief: '/repo/BRIEF.md', stateFile: join(dir, 'state.json'), deadlineMs: Date.now() + 60_000 };
    let refused = true; const calls = [];
    const execute = async (argv, body) => {
      if (argv[0] === 'files') { calls.push('upload'); return {}; }
      const op = Object.keys(JSON.parse(body))[0]; calls.push(op);
      if (op === 'SetAgentDeadline' && refused) throw new LaneError('kernel_incompatible');
      return { RelayStatus: { RelayStatus: { status: { capabilities: ['agent_deadline_v1'] } } }, GetAgentDeadline: { AgentDeadline: { deadline: { state: 'armed' } } }, CreateSession: { SessionCreated: { session: { id: 's' } } }, AttachToSession: { SessionAttached: { attachment: { id: 't' } } }, RefreshProviderAccountProfile: { ProviderAccountProfile: { profile: { auth_state: 'authenticated' } } }, SpawnAgent: { AgentSpawned: { agent: { id: 'a' } } }, HeartbeatAttachments: { AttachmentsHeartbeated: { lapsed: [] } }, SetAgentDeadline: {}, SubmitPrompt: { PromptSubmitted: { outcome: { Started: { prompt: { id: 'p' } } } } } }[op];
    };
    await assert.rejects(run('launch', input, { execute }), { code: 'kernel_incompatible' });
    assert.equal(JSON.parse(await readFile(input.stateFile)).agentId, 'a');
    refused = false; const state = await run('launch', input, { execute });
    assert.equal(state.agentId, 'a'); assert.equal(calls.filter(op => op === 'SpawnAgent').length, 1); assert.equal(calls.filter(op => op === 'upload').length, 1);
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('stop cleans a real owned child using its boot and start identity', { skip: process.platform !== 'linux' && 'Linux /proc process identity is unavailable on this host' }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-pid-'));
  const child = spawn(process.execPath, ['-e', 'setInterval(()=>{},1000)'], { stdio: 'ignore' });
  const ended = once(child, 'exit');
  try {
    if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw Error('unsafe owned child PID');
    const stat = await readFile(`/proc/${child.pid}/stat`, 'utf8');
    const entry = { owner: 'lane', pid: child.pid, startTime: stat.slice(stat.lastIndexOf(')') + 2).trim().split(/\s+/)[19], bootId: (await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim(), executable: await readlink(`/proc/${child.pid}/exe`) };
    assert.equal(await verifyOwnedPid(entry, 'lane'), true);
    const pidLedger = join(dir, 'ledger.json'); await writeFile(pidLedger, JSON.stringify({ owner: 'lane', processes: [entry] }));
    const execute = async () => ({ AgentsListed: { agents: [] } });
    const result = await run('stop', { kernel: 'b', taskId: 'lane', sessionId: 's', agentId: 'a', attachmentId: 't', pidLedger }, { execute });
    await ended;
    assert.deepEqual(result.signalled, [entry.pid]); assert.equal(await verifyOwnedPid(entry, 'lane'), false);
    assert.deepEqual(JSON.parse(await readFile(pidLedger, 'utf8')).processes, []);
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw Error('unsafe owned child PID');
      child.kill('SIGTERM'); await ended;
    }
    await rm(dir, { recursive: true, force: true });
  }
});

test('stop refuses an unsafe ledger before sending any product command', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-bad-pid-'));
  try {
    const pidLedger = join(dir, 'ledger.json'); await writeFile(pidLedger, JSON.stringify({ owner: 'lane', processes: [{ owner: 'lane', pid: 1 }] }));
    let calls = 0;
    await assert.rejects(run('stop', { kernel: 'b', taskId: 'lane', sessionId: 's', agentId: 'a', attachmentId: 't', pidLedger }, { execute: async () => { calls++; return {}; } }), { code: 'pid_ownership_refused' });
    assert.equal(calls, 0);
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('stop destroys by identity when a restart removed the attachment', async () => {
  const calls = [];
  const result = await run('stop', { kernel: 'b', taskId: 'lane', sessionId: 's', agentId: 'a', attachmentId: 'old' }, { execute: async (_, body) => {
    const name = Object.keys(JSON.parse(body))[0]; calls.push(name);
    if (name === 'CancelActivePrompt') throw new LaneError('attachment_not_found');
    return name === 'ListAgents' ? { AgentsListed: { agents: [] } } : { AgentDestroyed: {} };
  } });
  assert.deepEqual(calls, ['CancelActivePrompt', 'DestroyAgent', 'ListAgents']);
  assert.equal(result.stopped, true);
  assert.equal(result.cancelRefusal, 'attachment_not_found');
});

test('stop preserves an expected-prompt guard if its attachment vanished', async () => {
  const calls = [];
  await assert.rejects(run('stop', { kernel: 'b', taskId: 'lane', sessionId: 's', agentId: 'a', attachmentId: 'old', expectedPromptId: 'expected' }, { execute: async (_, body) => {
    calls.push(Object.keys(JSON.parse(body))[0]); throw new LaneError('attachment_not_found');
  } }), { code: 'attachment_not_found' });
  assert.deepEqual(calls, ['CancelActivePrompt']);
});

test('stop remains idempotent after the agent was already removed', async () => {
  const calls = [];
  const result = await run('stop', { kernel: 'b', taskId: 'lane', sessionId: 's', agentId: 'gone', attachmentId: 'old' }, { execute: async (_, body) => {
    const name = Object.keys(JSON.parse(body))[0]; calls.push(name);
    if (name !== 'ListAgents') throw new LaneError('agent_not_found');
    return { AgentsListed: { agents: [] } };
  } });
  assert.equal(result.stopped, true);
  assert.deepEqual(calls, ['CancelActivePrompt', 'DestroyAgent', 'ListAgents']);
});

test('guarded stop never destroys or cleans processes after a newer prompt starts', async () => {
  for (const refusal of [null, 'no_active_prompt']) {
    const calls = [];
    let current = 'expected';
    const result = await run('stop', {kernel:'b',taskId:'lane',sessionId:'s',agentId:'a',attachmentId:'t',expectedPromptId:'expected'}, {execute: async (_, body) => {
      const request = JSON.parse(body); const name = Object.keys(request)[0]; calls.push(name);
      if (name === 'CancelActivePrompt') {
        assert.equal(request[name].expected_prompt_id, current);
        current = 'newer';
        if (refusal) throw new LaneError(refusal);
        return {PromptCancelled:{}};
      }
      assert.fail(`must preserve ${current}: ${name} is not prompt-fenced`);
    }});
    assert.equal(current, 'newer');
    assert.equal(result.stopped, false);
    assert.equal(result.agentRetained, true);
    assert.deepEqual(result.signalled, []);
    assert.deepEqual(calls, ['CancelActivePrompt']);
  }
});


test('resumed launch replaces a lapsed attachment without duplicating Room, agent or prompt', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-reattach-'));
  try {
    const brief = join(dir, 'brief.md'); await writeFile(brief, 'MP-11: task');
    const input = {kernel:'b',taskId:'lane',alias:'lane',worktree:'/repo',provider:'codex',profile:'profile',model:'gpt-6-luna',effort:'low',permission:'required',brief,remoteBrief:'/repo/BRIEF.md',stateFile:join(dir,'launch.json'),deadlineMs:Date.now()+60_000};
    const calls = []; let resumed = false;
    const execute = async (argv, body) => {
      if (argv[0] === 'files') return {};
      const req = JSON.parse(body); const op = Object.keys(req)[0]; calls.push({op, argv, req});
      if (op === 'SetAgentDeadline' && !resumed) throw new LaneError('kernel_incompatible');
      if (op === 'SubmitPrompt') {
        assert.equal(req[op].attachment_id, 'new-attachment');
        return {PromptSubmitted:{outcome:{Started:{prompt:{id:'p'}}}}};
      }
      return {HeartbeatAttachments:{AttachmentsHeartbeated:{lapsed:[]}},GetAgentDeadline:{AgentDeadline:{deadline:{state:'armed'}}},CreateSession:{SessionCreated:{session:{id:'s'}}},AttachToSession:{SessionAttached:{attachment:{id:resumed?'new-attachment':'old-attachment'}}},HeartbeatAttachments:{AttachmentsHeartbeated:{lapsed:['old-attachment']}},RefreshProviderAccountProfile:{ProviderAccountProfile:{profile:{auth_state:'authenticated'}}},SpawnAgent:{AgentSpawned:{agent:{id:'a'}}},SetAgentDeadline:{},RelayStatus:{RelayStatus:{status:{capabilities:['agent_deadline_v1']}}}}[op];
    };
    await assert.rejects(run('launch',input,{execute}), {code:'kernel_incompatible'});
    resumed = true;
    const state = await run('launch',input,{execute});
    assert.equal(state.promptId,'p');
    for (const op of ['CreateSession','SpawnAgent']) assert.equal(calls.filter(c=>c.op===op).length,1);
    const attachments = calls.filter(c=>c.op==='AttachToSession');
    assert.equal(attachments.length,2);
    assert.notDeepEqual(attachments[0].argv,attachments[1].argv);
    const prompts = calls.filter(c=>c.op==='SubmitPrompt');
    assert.equal(prompts.length,1);
  } finally {await rm(dir,{recursive:true,force:true});}
});

test('attachment replacement first replays an uncertain prompt with identical bytes and id', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-prompt-replay-'));
  try {
    const brief = join(dir, 'brief.md'); await writeFile(brief, 'MP-11: task');
    const input = {kernel:'b',taskId:'lane',alias:'lane',worktree:'/repo',provider:'codex',profile:'profile',model:'gpt-6-luna',effort:'low',permission:'required',brief,remoteBrief:'/repo/BRIEF.md',stateFile:join(dir,'launch.json'),deadlineMs:Date.now()+60_000};
    const prompts = []; let resumed = false, attachments = 0;
    const execute = async (argv, body) => {
      if (argv[0] === 'files') return {};
      const req = JSON.parse(body); const op = Object.keys(req)[0];
      if (op === 'AttachToSession') return {SessionAttached:{attachment:{id:`t${++attachments}`}}};
      if (op === 'SubmitPrompt') {
        prompts.push({argv, body});
        if (!resumed) throw new LaneError('kernel_connection_lost');
        if (req[op].attachment_id === 't1') throw new LaneError('attachment_not_found');
        return {PromptSubmitted:{outcome:{Started:{prompt:{id:'p'}}}}};
      }
      return {HeartbeatAttachments:{AttachmentsHeartbeated:{lapsed:[]}},GetAgentDeadline:{AgentDeadline:{deadline:{state:'armed'}}},CreateSession:{SessionCreated:{session:{id:'s'}}},RefreshProviderAccountProfile:{ProviderAccountProfile:{profile:{auth_state:'authenticated'}}},SpawnAgent:{AgentSpawned:{agent:{id:'a'}}},SetAgentDeadline:{},RelayStatus:{RelayStatus:{status:{capabilities:['agent_deadline_v1']}}}}[op];
    };
    await assert.rejects(run('launch',input,{execute}),{code:'kernel_connection_lost'});
    resumed = true;
    assert.equal((await run('launch',input,{execute})).promptId,'p');
    assert.equal(attachments,2);
    assert.deepEqual(prompts[0],prompts[1]);
    assert.deepEqual(prompts[0],prompts[2]);
    assert.notDeepEqual(prompts[2].argv,prompts[3].argv);
    assert.equal(JSON.parse(prompts[3].body).SubmitPrompt.attachment_id,'t2');
  } finally {await rm(dir,{recursive:true,force:true});}
});


test('launch and status negotiate deadline support without sending unknown requests', async () => {
  for (const supported of [false, true]) {
    const dir = await mkdtemp(join(tmpdir(), 'dogfood-deadline-capability-'));
    try {
      const brief = join(dir,'brief.md'); await writeFile(brief,'MP-11: task');
      const input = {kernel:'b',taskId:'lane',alias:'lane',worktree:'/repo',provider:'codex',profile:'profile',model:'gpt-6-luna',effort:'low',permission:'required',brief,remoteBrief:'/repo/BRIEF.md',stateFile:join(dir,'state.json'),deadlineMs:Date.now()+60_000};
      const calls = []; const timers = [];
      const execute = async (argv, body) => {
        if (argv[0] === 'files') return {};
        const op = Object.keys(JSON.parse(body))[0]; calls.push(op);
        if (!supported && ['SetAgentDeadline','GetAgentDeadline'].includes(op)) throw new LaneError('invalid_request');
        return {RelayStatus:{RelayStatus:{status:{capabilities:supported?['agent_deadline_v1']:[]}}},CreateSession:{SessionCreated:{session:{id:'s'}}},AttachToSession:{SessionAttached:{attachment:{id:'t'}}},RefreshProviderAccountProfile:{ProviderAccountProfile:{profile:{auth_state:'authenticated'}}},SpawnAgent:{AgentSpawned:{agent:{id:'a'}}},SubmitPrompt:{PromptSubmitted:{outcome:{Started:{prompt:{id:'p'}}}}},SetAgentDeadline:{AgentDeadline:{deadline:{state:'armed'}}},GetAgentDeadline:{AgentDeadline:{deadline:{state:'armed'}}},GetSessionState:{SessionState:{agents:[{id:'a',is_processing:false}],session:{active_prompt:null}}}}[op];
      };
      const dependencies = {execute, startTimer:async (file)=>{timers.push(file);return {pid:1234,receipt:`${file}.deadline.json`};}};
      const launched = await run('launch',input,dependencies);
      assert.equal(launched.deadline.mode,supported?'kernel':'coordinator');
      assert.equal(timers.length,0);
      if (!supported) assert.equal(launched.coordinatorTimer.state,'required');
      if (!supported) assert.match(launched.deadline.message,/no kernel deadline/i);
      const status = await run('status',{kernel:'b',taskId:'lane',sessionId:'s',agentId:'a',stateFile:input.stateFile},dependencies);
      assert.equal(status.deadline.mode,supported?'kernel':'coordinator');
      assert.equal(calls.includes('SetAgentDeadline'),supported);
      assert.equal(calls.includes('GetAgentDeadline'),supported);
    } finally {await rm(dir,{recursive:true,force:true});}
  }
});


test('coordinator timer stops the exact lane at expiry and retains a safe failure receipt', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-timer-'));
  try {
    const stateFile = join(dir,'state.json');
    await writeFile(stateFile,JSON.stringify({kernel:'b',taskId:'lane',sessionId:'s',agentId:'a',attachmentId:'t',deadlineMs:100,deadline:{mode:'coordinator'}}));
    let now = 40; const calls = [];
    const result = await enforceCoordinatorDeadline(stateFile,{now:()=>now,wait:async ms=>{assert.equal(ms,60);now+=ms;},stop:async input=>{calls.push(input);return {stopped:true};}});
    assert.equal(result.status,'stopped'); assert.equal(calls[0].agentId,'a');
    assert.equal(JSON.parse(await readFile(`${stateFile}.deadline.json`)).status,'stopped');
    const refused = await enforceCoordinatorDeadline(stateFile,{now:()=>100,stop:async()=>{throw new LaneError('kernel_access_ended','PRIVATE-CANARY');}});
    assert.equal(refused.status,'failed'); assert.equal(refused.code,'kernel_access_ended');
    assert.ok(!JSON.stringify(refused).includes('PRIVATE-CANARY'));
  } finally {await rm(dir,{recursive:true,force:true});}
});

test('replayed successful submission renews its expired attachment without a second prompt', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-replayed-attachment-'));
  try {
    const brief = join(dir,'brief.md'); await writeFile(brief,'MP-11: task');
    const input={kernel:'b',taskId:'lane',alias:'lane',worktree:'/repo',provider:'codex',profile:'profile',model:'gpt-6-luna',effort:'low',permission:'required',brief,remoteBrief:'/repo/BRIEF.md',stateFile:join(dir,'state.json'),deadlineMs:Date.now()+60_000};
    const prompts=[];let replay=false,attachments=0;
    const execute=async(argv,body)=>{
      if(argv[0]==='files')return {};
      const req=JSON.parse(body),op=Object.keys(req)[0];
      if(op==='AttachToSession')return {SessionAttached:{attachment:{id:`t${++attachments}`}}};
      if(op==='SubmitPrompt'){
        prompts.push({argv,body});
        if(!replay)throw new LaneError('kernel_connection_lost');
        return {PromptSubmitted:{outcome:{Started:{prompt:{id:'original'}}}}};
      }
      return {RelayStatus:{RelayStatus:{status:{capabilities:['agent_deadline_v1']}}},CreateSession:{SessionCreated:{session:{id:'s'}}},RefreshProviderAccountProfile:{ProviderAccountProfile:{profile:{auth_state:'authenticated'}}},SpawnAgent:{AgentSpawned:{agent:{id:'a'}}},SetAgentDeadline:{},GetAgentDeadline:{AgentDeadline:{deadline:{state:'armed'}}},HeartbeatAttachments:{AttachmentsHeartbeated:{lapsed:['t1']}}}[op];
    };
    await assert.rejects(run('launch',input,{execute}),{code:'kernel_connection_lost'});
    replay=true;
    const state=await run('launch',input,{execute});
    assert.equal(state.promptId,'original');assert.equal(state.attachmentId,'t2');assert.equal(prompts.length,3);
    assert.deepEqual(prompts[0],prompts[2]);
    assert.equal(state.pendingPrompt,undefined);
  }finally{await rm(dir,{recursive:true,force:true});}
});

test('foreground deadline command rejects another launch and propagates cleanup refusal', async () => {
  const dir=await mkdtemp(join(tmpdir(),'dogfood-deadline-command-'));
  try {
    const stateFile=join(dir,'state.json');
    const state={kernel:'b',taskId:'lane',sessionId:'s',agentId:'a',attachmentId:'t',deadlineMs:1,deadline:{mode:'coordinator'}};
    await writeFile(stateFile,JSON.stringify(state));
    let calls=0;
    const dependencies={execute:async()=>{calls++;throw new LaneError('kernel_access_ended','PRIVATE-CANARY');}};
    const input={kernel:'b',taskId:'lane',sessionId:'s',agentId:'a',stateFile};
    for(const change of [{kernel:'other'},{taskId:'other'},{sessionId:'other'},{agentId:'other'}]) {
      await assert.rejects(run('deadline',{...input,...change},dependencies),{code:'launch_conflict'});
      assert.equal(calls,0);
    }
    await assert.rejects(run('deadline',input,dependencies), error=>error.code==='kernel_access_ended'&&!error.message.includes('PRIVATE-CANARY'));
    assert.equal(calls,1);
    assert.equal(JSON.parse(await readFile(`${stateFile}.deadline.json`)).status,'failed');
    await writeFile(stateFile,JSON.stringify({...state,deadline:{mode:'kernel'}}));
    await assert.rejects(run('deadline',input,dependencies),{code:'invalid_operation'});
    assert.equal(calls,1);
  }finally{await rm(dir,{recursive:true,force:true});}
});

test('stop rejects an empty or malformed requested prompt fence before any mutation', async () => {
  for (const expectedPromptId of ['', null, undefined, false, 0]) {
    let calls=0;
    await assert.rejects(run('stop',{kernel:'b',taskId:'lane',sessionId:'s',agentId:'a',attachmentId:'t',expectedPromptId},{execute:async()=>{calls++;return {AgentsListed:{agents:[]}};}}),{code:'invalid_input'});
    assert.equal(calls,0);
  }
});

test('resumed fallback launch refuses new prompt admission after its coordinator deadline', async () => {
  const dir=await mkdtemp(join(tmpdir(),'dogfood-expired-launch-'));const originalNow=Date.now;
  try {
    const brief=join(dir,'brief.md');await writeFile(brief,'MP-11: task');
    const input={kernel:'b',taskId:'lane',alias:'lane',worktree:'/repo',provider:'codex',profile:'profile',model:'gpt-6-luna',effort:'low',permission:'required',brief,remoteBrief:'/repo/BRIEF.md',stateFile:join(dir,'state.json'),deadlineMs:Date.now()+60_000};
    let submissions=0;
    const execute=async(argv,body)=>{
      if(argv[0]==='files')return {};
      const op=Object.keys(JSON.parse(body))[0];
      if(op==='SubmitPrompt'){submissions++;throw new LaneError('kernel_connection_lost');}
      return {RelayStatus:{RelayStatus:{status:{capabilities:[]}}},CreateSession:{SessionCreated:{session:{id:'s'}}},AttachToSession:{SessionAttached:{attachment:{id:'t'}}},RefreshProviderAccountProfile:{ProviderAccountProfile:{profile:{auth_state:'authenticated'}}},SpawnAgent:{AgentSpawned:{agent:{id:'a'}}}}[op];
    };
    await assert.rejects(run('launch',input,{execute}),{code:'kernel_connection_lost'});
    const before=submissions;Date.now=()=>input.deadlineMs+1;
    await assert.rejects(run('launch',input,{execute}),{code:'agent_deadline_reached'});
    assert.equal(submissions,before);
    assert.equal(JSON.parse(await readFile(input.stateFile)).agentId,'a');
  }finally{Date.now=originalNow;await rm(dir,{recursive:true,force:true});}
});

test('advertised kernel support must yield an armed deadline before prompt admission', async () => {
  const dir=await mkdtemp(join(tmpdir(),'dogfood-missing-deadline-'));
  try {
    const brief=join(dir,'brief.md');await writeFile(brief,'MP-11: task');
    const input={kernel:'b',taskId:'lane',alias:'lane',worktree:'/repo',provider:'codex',profile:'profile',model:'gpt-6-luna',effort:'low',permission:'required',brief,remoteBrief:'/repo/BRIEF.md',stateFile:join(dir,'state.json'),deadlineMs:Date.now()+60_000};
    let submitted=false;
    const execute=async(argv,body)=>{
      if(argv[0]==='files')return {};
      const op=Object.keys(JSON.parse(body))[0];if(op==='SubmitPrompt')submitted=true;
      return {RelayStatus:{RelayStatus:{status:{capabilities:['agent_deadline_v1']}}},CreateSession:{SessionCreated:{session:{id:'s'}}},AttachToSession:{SessionAttached:{attachment:{id:'t'}}},RefreshProviderAccountProfile:{ProviderAccountProfile:{profile:{auth_state:'authenticated'}}},SpawnAgent:{AgentSpawned:{agent:{id:'a'}}},SetAgentDeadline:{AgentDeadline:{deadline:null}},GetAgentDeadline:{AgentDeadline:{deadline:null}},SubmitPrompt:{PromptSubmitted:{outcome:{Started:{prompt:{id:'p'}}}}}}[op];
    };
    await assert.rejects(run('launch',input,{execute}),{code:'agent_deadline_missing'});
    assert.equal(submitted,false);
  }finally{await rm(dir,{recursive:true,force:true});}
});

test('guarded cancellation and unconditional cleanup use separate command receipts', async () => {
  const receipts=new Map(),calls=[];
  const execute=async(argv,body)=>{
    const id=argv[argv.indexOf('--command-id')+1],op=Object.keys(JSON.parse(body))[0];
    if(argv.includes('--command-id')) {
      if(receipts.has(id)&&receipts.get(id)!==body)throw new LaneError('duplicate_command_conflict');
      receipts.set(id,body);
    }
    calls.push({id,op});return op==='ListAgents'?{AgentsListed:{agents:[]}}:{};
  };
  const input={kernel:'b',taskId:'lane',sessionId:'s',agentId:'a',attachmentId:'t'};
  assert.equal((await run('stop',{...input,expectedPromptId:'p'},{execute})).agentRetained,true);
  assert.equal((await run('stop',input,{execute})).stopped,true);
  assert.equal((await run('stop',input,{execute})).stopped,true);
  const cancellations=calls.filter(x=>x.op==='CancelActivePrompt');
  assert.notEqual(cancellations[0].id,cancellations[1].id);
  assert.equal(cancellations[1].id,cancellations[2].id);
});

test('respond answers one interaction by id with a stable command identity', async () => {
  const calls = [];
  await run('respond', { kernel: 'b', taskId: 'lane', sessionId: 's', interactionId: 'claude-permission-run-1-tool', choiceId: 'allow_once' }, { execute: async (argv, body) => { calls.push([argv, JSON.parse(body)]); return {}; } });
  assert.deepEqual(calls, [[['request', '--kernel', 'b', '--command-id', 'lane:respond-claude-permission-run-1-tool'], { RespondToInteraction: { session_id: 's', interaction_id: 'claude-permission-run-1-tool', choice_id: 'allow_once' } }]]);
  await assert.rejects(run('respond', { kernel: 'b', taskId: 'lane', sessionId: 's', interactionId: 'i' }, { execute: async () => ({}) }));
});

test('message returns the summary of the latest finished turn, else its last inline provider output', async () => {
  const page = (text, extra = {}) => ({ entry: { kind: 'provider_output', text }, fragment_start: 0, fragment_end: text.length, total_chars: text.length, ...extra });
  const outline = turns => ({ SessionHistoryOutline: { agents: [{ agent_id: 'a', turns }] } });
  const ask = turns => run('message', { kernel: 'b', taskId: 'lane', sessionId: 's', agentId: 'a', attachmentId: 't' }, { execute: async () => outline(turns) });
  assert.deepEqual(await ask([
    { turn_id: 't1', lifecycle: 'completed', completed_at_ms: 5, summary: page('first') },
    { turn_id: 't2', lifecycle: 'completed', completed_at_ms: 9, summary: page('Round done.'), entries: [page('earlier')] },
    { turn_id: 't3', lifecycle: 'open', summary: page('still working') }]), { text: 'Round done.', truncated: false, turnId: 't2', completedAtMs: 9 });
  assert.equal((await ask([{ turn_id: 't', lifecycle: 'completed', entries: [{ entry: { kind: 'notice', text: 'x' } }, page('inline answer'), { entry: { kind: 'notice', text: 'y' } }] }])).text, 'inline answer');
  assert.equal((await ask([{ turn_id: 't', lifecycle: 'completed', summary: page('cut', { fragment_end: 3, total_chars: 90 }) }])).truncated, true);
  assert.deepEqual(await ask([{ turn_id: 't', lifecycle: 'open', summary: page('not yet') }]), { text: null, reason: 'no_final_message' });
});
