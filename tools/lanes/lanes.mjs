// MP-08 / MP-09 / MP-10 / MP-11: coordinator policy over the public CLI only.
import { spawn } from 'node:child_process';
import { readFile, writeFile, rename, mkdir, lstat, readlink } from 'node:fs/promises';
import { dirname, resolve, join } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';

export class LaneError extends Error {
  constructor(code, message = code) { super(message); this.code = code; }
}
const fail = (code, message) => { throw new LaneError(code, message); };
const text = (value, name) => typeof value === 'string' && value.length && !value.includes('\0') ? value : fail('invalid_input', `${name} must be a nonempty string`);
const hash = (value, name) => /^[a-f0-9]{40}$/.test(value) ? value : fail('invalid_input', `${name} must be a full git commit id`);
const MESSAGE_LIMIT = 64 * 1024;
const quote = value => `'${String(value).replaceAll("'", "'\\''")}'`;
const safeCode = value => typeof value === 'string' && /^[a-z][a-z0-9_]{0,79}$/.test(value) ? value : 'cli_failed';
const remediation = code => ({
  provider_profile_unverified: 'Run RefreshProviderAccountProfile {provider, account_profile} on the executing kernel, then retry.',
  provider_profile_unauthenticated: 'Run RefreshProviderAccountProfile; if it remains unauthenticated, reconnect in Provider Accounts.',
  provider_profile_unavailable: 'Run ListProviderAccountProfiles on the executing kernel and select or link a profile.',
  provider_profile_usage_exhausted: 'Run RefreshProviderAccountProfile to refresh usage, or select another account.',
  agent_not_in_session: 'Run ListAgents in the intended Room and use the Room that owns this agent.',
  agent_not_found: 'Run ListAgents in the intended Room and use its returned agent id.',
  attachment_not_found: 'Run AttachToSession with --command-id and use SessionAttached.attachment.id.',
  attachment_not_in_session: 'Attach to the intended Room with --command-id and use its returned attachment id.',
  command_id_required: 'Supply --command-id ID; reuse it only to retry the identical mutation.',
  no_active_prompt: 'Run lane-status; there is no matching active prompt. Do not submit a replacement automatically.',
  prompt_steer_rejected: 'Run lane-status and use its expected current prompt id; do not submit a replacement automatically.',
  agent_deadline_reached: 'The lane deadline was reached; start a new lane agent for new work.',
  agent_deadline_requires_executor: 'Arm the deadline on the executing kernel; deadline-bound agents cannot move to a leased worker.',
  agent_deadline_extension_refused: 'Deadlines cannot be extended; start a new lane agent.',
  invalid_acceptance: 'Fix ACCEPTANCE_UPDATE.json: full area/ROW ids, a known status, a date, and one full 40-character commit id as every row head; then collect again.',
  agent_deadline_invalid: 'Supply deadline_ms as a positive bounded epoch millisecond.',
}[code] || 'chariox refused the operation; inspect its typed code and retained lane state');

// Never include child diagnostics in errors: provider/terminal text can contain values.
export function execute(argv, stdin = '', executable = process.env.CHARIOX_BIN || 'chariox', raw = false, acceptedExitCodes = [0]) {
  return new Promise((ok, bad) => {
    const child = spawn(executable, argv, { stdio: ['pipe', 'pipe', 'pipe'], env: { ...process.env, NODE_OPTIONS: process.env.NODE_OPTIONS || '--max-old-space-size=4096' } });
    let out = '', err = '', overflow = false;
    const collect = (chunk, stderr) => {
      if (out.length + err.length + chunk.length > 4 * 1024 * 1024) { overflow = true; return; }
      if (stderr) err += chunk; else out += chunk;
    };
    child.stdout.on('data', chunk => collect(chunk, false));
    child.stderr.on('data', chunk => collect(chunk, true));
    child.on('error', () => bad(new LaneError('cli_unavailable', 'start the chariox CLI or set CHARIOX_BIN')));
    child.on('close', code => {
      if (overflow) return bad(new LaneError('cli_output_limit'));
      if (!acceptedExitCodes.includes(code)) {
        let reason; try { reason = JSON.parse(err).error?.code; } catch {}
        return bad(new LaneError(safeCode(reason), remediation(safeCode(reason))));
      }
      if (raw) return ok(out);
      try { ok(JSON.parse(out)); } catch { bad(new LaneError('invalid_cli_json')); }
    });
    child.stdin.on('error', () => {});
    child.stdin.end(stdin);
  });
}

