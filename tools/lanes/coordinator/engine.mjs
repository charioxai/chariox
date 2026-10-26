// MP-08 / MP-09 / MP-10 / MP-11: the Mac's round loop for several lanes, event-driven (the shared engine).
import { readFile, stat, writeFile, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join, dirname } from 'node:path';
import { coordinate, defaultCli, save } from './loop.mjs';
import { followLaunch } from './events.mjs';
import { classifyDecision } from './decisions.mjs';
import { execute, acceptanceIssues, LaneError } from '../lanes.mjs';
import { newSections } from './sections.mjs';
import { stopRequestPath, matchesStop } from './round-stop.mjs';
import { tryLaneLock, releaseLaneLock } from './lane-lock.mjs';
import { resumeAfterRestart } from './restart-resume.mjs';

const read = async path => JSON.parse(await readFile(path, 'utf8'));
const sha256 = text => createHash('sha256').update(text).digest('hex');
const KEEP_EVENTS = 400;
const NOTE_ATTEMPTS = 10;
const ACCEPTANCE_NUDGES = 3;
const sameCommit = /^[0-9a-f]{40}$/;

const target = event => `${event.kernel}/${event.session_id}/${event.agent_id}`;
const compact = e => ({ cursor: e.cursor, at_ms: e.at_ms, type: e.type, state: e.value?.state, summary: e.summary, agent_id: e.agent_id, ...(e.refs?.task_id && { taskId: e.refs.task_id }) });

// Notes the coordinator appended to REVIEW_INBOX.md after launch, one per `## ` section.
export { newSections };

