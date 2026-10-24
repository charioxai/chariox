// MP-08 / MP-09 / MP-10 / MP-11: Mac coordinator policy over lane adapters.
import { readFile, writeFile, mkdir, readdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { homedir } from 'node:os';
import { createHash } from 'node:crypto';
import { execute, LaneError, save } from '../lanes.mjs';
import { openInbox, waitBatch, confirmSteer, confirmRemoval } from './events.mjs';
export { save };

const validLane = lane => {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]{0,79}$/.test(lane || '')) throw new LaneError('invalid_lane');
  return lane;
};
const read = async path => JSON.parse(await readFile(path, 'utf8'));
export async function coordinate(operation, args, options = {}) {
  const home = resolve(options.home || process.env.CHARIOX_COORDINATOR_HOME || join(homedir(), '.chariox', 'dev', 'coordinator-lanes'));
  await mkdir(home, { recursive: true });
  const adapter = options.adapter || (async (op, input) => execute([new URL(`../lane-${op}.mjs`, import.meta.url).pathname], JSON.stringify(input), process.execPath));
  const cli = options.cli || (argv => execute(argv, '', process.env.CHARIOX_BIN || 'chariox', false, argv[0] === 'events' && argv[1] === 'wait' ? [0, 3, 5] : [0]));
  const now = options.now || Date.now;
  if (operation === 'lanehealth' && !args.length) {
    const records = await Promise.all((await readdir(home)).filter(f => f.endsWith('.json')).map(f => read(join(home, f))));
    return Promise.all(records.filter(r => r.lane && r.launch).map(r => coordinate(operation, [r.lane], options)));
  }
  const lane = operation === 'watch' ? null : validLane(args[0]);
  const path = lane && join(home, `${lane}.json`);
  const load = async () => {
    const record = await read(path);
    if (record.lane !== lane) throw new LaneError('lane_receipt_mismatch');
    return record;
  };
  const ids = r => ({ kernel: r.launch.kernel, taskId: r.launch.taskId, sessionId: r.launch.sessionId, attachmentId: r.launch.attachmentId, agentId: r.launch.agentId });
  if (operation === 'brief') {
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
  }
  if (operation === 'watch') {
    const timeout = Number(args[0] || 15) * 60;
    if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 14400) throw new LaneError('invalid_watch_timeout', 'watch takes 1/60..240 minutes');
    // Requests and launch automatically add targets to this grant's single inbox.
    await openInbox(home, cli);
    let remaining = timeout;
    while (remaining > 0) {
      const seconds = Math.min(remaining, 900);
      const { batch } = await waitBatch(home, cli, now, seconds);
      if (batch.events.length) await cli(['events', 'ack', String(batch.next_cursor)]);
      remaining -= seconds;
      if (batch.events.length || !batch.timed_out || remaining === 0) return batch;
    }
  }
  const record = await load();
  if (!record.launch) throw new LaneError('launch_incomplete', 'Resume brief with its original inputs first.');
  const common = ids(record);
  if (operation === 'deliver') {
    const reviewFile = args[1];
    if (!reviewFile) throw new LaneError('missing_review_file');
    const prompt = await readFile(reviewFile, 'utf8');
    if (!prompt.trim()) throw new LaneError('empty_review');
    const status = await adapter('status', { ...common, stateFile: record.input.stateFile });
    const deliveryPath = join(home, `${lane}.delivery.json`);
    let receipt; try { receipt = await read(deliveryPath); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    const commandId = expected => `review-${createHash('sha256').update(expected).update(prompt).digest('hex')}`;
    // Resume confirmation of an accepted steer even if its turn has since settled.
    const resuming = receipt && receipt.steerId === commandId(receipt.expectedPromptId) && (!status.currentPrompt || status.currentPrompt === receipt.expectedPromptId || !receipt.result.confirmation);
    const expectedPromptId = resuming ? receipt.expectedPromptId : status.currentPrompt;
    if (!expectedPromptId) throw new LaneError('no_active_prompt', 'The worker settled; start a new round to deliver new work.');
    const steerId = commandId(expectedPromptId);
    if (receipt?.steerId !== steerId) receipt = undefined;
    if (receipt?.result.confirmation) return receipt.result;
    const advertised = Array.isArray(status.capabilities) && status.capabilities.includes('outside_steer_events_v1');
    const eventMode = receipt?.confirmationMode ? receipt.confirmationMode === 'event' : advertised;
    if (eventMode && !advertised) throw new LaneError('steer_events_unavailable', 'Restore the kernel steer-event capability to resume this event confirmation.');
    if (eventMode) await openInbox(home, cli);
    if (!receipt) {
      const result = await adapter('steer', { ...common, expectedPromptId, prompt, steerId });
      receipt = { steerId, expectedPromptId, confirmationMode: eventMode ? 'event' : 'acknowledged-steer-status', result };
      await save(deliveryPath, receipt);
    }
    const returnedSteerId = receipt.result.ActivePromptSteered?.prompt?.id;
    if (typeof returnedSteerId !== 'string' || !returnedSteerId) throw new LaneError('unexpected_steer_response');
    const confirmed = async confirmation => {
      receipt.result = { ...receipt.result, confirmation }; await save(deliveryPath, receipt);
    };
    if (eventMode) await confirmSteer({ home, cli, now, target: common, steerId: returnedSteerId, confirmed });
    else await confirmed({ mode: 'acknowledged-steer-status', status: await adapter('status', { ...common, stateFile: record.input.stateFile }) });
    return receipt.result;
  }
  if (operation === 'lanehealth') {
    const status = await adapter('status', { ...common, stateFile: record.input.stateFile });
    const flags = [];
    if (now() >= record.input.deadlineMs) flags.push('LONG-ROUND');
    if (status.state === 'working' && Number.isFinite(status.lastActivityMs) && now() - status.lastActivityMs > 15 * 60_000) flags.push('SILENT');
    return { lane, ...status, flags, limitation: 'Activity/deadline flags are signals; working state alone does not prove progress.' };
  }
  if (operation === 'lanefin') {
    // A stopped worker is gone from its Room; its worktree and bundle are still collectable.
    const status = record.stopped ? { state: 'stopped', currentPrompt: null } : await adapter('status', { ...common, stateFile: record.input.stateFile });
    if (status.currentPrompt || status.state === 'working') throw new LaneError('lane_still_running');
    const directory = join(home, 'artifacts', lane);
    const files = await adapter('fetch', { ...common, worktree: record.laneDirectory || record.input.worktree, directory });
    let head;
    if (!files['ACCEPTANCE_UPDATE.json']?.missing) {
      const update = await read(join(directory, 'ACCEPTANCE_UPDATE.json'));
      const heads = [...new Set(update.rows.map(r => r.head))];
      if (heads.length !== 1) throw new LaneError('ambiguous_bundle_head');
      head = heads[0];
    }
    const config = options.config || await read(process.env.CHARIOX_COORDINATOR_CONFIG || join(home, 'config.json'));
    const bundle = await adapter('bundle', { ...common, sessionId: record.launch.operationsSessionId, worktree: record.input.worktree, base: config.bases?.[lane], head, remoteBundle: `${record.laneDirectory || record.input.worktree}/round.bundle`, localBundle: join(directory, 'round.bundle'), repository: config.repository, timeout: 300 });
    const result = { lane, status, files, bundle };
    await save(join(home, `${lane}.finish.json`), result); return result;
  }
  if (operation === 'stop') {
    const result = record.stopped ? record.stop : await adapter('stop', common);
    record.stopped = result.stopped === true; record.stop = result; await save(path, record);
    if (!record.stopped || result.removal) return result;
    await openInbox(home, cli);
    await confirmRemoval({ home, cli, now, target: common, confirmed: async removal => {
      record.stop = { ...result, removal }; await save(path, record);
    } });
    return record.stop;
  }
  throw new LaneError('unknown_operation');
}