export async function save(path, value) {
  await mkdir(dirname(resolve(path)), { recursive: true });
  const temporary = `${path}.${randomUUID()}.tmp`;
  await writeFile(temporary, JSON.stringify(value, null, 2) + '\n', { mode: 0o600, flag: 'wx' });
  await rename(temporary, path);
}
const variant = (value, name) => value?.[name] || fail('unexpected_response', `expected ${name}`);

// Every problem with its exact field, so a worker (or the coordinator) knows what to correct.
export function acceptanceIssues(value) {
  if (!value || !Array.isArray(value.rows)) return [{ field: 'rows', problem: 'must be an array of rows' }];
  const issues = [], seen = new Set();
  value.rows.forEach((row, index) => {
    const at = name => `rows[${index}].${name}`;
    if (!row || typeof row !== 'object') { issues.push({ field: `rows[${index}]`, problem: 'must be an object' }); return; }
    if (typeof row.id !== 'string' || !/^[a-z][a-z0-9-]*\/[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(row.id)) issues.push({ field: at('id'), problem: 'must be a full area/ROW id' });
    else if (seen.has(row.id)) issues.push({ field: at('id'), problem: 'duplicate id' });
    else seen.add(row.id);
    if (!['PASS', 'FAIL', 'NOT RUN'].includes(row.status) && !/^BLOCKED\([^():]+: [^\n]+\)$/.test(row.status)) issues.push({ field: at('status'), problem: 'must be PASS, FAIL, NOT RUN or BLOCKED(<owner>: <reason>)' });
    if ('new' in row && typeof row.new !== 'boolean') issues.push({ field: at('new'), problem: 'must be boolean' });
    if (!/^[a-f0-9]{40}$/.test(row.head)) issues.push({ field: at('head'), problem: 'must be one full 40-character git commit id' });
    if (typeof row.date !== 'string' || !/^\d{4}-\d{2}-\d{2}(?:T.*)?$/.test(row.date) || !Number.isFinite(Date.parse(row.date))) issues.push({ field: at('date'), problem: 'must be an ISO date' });
    for (const key of ['evidence', 'note']) if (key in row && typeof row[key] !== 'string') issues.push({ field: at(key), problem: 'must be text' });
  });
  return issues;
}

export function validateAcceptance(value) {
  const [first] = acceptanceIssues(value);
  if (first) fail('invalid_acceptance', `${first.field}: ${first.problem}`);
  return value;
}

export async function verifyOwnedPid(entry, owner) {
  if (!Number.isSafeInteger(entry.pid) || entry.pid <= 1 || !owner || entry.owner !== owner || !entry.startTime || !entry.bootId || !entry.executable) fail('pid_ownership_refused');
  let stat, boot, executable;
  try {
    stat = await readFile(`/proc/${entry.pid}/stat`, 'utf8');
    boot = (await readFile('/proc/sys/kernel/random/boot_id', 'utf8')).trim();
    executable = await readlink(`/proc/${entry.pid}/exe`);
  } catch (error) {
    if (error.code === 'ENOENT') return false;
    fail('pid_ownership_refused');
  }
  const fields = stat.slice(stat.lastIndexOf(')') + 2).trim().split(/\s+/);
  if (fields[19] !== String(entry.startTime) || boot !== entry.bootId || executable !== entry.executable) fail('pid_ownership_refused');
  return fields[0] !== 'Z';
}

const coordinatorDeadline = deadlineMs => ({
  mode: 'coordinator', deadline_ms: deadlineMs ?? null,
  state: deadlineMs == null ? 'unknown' : Date.now() >= deadlineMs ? 'overdue' : 'pending',
  message: 'This lane has no kernel deadline. A coordinator-side timer requires its process, network and access grant to remain available; it is not a durable kernel hard stop.',
});

export async function run(operation, input, dependencies = {}) {
  if (!input || typeof input !== 'object' || Array.isArray(input)) fail('invalid_input');
  input = structuredClone(input);
  const call = dependencies.execute || execute;
  const kernel = text(input.kernel, 'kernel');
  const taskId = text(input.taskId, 'taskId');
  if (!/^[A-Za-z0-9_.-]{1,100}$/.test(taskId)) fail('invalid_input', 'taskId must be a stable lane identifier');
  const common = ['--kernel', kernel];
  const request = async (name, body, suffix, mutation = true) => {
    const argv = ['request', ...common, ...(mutation ? ['--command-id', `${taskId}:${suffix}`] : [])];
    const wire = JSON.stringify({ [name]: body });
    try { return await call(argv, wire); } catch (error) {
      if (!['kernel_request_timeout', 'kernel_connection_lost'].includes(error.code)) throw error;
      // Retry once with the identical command identity, never with a new mutation.
      return call(argv, wire);
    }
  };
  const session = () => text(input.sessionId, 'sessionId');
  const agent = () => text(input.agentId, 'agentId');
  const attachment = () => text(input.attachmentId, 'attachmentId');
  const terminal = async argv => call(['terminal', ...argv, ...common, '--session', session()]);
  const terminalCommand = async (argv, timeout = input.timeout || 120) => {
    if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 18_000) fail('invalid_input', 'timeout must be 1..18000 seconds');
    const opened = await call(['terminal', 'open', ...common, '--session', session(), '--', ...argv]);
    const id = text(opened.terminal?.terminal_id, 'terminal_id');
    try {
      const waited = await terminal(['wait', '--terminal', id, '--for', 'finished', '--timeout', String(timeout)]);
      if (!waited.fired) fail('terminal_wait_timeout', 'command has no completion receipt; cleanup stopped its terminal');
      if (waited.fired.kind !== 'finished') fail('unexpected_response', 'terminal wait did not return a finished receipt');
      const code = waited.fired.exit_code;
      if (code !== 0) fail('terminal_command_failed', `Room terminal command exited with code ${Number.isInteger(code) ? code : 'unknown'}; inspect the lane build output.`);
      return { terminalId: id, exitCode: code, screenText: waited.screen_text || '' };
    } finally { await terminal(['close', '--terminal', id]); }
  };

  if (operation === 'launch') {
    for (const key of ['alias', 'worktree', 'provider', 'profile', 'model', 'effort', 'permission', 'brief', 'remoteBrief', 'stateFile']) text(input[key], key);
    if (!Number.isSafeInteger(input.deadlineMs) || input.deadlineMs <= 0) fail('invalid_input', 'deadlineMs must be a positive epoch millisecond');
    if (!(await lstat(input.brief)).isFile()) fail('invalid_input', 'brief must be a regular file');
    const brief = await readFile(input.brief, 'utf8');
    if (!brief.trim() || Buffer.byteLength(brief) > 256 * 1024) fail('invalid_input', 'brief must contain 1..262144 bytes');
    let state;
    try { state = JSON.parse(await readFile(input.stateFile, 'utf8')); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    if (!state && input.deadlineMs <= Date.now()) fail('invalid_input', 'a new lane deadline must be in the future');
    const fingerprint = createHash('sha256').update(JSON.stringify(input)).update(brief).digest('hex');
    if (state && state.fingerprint !== fingerprint) fail('launch_conflict', 'stateFile belongs to another launch; reuse its exact input or choose another taskId and stateFile');
    state ||= { kernel, taskId, fingerprint, worktree: input.worktree, deadlineMs: input.deadlineMs };
    const remember = async (key, work) => { if (!state[key]) { state[key] = await work(); await save(input.stateFile, state); } return state[key]; };
    if (input.operationsAlias) await remember('operationsSessionId', async () => variant(await request('CreateSession', { workspace_id: input.worktree, worktree_id: input.worktree, alias: text(input.operationsAlias, 'operationsAlias') }, 'operations-room'), 'SessionCreated').session.id);
    input.sessionId = await remember('sessionId', async () => variant(await request('CreateSession', { workspace_id: input.worktree, worktree_id: input.worktree, alias: input.alias }, 'room'), 'SessionCreated').session.id);
    const attach = async () => {
      if (!state.attachCommand) {
        state.attachCommand = `attach-${randomUUID()}`;
        await save(input.stateFile, state);
      }
      input.attachmentId = await remember('attachmentId', async () => variant(await request('AttachToSession', { session_id: session(), client_id: taskId, capability_level: 'FullTerminal' }, state.attachCommand), 'SessionAttached').attachment.id);
    };
    const retireAttachment = async () => {
      delete state.attachmentId; delete state.attachCommand;
      await save(input.stateFile, state);
    };
    const replayingPrompt = Boolean(state.pendingPrompt);
    const refreshAttachment = async () => {
      if (!state.attachmentId) return;
      const heartbeat = variant(await request('HeartbeatAttachments', { attachments: [{ session_id: session(), attachment_id: state.attachmentId }] }, 'attachment-check', false), 'AttachmentsHeartbeated');
      if (heartbeat.lapsed.includes(state.attachmentId)) await retireAttachment();
    };
    if (!replayingPrompt) await refreshAttachment();
    await attach();
    await remember('profileRefreshed', async () => {
      state.refreshAttempt = (state.refreshAttempt || 0) + 1; await save(input.stateFile, state);
      const profile = variant(await request('RefreshProviderAccountProfile', { provider: input.provider, account_profile: input.profile }, `refresh-${state.refreshAttempt}`), 'ProviderAccountProfile').profile;
      if (profile.auth_state !== 'authenticated') fail('provider_profile_unauthenticated', 'profile refresh did not authenticate; reconnect through Provider Accounts');
      return true;
    });
    input.agentId = await remember('agentId', async () => {
      const agent = variant(await request('SpawnAgent', { session_id: session(), alias: input.alias, provider: input.provider, account_profile: input.profile, model: input.model, effort: input.effort, permission_level: input.permission, worktree_id: input.worktree }, 'agent'), 'AgentSpawned').agent;
      state.selection = { provider: agent.provider, profile: agent.account_profile, model: agent.model, effort: agent.effort, permission: agent.permission_level_override };
      return text(agent.id, 'agent id');
    });
    await remember('briefUploaded', async () => { await call(['files', 'put', ...common, '--overwrite', input.brief, input.remoteBrief]); return true; });
    const capabilities = variant(await request('RelayStatus', null, 'deadline-capability', false), 'RelayStatus').status.capabilities;
    if (Array.isArray(capabilities) && capabilities.includes('agent_deadline_v1')) {
      await remember('deadlineArmed', async () => { await request('SetAgentDeadline', { session_id: session(), agent_id: agent(), deadline_ms: input.deadlineMs }, 'deadline'); return true; });
      const receipt = variant(await request('GetAgentDeadline', { session_id: session(), agent_id: agent() }, 'deadline-read', false), 'AgentDeadline').deadline;
      if (!receipt) fail('agent_deadline_missing', 'The executing kernel did not retain its lane deadline; inspect GetAgentDeadline before admitting work.');
      state.deadline = { ...receipt, mode: 'kernel' };
    } else {
      state.deadline = coordinatorDeadline(input.deadlineMs);
    }
    await save(input.stateFile, state);
    if (state.deadline.mode === 'coordinator' && !state.promptId && Date.now() >= input.deadlineMs) fail('agent_deadline_reached', 'The coordinator deadline was reached; use the retained Room/agent state to stop the lane.');
    await remember('promptId', async () => {
      const submit = async () => {
        if (!state.pendingPrompt) {
          state.pendingPrompt = { attachmentId: attachment(), command: `prompt-${randomUUID()}` };
          await save(input.stateFile, state);
        }
        // An interrupted request is retried with its original attachment/body/id.
        return variant(await request('SubmitPrompt', { session_id: session(), attachment_id: state.pendingPrompt.attachmentId, target_agent_id: agent(), prompt: `MP-08 / MP-09 / MP-10 / MP-11: Read the lane brief at ${input.remoteBrief} and complete it.`, attachments: [] }, state.pendingPrompt.command), 'PromptSubmitted');
      };
      let submitted;
      try { submitted = await submit(); }
      catch (error) {
        if (error.code !== 'attachment_not_found') throw error;
        // Definitive pre-admission refusal: no prompt started under this identity.
        delete state.pendingPrompt;
        await retireAttachment(); await attach();
        submitted = await submit();
      }
      const prompt = submitted.outcome?.Started?.prompt || submitted.outcome?.Queued?.prompt;
      const id = text(prompt?.id, 'submitted prompt id');
      delete state.pendingPrompt;
      return id;
    });
    if (replayingPrompt) {
      await refreshAttachment(); await attach();
    }
    if (state.deadline.mode === 'coordinator') {
      // Foreground coordinator jobs retain the OS ancestry of its access grant.
      // An orphaned timer would lose that authority when this CLI exits.
      state.coordinatorTimer = {
        state: 'required', receipt: `${resolve(input.stateFile)}.deadline.json`,
        argv: [process.execPath, fileURLToPath(new URL('./lane-deadline.mjs', import.meta.url))],
        input: { kernel, taskId, sessionId: session(), agentId: agent(), stateFile: resolve(input.stateFile) },
      };
      await save(input.stateFile, state);
    }
    return state;
  }
  if (operation === 'deadline') {
    const saved = JSON.parse(await readFile(text(input.stateFile, 'stateFile'), 'utf8'));
    if (saved.kernel !== kernel || saved.taskId !== taskId || saved.sessionId !== session() || saved.agentId !== agent()) fail('launch_conflict');
    if (saved.deadline?.mode !== 'coordinator') fail('invalid_operation', 'the kernel owns this lane deadline');
    const { enforceCoordinatorDeadline } = await import('./coordinator-deadline.mjs');
    const receipt = await enforceCoordinatorDeadline(input.stateFile, { stop: value => run('stop', value, dependencies) });
    if (receipt.status === 'failed') fail(receipt.code, receipt.message);
    return receipt;
  }
  if (operation === 'steer') return request('SteerActivePrompt', { session_id: session(), attachment_id: attachment(), target_agent_id: agent(), expected_active_prompt_id: text(input.expectedPromptId, 'expectedPromptId'), prompt: text(input.prompt, 'prompt'), attachments: [] }, text(input.steerId, 'steerId'));
  if (operation === 'message') {
    // The worker's final message: the last provider output of its latest finished turn.
    const outline = variant(await request('GetSessionHistoryOutline', { session_id: session(), agent_ids: [agent()], latest_prompt_count: 1 }, 'message-outline', false), 'SessionHistoryOutline');
    const turn = outline.agents?.find(value => value.agent_id === agent())?.turns?.filter(value => value.lifecycle === 'completed').at(-1);
    // The kernel keeps a turn's last provider output as its summary (inline entries carry the earlier ones).
    const last = turn?.summary?.entry?.kind === 'provider_output' ? turn.summary : turn?.entries?.filter(item => item.entry?.kind === 'provider_output').at(-1);
    if (!last?.entry?.text) return { text: null, reason: 'no_final_message' };
    return { text: last.entry.text.slice(0, MESSAGE_LIMIT), truncated: last.entry.text.length > MESSAGE_LIMIT || last.fragment_end < last.total_chars, turnId: turn.turn_id, completedAtMs: turn.completed_at_ms ?? null };
  }
  if (operation === 'respond') {
    const interaction = text(input.interactionId, 'interactionId');
    return request('RespondToInteraction', { session_id: session(), interaction_id: interaction, choice_id: text(input.choiceId, 'choiceId') }, `respond-${interaction}`);
  }
  if (operation === 'status') {
    if (input.waitForIdle) {
      attachment(); text(input.expectedPromptId, 'expectedPromptId');
      const observed = await (dependencies.waitForIdle || (await import('./watch.mjs')).waitForIdle)(input);
      const current = await run('status', { ...input, waitForIdle: false }, dependencies);
      if (current.currentPrompt) fail('prompt_changed', 'a prompt is active after the observed idle receipt; inspect lane status');
      return { ...current, observed };
    }
    const state = variant(await request('GetSessionState', { session_id: session() }, 'status', false), 'SessionState');
    const selected = state.agents?.find(value => value.id === agent()) || state.session?.agents?.find(value => value.id === agent());
    if (!selected) fail('agent_not_in_session', 'use ListAgents in the lane Room');
    const activity = state.agent_activity?.[agent()] || state.session?.agent_activity?.[agent()];
    const capabilities = variant(await request('RelayStatus', null, 'deadline-capability', false), 'RelayStatus').status.capabilities;
    let deadline;
    if (Array.isArray(capabilities) && capabilities.includes('agent_deadline_v1')) {
      const receipt = variant(await request('GetAgentDeadline', { session_id: session(), agent_id: agent() }, 'deadline-status', false), 'AgentDeadline').deadline;
      deadline = receipt ? { ...receipt, mode: 'kernel' } : { mode: 'kernel', state: 'unset', deadline_ms: null };
    } else {
      let deadlineMs = input.deadlineMs;
      if (input.stateFile) {
        const saved = JSON.parse(await readFile(input.stateFile, 'utf8'));
        if (saved.kernel !== kernel || saved.taskId !== taskId || saved.sessionId !== session() || saved.agentId !== agent()) fail('launch_conflict');
        deadlineMs = saved.deadlineMs;
      }
      deadline = coordinatorDeadline(deadlineMs);
    }
    const active = state.session.active_prompt;
    return { agentId: agent(), state: activity?.status || (selected.is_processing ? 'working' : 'idle'), currentPrompt: activity?.active_turn?.prompt_id || (active?.target_agent_id === agent() ? active.id : null), lastActivityMs: selected.last_activity_at_ms, interactions: (state.session.active_interactions || []).filter(value => value.agent_id === agent()).map(value => ({ id: value.id, kind: value.kind, level: value.level, ownerOnly: Boolean(value.handoff || value.provider_login || value.sudo_request || value.requester), title: value.title, choices: (value.choices || []).map(choice => choice.id) })), deadline, capabilities: Array.isArray(capabilities) ? capabilities : [] };
  }
  if (operation === 'fetch') {
    const directory = resolve(text(input.directory, 'directory')); await mkdir(directory, { recursive: true });
    const remote = text(input.worktree, 'worktree'); const result = {};
    for (const name of ['LANE_STATUS.md', 'PUSH_READY.md', 'ACCEPTANCE_UPDATE.json']) {
      const local = join(directory, name);
      try { await call(['files', 'get', ...common, '--overwrite', `${remote}/${name}`, local]); }
      catch (error) {
        // A worker stopped early may not have written its handoff; the round's commits are still bundled.
        if (error.code !== 'file_not_found') throw error;
        result[name] = { missing: true, reason: error.code };
        continue;
      }
      const bytes = await readFile(local);
      result[name] = { path: local, sha256: createHash('sha256').update(bytes).digest('hex') };
      // Workers write what they can; collection keeps the round's work and reports what to correct.
      if (name.endsWith('.json')) {
        let issues;
        try { issues = acceptanceIssues(JSON.parse(bytes.toString('utf8'))); } catch { issues = [{ field: '$', problem: 'is not valid JSON' }]; }
        result[name].acceptance = issues.length ? { valid: false, issues: issues.slice(0, 20) } : { valid: true };
      }
    }
    return result;
  }
  if (operation === 'bundle') {
    const base = hash(input.base, 'base'), expectedHead = input.head == null ? undefined : hash(input.head, 'head');
    const worktree = text(input.worktree, 'worktree'), remote = text(input.remoteBundle, 'remoteBundle');
    const local = resolve(text(input.localBundle, 'localBundle')), repo = resolve(text(input.repository, 'repository'));
    const guard = expectedHead ? `test "$(git rev-parse HEAD)" = ${quote(expectedHead)}; ` : '';
    const command = `set -eu; cd ${quote(worktree)}; ${guard}git merge-base --is-ancestor ${quote(base)} HEAD; git bundle create ${quote(remote)} HEAD`;
    await terminalCommand(['/bin/bash', '-lc', command]);
    const before = variant(await request('StatKernelFile', { path: remote, with_sha256: true }, 'bundle-stat', false), 'KernelFileStat').stat;
    await mkdir(dirname(local), { recursive: true }); await call(['files', 'get', ...common, '--overwrite', remote, local]);
    const digest = createHash('sha256').update(await readFile(local)).digest('hex');
    if (digest !== before.sha256) fail('bundle_hash_mismatch');
    const git = dependencies.git || ((argv) => execute(argv, '', 'git', true));
    await git(['-C', repo, 'bundle', 'verify', local]);
    const heads = String(await git(['bundle', 'list-heads', local])).trim().split('\n');
    const tip = heads.length === 1 && /^([a-f0-9]{40}) HEAD$/.exec(heads[0]);
    if (!tip || (expectedHead && tip[1] !== expectedHead)) fail('bundle_head_mismatch');
    const head = tip[1];
    const incoming = `refs/lanes/incoming/${taskId}`;
    await git(['-C', repo, 'fetch', '--no-tags', local, `${head}:${incoming}`]);
    try {
      await git(['-C', repo, 'merge-base', '--is-ancestor', base, head]);
      await git(['-C', repo, 'fetch', '--no-tags', local, `${head}:refs/remotes/lanes/${taskId}`]);
    } finally { await git(['-C', repo, 'update-ref', '-d', incoming, head]); }
    return { base, head, sha256: digest, path: local };
  }
  if (operation === 'slot') {
    const script = text(input.slotScript, 'slotScript');
    if (!Array.isArray(input.argv) || input.argv.length === 0 || input.argv.some(value => typeof value !== 'string')) fail('invalid_input', 'argv must be a nonempty string array');
    const command = `cd ${quote(text(input.worktree, 'worktree'))} && ${[script, ...input.argv].map(quote).join(' ')}`;
    return terminalCommand(['/bin/bash', '-lc', command]);
  }
  if (operation === 'stop') {
    if ('expectedPromptId' in input) text(input.expectedPromptId, 'expectedPromptId');
    // Validate the entire PID ledger before the first cancellation or signal.
    const ledger = input.pidLedger ? JSON.parse(await readFile(input.pidLedger, 'utf8')) : { owner: taskId, processes: [] };
    if (ledger.owner !== taskId || !Array.isArray(ledger.processes)) fail('pid_ownership_refused');
    const owned = [];
    for (const entry of ledger.processes) if (await verifyOwnedPid(entry, taskId)) owned.push(entry);
    const cancelBody = { session_id: session(), attachment_id: attachment(), target_agent_id: agent(), ...(input.expectedPromptId ? { expected_prompt_id: input.expectedPromptId } : {}) };
    const cancelId = `cancel-${createHash('sha256').update(JSON.stringify(cancelBody)).digest('hex')}`;
    let cancelRefusal;
    try {
      await request('CancelActivePrompt', cancelBody, cancelId);
    } catch (error) {
      if (!['no_active_prompt', 'agent_not_found'].includes(error.code) && !(error.code === 'attachment_not_found' && !input.expectedPromptId)) throw error;
      // DestroyAgent owns cancellation too; ephemeral attachments vanish on restart.
      cancelRefusal = error.code;
    }
    // DestroyAgent and PID cleanup cannot compare an expected prompt atomically.
    // Retain both after a guarded cancellation; a new turn may already be active.
    if (input.expectedPromptId) return {
      stopped: false, agentRetained: true, agentId: agent(),
      promptCancelled: !cancelRefusal, cancelRefusal, signalled: [],
      message: 'Guarded cancellation requested. Agent and owned processes retained; unconditional cleanup requires a separate stop without expectedPromptId.',
    };
    try { await request('DestroyAgent', { session_id: session(), agent_id: agent() }, 'destroy'); }
    catch (error) { if (error.code !== 'agent_not_found') throw error; }
    const roster = variant(await request('ListAgents', { session_id: session() }, 'stopped-roster', false), 'AgentsListed').agents;
    if (roster.some(value => value.id === agent())) fail('cleanup_incomplete', 'agent remains in its Room roster');
    for (const entry of owned) {
      if (!Number.isSafeInteger(entry.pid) || entry.pid <= 1) fail('pid_ownership_refused');
      if (await verifyOwnedPid(entry, taskId)) {
        try { process.kill(entry.pid, 'SIGTERM'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
      }
    }
    const until = Date.now() + 10_000;
    for (const entry of owned) {
      while (await verifyOwnedPid(entry, taskId)) {
        if (Date.now() >= until) fail('cleanup_incomplete', 'owned process did not stop; its identity remains in the ledger');
        await new Promise(ok => setTimeout(ok, 100));
      }
    }
    if (input.pidLedger) await save(input.pidLedger, { ...ledger, processes: [], stopped: ledger.processes, stoppedAt: new Date().toISOString() });
    return { stopped: true, agentId: agent(), cancelRefusal, signalled: owned.map(value => value.pid) };
  }
  fail('invalid_operation');
}

export async function main(operation) {
  try {
    let body = ''; for await (const part of process.stdin) { body += part; if (body.length > 1024 * 1024) fail('input_limit'); }
    const result = await run(operation, JSON.parse(body)); process.stdout.write(JSON.stringify(result) + '\n');
  } catch (error) {
    process.stderr.write(JSON.stringify({ error: { code: safeCode(error.code), message: error instanceof LaneError ? error.message : 'lane adapter failed; inspect input and retained lane state' } }) + '\n');
    process.exitCode = 1;
  }
}