// One engine, one inbox: lanes can join while the round runs (the standing loopd service) or all
// at once (runRound). Every lane keeps its own receipts, so a restarted driver resumes, never relaunches.
export function createRound(options) {
  const now = options.now || Date.now;
  const respond = options.respond || ((input) => execute([new URL('../lane-respond.mjs', import.meta.url).pathname], JSON.stringify(input), process.execPath));
  const cli = options.cli || defaultCli;
  const coordinator = (operation, args, admissionLockHeld = false) => coordinate(operation, args, admissionLockHeld ? { ...options, admissionLockHeld } : options);
  const adapter = options.adapter || (async (op, input) => execute([new URL(`../lane-${op}.mjs`, import.meta.url).pathname], JSON.stringify(input), process.execPath));
  const home = options.home;
  const reconcileMs = options.reconcileMs ?? 5 * 60_000;
  const roundDirectory = join(home, 'rounds');
  const lanes = [];
  const persist = lane => save(lane.entry.runFile, lane.run);
  const log = (lane, kind, detail) => { lane.run.events.push({ at_ms: now(), kind, ...detail }); lane.run.events = lane.run.events.slice(-KEEP_EVENTS); };
  const byTarget = new Map();

  // Launch is idempotent: it reuses the receipt, the original brief and the original deadline.
  const launch = async lane => {
    const { entry, run } = lane;
    const record = await coordinator('brief', [entry.lane, entry.worktree, entry.provider, entry.brief]);
    run.launch = { kernel: record.launch.kernel, sessionId: record.launch.sessionId, agentId: record.launch.agentId };
    lane.receipt = record.launch;
    byTarget.set(`${record.launch.kernel}/${record.launch.sessionId}/${record.launch.agentId}`, lane);
    log(lane, 'launched', { agentId: record.launch.agentId });
    await persist(lane);
  };

  // Joining is idempotent: an identical resume reuses the original launch and its deadline.
  const add = async entry => {
    await mkdir(roundDirectory, { recursive: true });
    let run; try { run = await read(entry.runFile); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    if (run && run.round !== entry.round) run = undefined; // another round's file: loopd has kept its copy
    const restored = Boolean(run?.launchedAtMs && !run.finishReason);
    run ??= { lane: entry.name ?? entry.lane, key: entry.lane, round: entry.round, effort: entry.effort, model: entry.model, provider: entry.provider, worktree: entry.worktree, brief: entry.brief, events: [], notes: [], permissions: [], unanswered: [], statusReads: 0 };
    if (restored && options.config?.kernelStartedAtMs > (run.lastKernelSeenAtMs ?? run.health?.atMs ?? run.launchedAtMs)) {
      const downMs = options.config.kernelStartedAtMs - (run.lastKernelSeenAtMs ?? run.health?.atMs ?? run.launchedAtMs);
      run.pausedMs = (run.pausedMs ?? 0) + downMs;
      run.lastKernelSeenAtMs = options.config.kernelStartedAtMs;
      log({ run }, 'kernel_downtime', { pausedMs: downMs });
    }
    if (run.driverError) { (run.previousDriverErrors ??= []).push(run.driverError); delete run.driverError; }
    const lane = { entry, run, recovering: restored, recoveryStartedAtMs: now(), finished: Boolean(run.finishReason), needsRead: true };
    if (!lane.finished) {
      if (!run.launchedAtMs) {
        const brief = await readFile(entry.brief, 'utf8');
        const inbox = await stat(entry.inbox).catch(error => { if (error.code === 'ENOENT') return { size: 0 }; throw error; });
        run.briefSha256 = sha256(brief);
        run.inboxOffset = inbox.size;
        run.launchedAtMs = now();
        await persist(lane);
      }
      await launch(lane);
    }
    lanes.push(lane);
    return lane;
  };

  // Routine permissions are answered for the session when the kernel offers that choice, so the worker is not asked again
  // for the same kind of call. Everything else (critical, owner hand-offs, credentials, questions, or a decision whose
  // details cannot be read) is left for the coordinator, whose lanewait lists it with the reason.
  const detailsFor = async (lane, id) => {
    try { return (await coordinator('lanehealth', [lane.entry.lane])).interactions?.find(item => item.id === id); }
    catch { return undefined; }
  };

  // A reply that failed stays unresolved: the next read of the lane's status lists the decision again and it is retried.
  const answer = async (lane, id, summary, known) => {
    if (lane.stopping || !id || lane.run.permissions.some(p => p.interactionId === id && p.choice)) return;
    const interaction = known ?? await detailsFor(lane, id);
    const verdict = classifyDecision(interaction);
    if (!verdict.routine) {
      if (!lane.run.unanswered.some(p => p.interactionId === id)) lane.run.unanswered.push({ interactionId: id, summary: interaction?.title ?? summary, ...(interaction?.kind === 'choice' && { message: interaction.message, choices: interaction.choices, choiceLabels: interaction.choiceLabels }), reason: verdict.reason, at_ms: now() });
      if (verdict.reason === 'unclassified') lane.needsRead = true; // the status could not be read: look again at the next wake
      return;
    }
    lane.run.unanswered = lane.run.unanswered.filter(p => p.interactionId !== id);
    let entry = lane.run.permissions.find(p => p.interactionId === id);
    if (!entry) lane.run.permissions.push(entry = { interactionId: id, summary: interaction.title, at_ms: now() });
    entry.attempts = (entry.attempts ?? 0) + 1;
    entry.offered = interaction.choices; // what the kernel offered, so a run shows whether allow_session was available
    const choiceId = interaction.choices.includes('allow_session') ? 'allow_session' : 'allow_once';
    try {
      await respond({ kernel: lane.run.launch.kernel, taskId: lane.entry.lane, sessionId: lane.run.launch.sessionId, interactionId: id, choiceId });
      entry.choice = choiceId; delete entry.error;
    } catch (error) {
      entry.error = error.code || 'respond_failed';
      lane.needsRead = true;
    }
  };

  const handle = async events => {
    const touched = new Set();
    for (const event of events) {
      const lane = byTarget.get(target(event));
      if (!lane || lane.finished) continue;
      lane.run.events.push({ ...compact(event), kind: 'event' });
      // The agent's own last_activity_at_ms only moves at turn boundaries; event times show a worker is alive.
      lane.run.lastEventAtMs = Math.max(lane.run.lastEventAtMs ?? 0, event.at_ms ?? 0);
      if (event.type === 'interaction' && event.value?.state === 'pending') await answer(lane, event.refs?.interaction_id, event.summary);
      else if (event.type === 'interaction' && event.value?.state === 'resolved') {
        lane.run.unanswered = lane.run.unanswered.filter(item => item.interactionId !== event.refs?.interaction_id);
        touched.add(lane); lane.needsRead = true;
      } else if (event.type !== 'interaction') { touched.add(lane); lane.needsRead = true; }
    }
    for (const lane of lanes) lane.run.events = lane.run.events.slice(-KEEP_EVENTS);
    return touched;
  };
  const archived = async archives => {
    const touched = new Set();
    for (const archive of archives || []) for (const lane of await handle((await read(archive)).events || [])) touched.add(lane);
    return touched;
  };

  // A section that could not be delivered stays pending: the next wake sends the same content, which resumes the
  // same steer receipt. Only a worker that has settled, or ten failures in a row, ends the attempts.
  const deliverNotes = async lane => {
    if (lane.stopping) return new Set();
    const text = await readFile(lane.entry.inbox, 'utf8').catch(error => { if (error.code === 'ENOENT') return ''; throw error; });
    const sections = newSections(text, lane.run.inboxOffset);
    const touched = new Set();
    let offset = lane.run.inboxOffset;
    // The coordinator steers its notes itself and records them; the inbox file is then only the record, and what is
    // in the ledger is never delivered a second time. Anything else in it (an older script, a failed steer) still is.
    const readLedger = () => read(join(home, `${lane.entry.lane}.direct.json`)).catch(error => { if (error.code === 'ENOENT') return { sections: [] }; throw error; });
    let direct = await readLedger();
    for (const section of sections) {
      const digest = sha256(section.trim());
      const findSent = () => direct.sections.find(item => item.sha256 === digest || item.parts?.includes(digest));
      // A direct note whose reply was lost may or may not have reached the worker: the same operation is replayed under its saved
      // identity, never a new steer built from the worker's current turn.
      const unresolved = [...Object.values(direct.pendingDeliveries ?? {}), ...(direct.pending ? [direct.pending] : [])];
      const replay = !findSent() ? unresolved.find(item => item.parts?.includes(digest) || item.sha256 === digest) : undefined;
      const sent = findSent();
      if (sent) {
        const note = { heading: section.split('\n', 1)[0].slice(0, 120), sha256: sha256(section), at_ms: now(), delivered: true, mode: `direct-${sent.mode}` };
        lane.run.notes.push(note);
        offset += Buffer.byteLength(section);
        lane.run.inboxOffset = offset;
        log(lane, 'note', note);
        await persist(lane);
        continue;
      }
      const file = join(roundDirectory, `${lane.entry.lane}.note-${lane.run.notes.length + 1}.md`);
      await writeFile(file, replay ? replay.prompt : section, { mode: 0o600 });
      const note = { heading: section.split('\n', 1)[0].slice(0, 120), sha256: sha256(section), at_ms: now() };
      const failures = lane.run.noteFailures?.sha256 === note.sha256 ? lane.run.noteFailures.count : 0;
      try {
        if (replay) {
          const result = await coordinator('steer', [lane.entry.lane, file]);
          note.mode = `direct-${result.mode}`; note.delivered = true;
          direct = await readLedger();
        } else {
          const result = await coordinator('deliver', [lane.entry.lane, file]);
          note.mode = result.confirmation?.mode; note.delivered = true;
          for (const other of await archived(result.confirmation?.archives)) touched.add(other);
        }
      } catch (error) {
        note.delivered = false; note.reason = error.code || 'deliver_failed';
        if (error.code !== 'no_active_prompt' && failures + 1 < NOTE_ATTEMPTS) {
          lane.run.noteFailures = { sha256: note.sha256, count: failures + 1, reason: note.reason, at_ms: now() };
          await persist(lane);
          break;
        }
        note.attempts = failures + 1;
      }
      delete lane.run.noteFailures;
      lane.run.notes.push(note);
      offset += Buffer.byteLength(section);
      lane.run.inboxOffset = offset;
      log(lane, 'note', note);
      await persist(lane);
    }
    return touched;
  };

  // The coordinator drops acceptance rows whose file does not validate, and only learns that after the round. So the file is
  // checked while the worker runs: a running turn is steered with the exact problems, and a worker that ends its turn with an
  // invalid file gets them as its next prompt instead of being collected. At most ACCEPTANCE_NUDGES per round; after that the
  // problems are reported with the collection as before.
  const nudgeAcceptance = async lane => {
    if (lane.stopping) return false;
    const path = join(dirname(lane.entry.runFile), 'ACCEPTANCE_UPDATE.json');
    let text; try { text = await readFile(path, 'utf8'); } catch (error) { if (error.code === 'ENOENT') return false; throw error; }
    let issues; try { issues = acceptanceIssues(JSON.parse(text)); } catch { issues = [{ field: 'file', problem: 'is not valid JSON' }]; }
    const nudges = lane.run.acceptanceNudges ??= [];
    if (!issues.length || nudges.length >= ACCEPTANCE_NUDGES) return false;
    const digest = sha256(text);
    // Mid-round, one nudge per distinct content: the worker may still be writing the file.
    if (!lane.settled && nudges.some(item => item.sha256 === digest)) return false;
    const file = join(roundDirectory, `${lane.entry.lane}.acceptance-${nudges.length + 1}.md`);
    await writeFile(file, `## ${new Date(now()).toISOString().slice(11, 16)}Z loopd: ACCEPTANCE_UPDATE.json does not validate (check ${nudges.length + 1} of ${ACCEPTANCE_NUDGES})\nThe coordinator drops every row of ${path} until it validates. Fix these and rewrite the file, then end your turn again:\n${issues.map(issue => `- ${issue.field}: ${issue.problem}`).join('\n')}\n`, { mode: 0o600 });
    try {
      const sent = await coordinator('steer', [lane.entry.lane, file]);
      nudges.push({ sha256: digest, at_ms: now(), mode: sent.mode, issues: issues.length });
      log(lane, 'acceptance_nudge', { mode: sent.mode, issues: issues.length });
      lane.needsRead = true;
      return true;
    } catch (error) {
      log(lane, 'acceptance_nudge_failed', { code: error.code || 'steer_failed' });
      return false;
    }
  };

  // A lane can change a second repository (e.g. OSS and Cloud); each tip gets its own verified bundle.
  const bundleExtras = async lane => {
    const results = [];
    const record = await read(join(home, `${lane.entry.lane}.json`));
    for (const extra of lane.entry.extraBundles || []) {
      if (!sameCommit.test(extra.base)) { results.push({ name: extra.name, error: 'invalid_base' }); continue; }
      try {
        const bundle = await adapter('bundle', {
          kernel: record.launch.kernel, taskId: `${lane.entry.lane}-${extra.name}`, sessionId: record.launch.operationsSessionId, attachmentId: record.launch.attachmentId, agentId: record.launch.agentId,
          worktree: extra.worktree, base: extra.base, remoteBundle: `${record.laneDirectory}/${extra.name}.bundle`, localBundle: join(home, 'artifacts', lane.entry.lane, `${extra.name}.bundle`), repository: extra.repository, timeout: 300,
        });
        results.push({ name: extra.name, base: bundle.base, head: bundle.head, sha256: bundle.sha256, path: bundle.path });
      } catch (error) { results.push({ name: extra.name, error: error.code || 'bundle_failed', message: String(error.message).slice(0, 200) }); }
    }
    return results;
  };

  // The agent is gone once stopped, so its final message is read first.
  const saveLastMessage = async lane => {
    if (!lane.entry.lastMessageFile) return;
    try {
      const record = await read(join(home, `${lane.entry.lane}.json`));
      const message = await adapter('message', { kernel: record.launch.kernel, taskId: lane.entry.lane, sessionId: record.launch.sessionId, attachmentId: record.launch.attachmentId, agentId: record.launch.agentId });
      if (!message.text) { lane.run.lastMessage = { missing: message.reason || 'no_final_message' }; return; }
      await writeFile(lane.entry.lastMessageFile, message.text, { mode: 0o600 });
      lane.run.lastMessage = { path: lane.entry.lastMessageFile, chars: message.text.length, truncated: Boolean(message.truncated) };
    } catch (error) { lane.run.lastMessage = { error: error.code || 'message_failed' }; }
  };

  // The worker is gone once its stop succeeds; a removal event that never arrives does not undo that or the collection.
  const stopLane = async lane => {
    try {
      lane.run.stop = await coordinator('stop', [lane.entry.lane], true);
      if (!lane.run.stop?.stopped) throw new LaneError('stop_not_confirmed', 'The kernel did not confirm the worker stopped; retain its lane.');
      // Stopping waits on the shared inbox; other lanes' events in those batches are handled here.
      await archived(lane.run.stop?.removal?.archives);
    } catch (error) {
      // Batches consumed while waiting were acknowledged, so their events are handled even when the removal never came.
      await archived(error.archives);
      if (error.code !== 'stop_confirmation_timeout') throw error;
      lane.run.stopConfirmationError = { code: error.code, message: String(error.message).slice(0, 300) };
    }
    lane.run.stopped = true;
  };

  const collect = async (lane, reason) => {
    await saveLastMessage(lane);
    try {
      if (['hard_stop', 'stopped'].includes(reason)) await stopLane(lane);
      const finished = await coordinator('lanefin', [lane.entry.lane, ...(reason === 'worker_lost' ? ['worker_lost'] : [])]);
      lane.run.head = finished.bundle?.head;
      lane.run.bundle = finished.bundle?.path;
      lane.run.bundleSha256 = finished.bundle?.sha256;
      lane.run.files = Object.keys(finished.files || {});
      if (finished.acceptanceIssues) lane.run.acceptanceIssues = finished.acceptanceIssues;
      lane.run.extraBundles = await bundleExtras(lane);
      if (!['hard_stop', 'stopped'].includes(reason)) await stopLane(lane);
      lane.run.finishReason = reason;
    } catch (error) {
      // New work reached the worker after it was read idle (a direct prompt): it is not a failed collection and must not be stopped.
      if (reason !== 'hard_stop' && error.code === 'lane_still_running') { lane.needsRead = true; log(lane, 'collect_deferred', { reason: error.code }); return; }
      lane.run.finishReason = reason === 'worker_lost' ? reason : `${reason}_collect_failed`;
      lane.run.collectError = { code: error.code || 'collect_failed', message: String(error.message).slice(0, 300) };
      if (!lane.run.stopped) {
        try { await stopLane(lane); } catch (stopError) { lane.run.stopError = stopError.code || 'stop_failed'; }
      }
    }
    if (!lane.run.stopped) {
      delete lane.run.finishReason;
      lane.run.stopPending = true;
      lane.run.driverError = { code: 'stop_unconfirmed', message: 'Worker stop is unconfirmed; lane and receipts retained for retry.', at_ms: now() };
      lane.needsRead = true;
      log(lane, 'stop_pending', { reason: lane.run.stopError || lane.run.collectError?.code });
      await persist(lane);
      return;
    }
    delete lane.run.stopPending;
    if (lane.run.driverError?.code === 'stop_unconfirmed') { (lane.run.previousDriverErrors ??= []).push(lane.run.driverError); delete lane.run.driverError; }
    lane.run.finishedAtMs = now();
    lane.finished = true;
    log(lane, 'finished', { reason: lane.run.finishReason, head: lane.run.head });
    await persist(lane);
  };

  // Collection (bundle, then stop) and the coordinator's direct prompts exclude each other per lane. Under the lock the worker is
  // read again, so a prompt admitted since the idle read keeps it running and the lane is simply read again at the next wake.
  const collectUnderLock = async (lane, reason, stillIdle) => {
    const lock = await tryLaneLock(home, lane.entry.lane, 'loopd', now);
    if (!lock) { lane.needsRead = true; return; }
    try {
      if (!stillIdle || await stillIdle()) await collect(lane, reason);
      else { lane.needsRead = true; log(lane, 'collect_deferred', { reason: 'prompted_since_read' }); }
    } finally { await releaseLaneLock(lock); }
  };
  const readsIdle = lane => async () => { const again = await coordinator('lanehealth', [lane.entry.lane]); return again.state === 'idle' && !again.currentPrompt; };

  const pending = () => lanes.filter(lane => !lane.finished);
  let rereadAll = false;
  // One wake: wait on the shared inbox, answer, deliver notes, detect finishes and hard stops.
  const step = async wakeMinutes => {
    for (const lane of pending()) {
      const request = await read(stopRequestPath(home, lane.entry.lane, lane.run.launchedAtMs)).catch(error => { if (error.code === 'ENOENT') return undefined; throw error; });
      if (!request) continue;
      if (!matchesStop(request, lane.run)) throw new LaneError('round_stop_mismatch', `Stop request does not match ${lane.entry.lane}; inspect its saved round identity.`);
      lane.stopping = true;
      await collectUnderLock(lane, 'stopped');
    }
    if (!pending().length) return;
    let batch;
    try { batch = await coordinator('watch', [String(wakeMinutes)]); }
    catch (error) {
      if (error.code === 'worker_lost') {
        const lost = byTarget.get(`${error.launch.kernel}/${error.launch.sessionId}/${error.launch.agentId}`);
        if (!lost) throw error;
        await collectUnderLock(lost, 'worker_lost');
        return;
      }
      if (error.code !== 'subscription_target_missing') throw error;
      // The inbox of a restarted kernel's new grant is empty: attach the running lanes to it, then read each one.
      for (const lane of pending()) await followLaunch(cli, lane.receipt);
      rereadAll = true;
      batch = await coordinator('watch', [String(wakeMinutes)]);
    }
    const touched = await handle(batch.events || []);
    if (rereadAll) { rereadAll = false; for (const lane of pending()) touched.add(lane); }
    for (const lane of pending()) for (const other of await deliverNotes(lane)) touched.add(other);
    for (const lane of pending()) {
      lane.run.lastKernelSeenAtMs = now();
      if (lane.stopping) continue;
      if (now() >= lane.run.launchedAtMs + lane.entry.hardStopMinutes * 60_000 + (lane.run.pausedMs ?? 0)) { await collectUnderLock(lane, 'hard_stop'); continue; }
      // A change seen in a lane's first minute is read at the next wake, not lost.
      // Events can be missed (a worker that finishes before the inbox sees it), so every lane is also read once in its
      // first minute and then every `reconcileMs`; a change seen earlier than that is read at the next wake.
      if (touched.has(lane) || now() - (lane.readAtMs ?? lane.run.launchedAtMs) >= reconcileMs) lane.needsRead = true;
      if (now() >= lane.run.launchedAtMs + 60_000) await nudgeAcceptance(lane); // a file write raises no event, so it is checked on every wake
      if (!lane.needsRead || now() < lane.run.launchedAtMs + 60_000) continue;
      lane.needsRead = false;
      lane.readAtMs = now();
      lane.run.statusReads++;
      let status;
      try { status = await coordinator('lanehealth', [lane.entry.lane]); } catch (error) {
        if (['agent_not_found', 'agent_not_in_session', 'session_not_found'].includes(error.code)) {
          await collectUnderLock(lane, 'worker_lost'); continue;
        }
        log(lane, 'status_error', { code: error.code || 'status_failed' }); continue;
      }
      // Kept for the coordinator: a restored worker that never resumed still reads as working, and only SILENT shows it.
      lane.run.health = { atMs: now(), state: status.state, flags: status.flags, lastActivityMs: status.lastActivityMs };
      if (Array.isArray(status.interactions)) {
        const pending = new Set(status.interactions.map(interaction => interaction.id));
        lane.run.unanswered = lane.run.unanswered.filter(item => pending.has(item.interactionId));
      }
      // A decision raised while no inbox followed the worker (a restart) has no event; it is read here and answered.
      for (const interaction of status.interactions || []) await answer(lane, interaction.id, interaction.title, interaction);
      const resuming = await resumeAfterRestart({ lane, lanes, status, kernelStartedAtMs: options.config?.kernelStartedAtMs, now, home,
        send: (key, file, promptId) => coordinate('steer', [key, file], { ...options, restartPromptId: promptId }), persist,
        log: (lane, kind, detail) => { log(lane, kind, detail); options.log?.(kind, { key: lane.entry.lane, ...detail }); } });
      if (resuming) { lane.needsRead = true; continue; }
      lane.run.currentPromptId = status.currentPrompt || status.admittedPromptId || lane.run.currentPromptId || lane.receipt.promptId;
      if (status.completedTurn) lane.run.completedTurn = status.completedTurn;
      if (status.state === 'idle' && !status.currentPrompt) {
        if (lane.recovering && !(status.completedTurn?.promptId === lane.run.currentPromptId && status.completedTurn.status === 'completed')) {
          if (now() - lane.recoveryStartedAtMs >= 360_000 || (status.completedTurn?.promptId === lane.run.currentPromptId && ['cancelled', 'failed'].includes(status.completedTurn.status))) {
            await collectUnderLock(lane, 'worker_lost');
          } else { lane.needsRead = true; log(lane, 'recovery_pending', {}); }
          continue;
        }
        lane.recovering = false;
        lane.settled = true;
        if (!await nudgeAcceptance(lane)) await collectUnderLock(lane, 'finished', readsIdle(lane));
        lane.settled = false;
      } else if (status.currentPrompt) await nudgeAcceptance(lane);
    }
    for (const lane of pending()) if (lane.run.driverError && !lane.run.stopPending) { (lane.run.previousDriverErrors ??= []).push(lane.run.driverError); delete lane.run.driverError; }
    for (const lane of lanes) await persist(lane);
  };
  // The round is resumable: a rerun reuses every receipt. Say why this driver stopped.
  const failed = async error => {
    for (const lane of pending()) { lane.run.driverError = { code: error.code || 'driver_failed', message: String(error.message).slice(0, 300), at_ms: now() }; await persist(lane); }
  };
  // A new access grant has a new inbox: attach every running lane to it and re-read each one's state.
  const rebrief = async () => { for (const lane of pending()) await launch(lane); rereadAll = true; };
  return { lanes, add, step, pending, failed, rebrief, touchAll: () => { rereadAll = true; }, cli };
}
