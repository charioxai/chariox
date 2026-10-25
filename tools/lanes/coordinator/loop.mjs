// MP-08 / MP-09 / MP-10 / MP-11: Mac coordinator policy over lane adapters.
import { readFile, writeFile, mkdir, readdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { homedir } from 'node:os';
import { createHash } from 'node:crypto';
import { execute, LaneError, save } from '../lanes.mjs';
import { newSections } from './sections.mjs';
import { withLaneLock } from './lane-lock.mjs';
import { openInbox, waitBatch, confirmSteer, confirmRemoval, dropAgentTarget, dropSessionTarget } from './events.mjs';
export { save };

const validLane = lane => {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]{0,79}$/.test(lane || '')) throw new LaneError('invalid_lane');
  return lane;
};
const read = async path => JSON.parse(await readFile(path, 'utf8'));
export const defaultCli = (argv, stdin = '') => execute(argv, stdin, process.env.CHARIOX_BIN || 'chariox', false, argv[0] === 'events' && argv[1] === 'wait' ? [0, 3, 5] : [0]);
// The coordinator names a lane (`panebuild`), the receipts name its round (`panebuild-r5`): a lane name resolves to the
// round loopd is running for it, and a round key is taken as it is.
async function runningKey(home, name, options) {
  validLane(name);
  const exists = path => readFile(path, 'utf8').then(() => true, () => false);
  // The round loopd is running for this lane name comes first: a retained receipt of an old classic lane of the same name is not its target.
  const agents = options.agents || process.env.LOOPD_AGENTS || '/root/.chariox/dev/browser-resume-20260930/agents';
  const run = await read(join(agents, name, 'LOOP_RUN.json')).catch(() => undefined);
  if (run?.round && !run.finishedAtMs && await exists(join(home, `${name}-${run.round}.json`))) return `${name}-${run.round}`;
  if (await exists(join(home, `${name}.json`))) return name;
  throw new LaneError('lane_not_running', `${name} has no round running under loopd; a classic lane gets its notes through REVIEW_INBOX.md only.`);
}
export async function coordinate(operation, args, options = {}) {
  const home = resolve(options.home || process.env.CHARIOX_COORDINATOR_HOME || join(homedir(), '.chariox', 'dev', 'coordinator-lanes'));
  await mkdir(home, { recursive: true });
  const adapter = options.adapter || (async (op, input) => execute([new URL(`../lane-${op}.mjs`, import.meta.url).pathname], JSON.stringify(input), process.execPath));
  const cli = options.cli || defaultCli;
  const now = options.now || Date.now;
  if (operation === 'lanehealth' && !args.length) {
    const records = await Promise.all((await readdir(home)).filter(f => f.endsWith('.json')).map(f => read(join(home, f))));
    return Promise.all(records.filter(r => r.lane && r.launch).map(r => coordinate(operation, [r.lane], options)));
  }
  const lane = operation === 'watch' ? null : ['steer', 'answer'].includes(operation) ? await runningKey(home, args[0], options) : validLane(args[0]);
  const path = lane && join(home, `${lane}.json`);
  const load = async () => {
    const record = await read(path);
    if (record.lane !== lane) throw new LaneError('lane_receipt_mismatch');
    return record;
  };
  const ids = r => ({ kernel: r.launch.kernel, taskId: r.launch.taskId, sessionId: r.launch.sessionId, attachmentId: r.launch.attachmentId, agentId: r.launch.agentId });
  if (operation === 'brief') return withLaneLock(home, lane, 'brief', async () => {
    const [, worktree, provider = 'codex', briefFile] = args;
    if (!worktree || !worktree.startsWith('/')) throw new LaneError('invalid_worktree');
    let record;
    try { record = await load(); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    const brief = briefFile ? await readFile(briefFile, 'utf8') : await (options.stdin || (() => readFile('/dev/stdin', 'utf8')))();
    if (!record) {
      const config = options.config || await read(process.env.CHARIOX_COORDINATOR_CONFIG || join(home, 'config.json'));
      const profile = config.providers?.[provider];
      if (!profile) throw new LaneError('provider_not_configured');
      const laneDirectory = config.laneDirectories?.[lane] || worktree;
      if (typeof laneDirectory !== 'string' || !laneDirectory.startsWith('/') || laneDirectory.includes('\0')) throw new LaneError('invalid_lane_directory');
      if (!brief.trim()) throw new LaneError('empty_brief');
      const localBrief = join(home, `${lane}.brief.md`);
      await writeFile(localBrief, brief, { mode: 0o600, flag: 'wx' });
      const input = { ...profile, ...config.laneProfiles?.[lane], kernel: config.kernel, taskId: lane, alias: lane, operationsAlias: `${lane}-operations`, worktree, provider, brief: localBrief, remoteBrief: `${laneDirectory}/ROUND_BRIEF.md`, stateFile: join(home, `${lane}.launch.json`), deadlineMs: now() + (config.laneProfiles?.[lane]?.deadlineSeconds || config.deadlineSeconds || 7200) * 1000 };
      record = { lane, laneDirectory, input, createdAtMs: now() }; await save(path, record);
    } else if (record.input.worktree !== worktree || record.input.provider !== provider || brief !== await readFile(record.input.brief, 'utf8')) throw new LaneError('launch_conflict');
    record.launch = await adapter('launch', record.input);
    await save(path, record);
    return record;
  });
  if (operation === 'watch') {
    const timeout = Number(args[0] || 15) * 60;
    if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 14400) throw new LaneError('invalid_watch_timeout', 'watch takes 1/60..240 minutes');
    // Requests and launch automatically add targets to this grant's single inbox.
    await openInbox(home, cli, options.settleMs);
    let remaining = timeout;
    while (remaining > 0) {
      const seconds = Math.min(remaining, 900);
      const { batch } = await waitBatch(home, cli, now, seconds);
      if (batch.events.length) await cli(['events', 'ack', String(batch.next_cursor)]);
      remaining -= seconds;
      if (batch.events.length || !batch.timed_out || remaining === 0) return batch;
    }
  }
  let record = await load();
  if (!record.launch) throw new LaneError('launch_incomplete', 'Resume brief with its original inputs first.');
  const common = ids(record);
  // Delivery and launch share the lane lock. Reload before saving so another process's
  // renewed attachment or latest prompt cannot be replaced with an older receipt.
  const reload = async () => {
    record = await load();
    Object.assign(common, ids(record));
    if (record.stopped) throw new LaneError('lane_not_running', `${lane} has been stopped; its round is over.`);
  };
  let attachmentRecovered = false;
  const renewedCommand = (id, attachment) => `reattach-${createHash('sha256').update(id).update(attachment).digest('hex')}`;
  const recoveryFailed = () => new LaneError('attachment_recovery_failed', 'Delivery was refused after one re-attach; inspect the retained round receipt before retrying.');
  const attached = async (send, renewed) => {
    try { return await send(); } catch (error) {
      if (error.code !== 'attachment_not_found') throw error;
      if (attachmentRecovered) throw recoveryFailed();
    }
    attachmentRecovered = true;
    record.launch = await adapter('launch', record.input);
    await save(path, record);
    Object.assign(common, ids(record));
    await renewed();
    try { return await send(); } catch (error) {
      if (error.code !== 'attachment_not_found') throw error;
      throw recoveryFailed();
    }
  };
  // Both delivery paths replay the same command after renewal, including its exact body.
  const dispatch = async (key, initial, send) => {
    record.deliveryCommands ??= {};
    let command = record.deliveryCommands[key];
    if (!command) {
      command = record.deliveryCommands[key] = initial;
      await save(path, record);
    }
    try { return await attached(() => send(command), async () => {
      command = { id: renewedCommand(key, common.attachmentId), attachmentId: common.attachmentId, prompt: command.prompt };
      record.deliveryCommands[key] = command;
      await save(path, record);
    }); } catch (error) {
      if (error.code !== 'duplicate_command_conflict' || !command.legacyAttachment) throw error;
      throw new LaneError('delivery_replay_attachment_unknown', 'The old pending note did not retain its attachment; recover its original launch receipt before retrying.');
    }
  };
  if (operation === 'deliver') return withLaneLock(home, lane, 'deliver', async () => {
    await reload();
    const reviewFile = args[1];
    if (!reviewFile) throw new LaneError('missing_review_file');
    const prompt = await readFile(reviewFile, 'utf8');
    if (!prompt.trim()) throw new LaneError('empty_review');
    const status = await adapter('status', { ...common, stateFile: record.input.stateFile });
    const deliveryPath = join(home, `${lane}.delivery.json`);
    let receipt; try { receipt = await read(deliveryPath); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    const commandId = expected => `review-${createHash('sha256').update(expected).update(prompt).digest('hex')}`;
    // Resume confirmation of an accepted steer even if its turn has since settled.
    const resuming = receipt && receipt.steerId === commandId(receipt.expectedPromptId) && (!status.currentPrompt || status.currentPrompt === receipt.expectedPromptId || !receipt.result?.confirmation);
    const expectedPromptId = resuming ? receipt.expectedPromptId : status.currentPrompt;
    if (!expectedPromptId) throw new LaneError('no_active_prompt', 'The worker settled; start a new round to deliver new work.');
    const steerId = commandId(expectedPromptId);
    if (receipt?.steerId !== steerId) receipt = undefined;
    if (receipt?.result?.confirmation) return receipt.result;
    const advertised = Array.isArray(status.capabilities) && status.capabilities.includes('outside_steer_events_v1');
    const eventMode = receipt?.confirmationMode ? receipt.confirmationMode === 'event' : advertised;
    if (eventMode && !advertised) throw new LaneError('steer_events_unavailable', 'Restore the kernel steer-event capability to resume this event confirmation.');
    if (eventMode) await openInbox(home, cli, options.settleMs);
    if (!receipt) {
      receipt = { steerId, expectedPromptId, confirmationMode: eventMode ? 'event' : 'acknowledged-steer-status', dispatch: { id: steerId, attachmentId: common.attachmentId } };
      await save(deliveryPath, receipt);
    }
    if (!receipt.result) {
      let result, mode = receipt.confirmationMode;
      try { result = await dispatch(steerId, { ...receipt.dispatch, prompt }, async command => {
        receipt.dispatch = command;
        await save(deliveryPath, receipt);
        return adapter('steer', { ...common, attachmentId: command.attachmentId, expectedPromptId, prompt: command.prompt, steerId: command.id });
      }); }
      catch (error) {
        // The kernel took the steer but could not record its event: it is delivered, and sending again would repeat it.
        if (error.code !== 'prompt_steer_event_unrecorded') throw error;
        mode = 'delivered-event-unrecorded';
        result = { ActivePromptSteered: { prompt: { id: receipt.dispatch.id } }, confirmation: { mode } };
      }
      receipt.result = result; receipt.confirmationMode = mode;
      await save(deliveryPath, receipt);
    }
    if (receipt.result.confirmation) return receipt.result;
    const returnedSteerId = receipt.result.ActivePromptSteered?.prompt?.id;
    if (typeof returnedSteerId !== 'string' || !returnedSteerId) throw new LaneError('unexpected_steer_response');
    const confirmed = async confirmation => {
      receipt.result = { ...receipt.result, confirmation }; await save(deliveryPath, receipt);
    };
    if (eventMode) await confirmSteer({ home, cli, now, target: common, steerId: returnedSteerId, confirmed });
    else await confirmed({ mode: 'acknowledged-steer-status', status: await adapter('status', { ...common, stateFile: record.input.stateFile }) });
    return receipt.result;
  });
  // The coordinator's direct note: steers the running turn, or becomes the next prompt when the worker is between turns.
  // The kernel's own answer is the confirmation, so no inbox is read and loopd's events are left alone. The command id is the
  // one loopd's inbox delivery would use, so a later fallback delivery of the same text is the same mutation to the kernel.
  if (operation === 'steer') {
    const reviewFile = args[1];
    if (!reviewFile) throw new LaneError('missing_review_file');
    const prompt = await readFile(reviewFile, 'utf8');
    if (!prompt.trim()) throw new LaneError('empty_review');
    const section = createHash('sha256').update(prompt.trim()).digest('hex');
    const ledgerPath = join(home, `${lane}.direct.json`);
    // One step at a time per lane: loopd collects an idle worker (bundle, then stop) under the same lock, so a prompt is never
    // admitted into a worker that is about to be stopped.
    return withLaneLock(home, lane, 'steer', async () => {
      await reload();
      let ledger; try { ledger = await read(ledgerPath); } catch (error) { if (error.code !== 'ENOENT') throw error; ledger = { sections: [] }; }
      const sent = ledger.sections.find(item => item.sha256 === section);
      if (sent) return { lane, mode: sent.mode, ...(sent.promptId !== undefined && { promptId: sent.promptId, agentStatus: sent.agentStatus }), repeated: true };
      // The operation and its identity are saved before anything is sent. If the kernel took the note but the reply was lost,
      // this call, a retry and loopd's inbox fallback all replay that same operation instead of choosing a new one.
      ledger.pendingDeliveries ??= {};
      if (ledger.pending) { ledger.pendingDeliveries[ledger.pending.sha256] = ledger.pending; delete ledger.pending; }
      let pending = ledger.pendingDeliveries[section];
      let status, mode, answer;
      for (let attempt = 1; !mode; attempt++) {
        if (!pending) {
          status ??= await adapter('status', { ...common, stateFile: record.input.stateFile });
          const operationFor = status.currentPrompt
            ? { operation: 'steer', id: `review-${createHash('sha256').update(status.currentPrompt).update(prompt).digest('hex')}`, expectedPromptId: status.currentPrompt }
            : { operation: 'prompt', id: `direct-${section}` };
          // parts: the inbox consumer cuts the appended note at its headings, so each of those sections counts as delivered with it.
          pending = { sha256: section, ...operationFor, attachmentId: common.attachmentId, parts: newSections(prompt, 0).map(part => createHash('sha256').update(part.trim()).digest('hex')), prompt };
          status = undefined;
          ledger.pendingDeliveries[section] = pending; await save(ledgerPath, ledger);
        }
        const key = pending.operation === 'steer'
          ? `review-${createHash('sha256').update(pending.expectedPromptId).update(pending.prompt).digest('hex')}`
          : `direct-${pending.sha256}`;
        const initial = { id: pending.id, attachmentId: pending.attachmentId || common.attachmentId, prompt: pending.prompt, ...(!pending.attachmentId && { legacyAttachment: true }) };
        try {
          if (pending.operation === 'prompt') {
            answer = await dispatch(key, initial, command => adapter('prompt', { ...common, attachmentId: command.attachmentId, prompt: command.prompt, commandId: command.id }));
            mode = answer.PromptSubmitted?.outcome?.Started ? 'prompt-started' : 'prompt-queued';
          } else { answer = await dispatch(key, initial, command => adapter('steer', { ...common, attachmentId: command.attachmentId, expectedPromptId: pending.expectedPromptId, prompt: command.prompt, steerId: command.id })); mode = 'steered'; }
        } catch (error) {
          // The kernel took the note but could not record its event: it is delivered, and sending again would repeat it.
          if (pending.operation === 'steer' && error.code === 'prompt_steer_event_unrecorded') { answer = { delivered: true, event: 'unrecorded' }; mode = 'steered'; }
          // The turn ended or changed between the read and the steer: nothing was taken, look once more, then say what is wrong.
          else if (pending.operation === 'steer' && error.code === 'prompt_steer_rejected' && attempt < 2) {
            delete ledger.pendingDeliveries[section]; pending = undefined; await save(ledgerPath, ledger);
            status = await adapter('status', { ...common, stateFile: record.input.stateFile });
          } else throw error; // any other failure may have been accepted: the pending operation stays for the replay
        }
      }
    // The kernel's answer carries the whole session and the note text: the coordinator needs only which prompt took it and its status.
      const taken = answer.ActivePromptSteered?.prompt ?? answer.PromptSubmitted?.outcome?.Started?.prompt ?? answer.PromptSubmitted?.outcome?.Queued?.prompt;
      const summary = { promptId: taken?.id ?? null, agentStatus: taken?.status ?? (answer.delivered ? 'delivered-event-unrecorded' : 'unknown') };
      if (mode === 'prompt-started' || mode === 'prompt-queued') {
        const current = await load();
        current.latestPromptId = summary.promptId;
        await save(path, current);
      }
      ledger.sections.push({ sha256: section, parts: pending.parts, mode, at_ms: now(), ...summary });
      delete ledger.pendingDeliveries[section];
      await save(ledgerPath, ledger);
      return { lane, mode, ...summary };
    });
  }
  // The coordinator's one-command answer to a decision loopd left alone.
  if (operation === 'answer') {
    const [, interactionId, choiceId] = args;
    if (!interactionId || !choiceId) throw new LaneError('missing_decision', 'usage: <lane> <decision id> <choice id>');
    const status = await adapter('status', { ...common, stateFile: record.input.stateFile });
    const interaction = status.interactions?.find(item => item.id === interactionId);
    if (!interaction) throw new LaneError('decision_not_pending', `${lane} is not waiting on ${interactionId}; waiting on: ${(status.interactions ?? []).map(item => item.id).join(', ') || 'nothing'}.`);
    if (!interaction.choices?.includes(choiceId)) throw new LaneError('choice_not_offered', `${interactionId} offers: ${(interaction.choices ?? []).join(', ') || 'no choices'}.`);
    await adapter('respond', { ...common, interactionId, choiceId });
    return { lane, decision: interactionId, choice: choiceId, answered: true };
  }
  if (operation === 'lanehealth') {
    const status = await adapter('status', { ...common, stateFile: record.input.stateFile });
    const flags = [];
    if (now() >= record.input.deadlineMs) flags.push('LONG-ROUND');
    if (status.state === 'working' && Number.isFinite(status.lastActivityMs) && now() - status.lastActivityMs > 15 * 60_000) flags.push('SILENT');
    if (status.state === 'error') flags.push('ERROR');
    return { lane, ...status, admittedPromptId: record.latestPromptId ?? record.launch.promptId, flags, limitation: 'Activity/deadline flags are signals; working state alone does not prove progress.' };
  }
  if (operation === 'lanefin') {
    // A stopped worker is gone from its Room; its worktree and bundle are still collectable.
    let status;
    try { status = record.stopped ? { state: 'stopped', currentPrompt: null } : await adapter('status', { ...common, stateFile: record.input.stateFile }); }
    catch (error) {
      if (args[1] !== 'worker_lost' || !['agent_not_found', 'agent_not_in_session', 'session_not_found'].includes(error.code)) throw error;
      await dropAgentTarget(cli, record.launch);
      if (error.code === 'session_not_found') await dropSessionTarget(cli, record.launch);
      record.stopped = true; record.stop = { stopped: true, workerLost: true };
      await save(path, record);
      status = { state: 'worker_lost', currentPrompt: null };
    }
    if (status.currentPrompt || status.state === 'working') throw new LaneError('lane_still_running');
    const directory = join(home, 'artifacts', lane);
    const files = await adapter('fetch', { ...common, worktree: record.laneDirectory || record.input.worktree, directory });
    // Acceptance rows can describe any worktree (or a previously tested commit).
    // The bundle operation resolves and verifies this worktree's own HEAD.
    const acceptance = files['ACCEPTANCE_UPDATE.json']?.acceptance;
    const config = options.config || await read(process.env.CHARIOX_COORDINATOR_CONFIG || join(home, 'config.json'));
    const bundle = await adapter('bundle', { ...common, sessionId: record.launch.operationsSessionId, worktree: record.input.worktree, base: config.bases?.[lane], remoteBundle: `${record.laneDirectory || record.input.worktree}/round.bundle`, localBundle: join(directory, 'round.bundle'), repository: config.repository, timeout: 300 });
    const result = { lane, status, files, bundle, ...(acceptance && !acceptance.valid && { acceptanceIssues: acceptance.issues }) };
    await save(join(home, `${lane}.finish.json`), result); return result;
  }
  if (operation === 'stop') {
    if (!record.stopped) {
      // The room session target still reports the removal; only the agent-level one dies with the agent.
      try { await dropAgentTarget(cli, record.launch); } catch (error) { record.targetCleanupError = error.code || 'unsubscribe_failed'; }
    }
    const result = record.stopped ? record.stop : await adapter('stop', common);
    record.stopped = result.stopped === true; record.stop = result; await save(path, record);
    if (!record.stopped || result.removal || result.workerLost) return result;
    await openInbox(home, cli, options.settleMs);
    await confirmRemoval({ home, cli, now, target: common, confirmed: async removal => {
      record.stop = { ...result, removal }; await save(path, record);
    } });
    return record.stop;
  }
  throw new LaneError('unknown_operation');
}
