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
        CreateSession: { SessionCreated: { session: { id: 's' } } },
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
    assert.deepEqual(calls, ['CreateSession', 'AttachToSession', 'RefreshProviderAccountProfile', 'SpawnAgent', 'files put', 'SetAgentDeadline', 'SubmitPrompt']);
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
  const result = await run('status', { kernel: 'b', taskId: 'lane', sessionId: 's', agentId: 'a' }, { execute: async (_, body) => JSON.parse(body).GetSessionState ? { SessionState: { agents: [{ id: 'a', is_processing: false, last_activity_at_ms: 123 }], session: { active_prompt: { id: 'other', target_agent_id: 'b' } } } } : { AgentDeadline: { deadline: { state: 'reached' } } } });
  assert.equal(result.currentPrompt, null);
  assert.equal(result.state, 'idle');
  assert.equal(result.lastActivityMs, 123);
});

test('stop destroys an idle agent after typed no-active-prompt refusal', async () => {
  const calls = [];
  const result = await run('stop', { kernel: 'b', taskId: 'lane', sessionId: 's', agentId: 'a', attachmentId: 't' }, { execute: async (_, body) => { const name = Object.keys(JSON.parse(body))[0]; calls.push(name); if (name === 'CancelActivePrompt') throw new LaneError('no_active_prompt'); return { AgentsListed: { agents: [] } }; } });
  assert.deepEqual(calls, ['CancelActivePrompt', 'DestroyAgent', 'ListAgents']);
  assert.equal(result.stopped, true);
});

test('fetch downloads all three artifacts and rejects malformed acceptance', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'dogfood-fetch-'));
  try {
    const execute = async argv => { await writeFile(argv.at(-1), argv.at(-2).endsWith('.json') ? '{"rows":[{"id":"SHORT","status":"PASS"}]}' : 'MP-11: notes'); return {}; };
    await assert.rejects(run('fetch', { kernel: 'b', taskId: 'lane', worktree: '/repo', directory: dir }, { execute }), { code: 'invalid_acceptance' });
    assert.match(await readFile(join(dir, 'PUSH_READY.md'), 'utf8'), /MP-11/);
  } finally { await rm(dir, { recursive: true, force: true }); }
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
    await writeFile(executable, `#!/usr/bin/env node\nfor(const frame of ${JSON.stringify(frames)}) process.stdout.write(JSON.stringify(frame)+'\\n');\n`);
    await chmod(executable, 0o700); process.env.CHARIOX_BIN = executable;
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
      return { CreateSession: { SessionCreated: { session: { id: 's' } } }, AttachToSession: { SessionAttached: { attachment: { id: 't' } } }, RefreshProviderAccountProfile: { ProviderAccountProfile: { profile: { auth_state: 'authenticated' } } }, SpawnAgent: { AgentSpawned: { agent: { id: 'a' } } }, SetAgentDeadline: {}, SubmitPrompt: { PromptSubmitted: { outcome: { Started: { prompt: { id: 'p' } } } } } }[op];
    };
    await assert.rejects(run('launch', input, { execute }), { code: 'kernel_incompatible' });
    assert.equal(JSON.parse(await readFile(input.stateFile)).agentId, 'a');
    refused = false; const state = await run('launch', input, { execute });
    assert.equal(state.agentId, 'a'); assert.equal(calls.filter(op => op === 'SpawnAgent').length, 1); assert.equal(calls.filter(op => op === 'upload').length, 1);
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('stop cleans a real owned child using its boot and start identity', async () => {
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
