// MP-08 / MP-09 / MP-10 / MP-11: the Mac's round loop for several lanes, event-driven (the shared engine).
import { readFile, stat, writeFile, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { coordinate, defaultCli, save } from './loop.mjs';
import { execute } from '../lanes.mjs';

const read = async path => JSON.parse(await readFile(path, 'utf8'));
const sha256 = text => createHash('sha256').update(text).digest('hex');
const KEEP_EVENTS = 400;
const sameCommit = /^[0-9a-f]{40}$/;

const target = event => `${event.kernel}/${event.session_id}/${event.agent_id}`;
const compact = e => ({ cursor: e.cursor, at_ms: e.at_ms, type: e.type, state: e.value?.state, summary: e.summary, agent_id: e.agent_id });

// Notes the coordinator appended to REVIEW_INBOX.md after launch, one per `## ` section.
export function newSections(text, offset) {
  const tail = Buffer.from(text).subarray(offset).toString('utf8');
  return tail.split(/^(?=## )/m).filter(s => s.trim());
}

// One engine, one inbox: lanes can join while the round runs (the standing loopd service) or all
// at once (runRound). Every lane keeps its own receipts, so a restarted driver resumes, never relaunches.
export function createRound(options) {
  const now = options.now || Date.now;
  const respond = options.respond || ((input) => execute([new URL('../lane-respond.mjs', import.meta.url).pathname], JSON.stringify(input), process.execPath));
  const coordinator = (operation, args) => coordinate(operation, args, options);
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
    byTarget.set(`${record.launch.kernel}/${record.launch.sessionId}/${record.launch.agentId}`, lane);
    log(lane, 'launched', { agentId: record.launch.agentId });
    await persist(lane);
  };

  // Joining is idempotent: an identical resume reuses the original launch and its deadline.
  const add = async entry => {
    await mkdir(roundDirectory, { recursive: true });
    let run; try { run = await read(entry.runFile); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    if (run && run.round !== entry.round) run = undefined; // another round's file: loopd has kept its copy
    run ??= { lane: entry.name ?? entry.lane, key: entry.lane, round: entry.round, effort: entry.effort, model: entry.model, provider: entry.provider, worktree: entry.worktree, brief: entry.brief, events: [], notes: [], permissions: [], unanswered: [], statusReads: 0 };
    if (run.driverError) { (run.previousDriverErrors ??= []).push(run.driverError); delete run.driverError; }
    const lane = { entry, run, finished: Boolean(run.finishReason), needsRead: true };
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

  const answer = async (lane, event) => {
    const id = event.refs?.interaction_id;
    if (!id || lane.run.permissions.some(p => p.interactionId === id)) return;
    if (!/^Approve Claude Code /.test(event.summary || '')) { lane.run.unanswered.push({ interactionId: id, summary: event.summary }); return; }
    const entry = { interactionId: id, summary: event.summary, at_ms: now() };
    try {
      await respond({ kernel: lane.run.launch.kernel, taskId: lane.entry.lane, sessionId: lane.run.launch.sessionId, interactionId: id, choiceId: 'allow_once' });
      entry.choice = 'allow_once';
    } catch (error) { entry.error = error.code || 'respond_failed'; }
    lane.run.permissions.push(entry);
  };

  const handle = async events => {
    const touched = new Set();
    for (const event of events) {
      const lane = byTarget.get(target(event));
      if (!lane || lane.finished) continue;
      lane.run.events.push({ ...compact(event), kind: 'event' });
      // The agent's own last_activity_at_ms only moves at turn boundaries; event times show a worker is alive.
      lane.run.lastEventAtMs = Math.max(lane.run.lastEventAtMs ?? 0, event.at_ms ?? 0);
      if (event.type === 'interaction' && event.value?.state === 'pending') await answer(lane, event);
      else if (event.type !== 'interaction') touched.add(lane);
    }
    for (const lane of lanes) lane.run.events = lane.run.events.slice(-KEEP_EVENTS);
    return touched;
  };
  const archived = async archives => {
    const touched = new Set();
    for (const archive of archives || []) for (const lane of await handle((await read(archive)).events || [])) touched.add(lane);
    return touched;
  };

  const deliverNotes = async lane => {
    const text = await readFile(lane.entry.inbox, 'utf8').catch(error => { if (error.code === 'ENOENT') return ''; throw error; });
    const sections = newSections(text, lane.run.inboxOffset);
    const touched = new Set();
    let offset = lane.run.inboxOffset;
    for (const section of sections) {
      const file = join(roundDirectory, `${lane.entry.lane}.note-${lane.run.notes.length + 1}.md`);
      await writeFile(file, section, { mode: 0o600 });
      const note = { heading: section.split('\n', 1)[0].slice(0, 120), sha256: sha256(section), at_ms: now() };
      try {
        const result = await coordinator('deliver', [lane.entry.lane, file]);
        note.mode = result.confirmation?.mode; note.delivered = true;
        for (const other of await archived(result.confirmation?.archives)) touched.add(other);
      } catch (error) { note.delivered = false; note.reason = error.code || 'deliver_failed'; }
      lane.run.notes.push(note);
      offset += Buffer.byteLength(section);
      lane.run.inboxOffset = offset;
      log(lane, 'note', note);
      await persist(lane);
    }
    return touched;
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

  const collect = async (lane, reason) => {
    await saveLastMessage(lane);
    try {
      if (reason === 'hard_stop') { lane.run.stop = await coordinator('stop', [lane.entry.lane]); lane.run.stopped = true; await archived(lane.run.stop?.removal?.archives); }
      const finished = await coordinator('lanefin', [lane.entry.lane]);
      lane.run.head = finished.bundle?.head;
      lane.run.bundle = finished.bundle?.path;
      lane.run.bundleSha256 = finished.bundle?.sha256;
      lane.run.files = Object.keys(finished.files || {});
      if (finished.acceptanceIssues) lane.run.acceptanceIssues = finished.acceptanceIssues;
      lane.run.extraBundles = await bundleExtras(lane);
      if (reason !== 'hard_stop') lane.run.stop = await coordinator('stop', [lane.entry.lane]);
      lane.run.stopped = true;
      lane.run.finishReason = reason;
      // Stopping waits on the shared inbox; other lanes' events in those batches are handled here.
      await archived(lane.run.stop?.removal?.archives);
    } catch (error) {
      lane.run.finishReason = `${reason}_collect_failed`;
      lane.run.collectError = { code: error.code || 'collect_failed', message: String(error.message).slice(0, 300) };
      if (!lane.run.stopped) {
        try { lane.run.stop = await coordinator('stop', [lane.entry.lane]); lane.run.stopped = true; } catch (stopError) { lane.run.stopError = stopError.code || 'stop_failed'; }
      }
    }
    lane.run.finishedAtMs = now();
    lane.finished = true;
    log(lane, 'finished', { reason: lane.run.finishReason, head: lane.run.head });
    await persist(lane);
  };

  const pending = () => lanes.filter(lane => !lane.finished);
  let rereadAll = false;
  // One wake: wait on the shared inbox, answer, deliver notes, detect finishes and hard stops.
  const step = async wakeMinutes => {
    const batch = await coordinator('watch', [String(wakeMinutes)]);
    const touched = await handle(batch.events || []);
    if (rereadAll) { rereadAll = false; for (const lane of pending()) touched.add(lane); }
    for (const lane of pending()) for (const other of await deliverNotes(lane)) touched.add(other);
    for (const lane of pending()) {
      if (now() >= lane.run.launchedAtMs + lane.entry.hardStopMinutes * 60_000) { await collect(lane, 'hard_stop'); continue; }
      // A change seen in a lane's first minute is read at the next wake, not lost.
      // Events can be missed (a worker that finishes before the inbox sees it), so every lane is also read once in its
      // first minute and then every `reconcileMs`; a change seen earlier than that is read at the next wake.
      if (touched.has(lane) || now() - (lane.readAtMs ?? lane.run.launchedAtMs) >= reconcileMs) lane.needsRead = true;
      if (!lane.needsRead || now() < lane.run.launchedAtMs + 60_000) continue;
      lane.needsRead = false;
      lane.readAtMs = now();
      lane.run.statusReads++;
      let status;
      try { status = await coordinator('lanehealth', [lane.entry.lane]); } catch (error) { log(lane, 'status_error', { code: error.code || 'status_failed' }); continue; }
      if (status.state === 'idle' && !status.currentPrompt) await collect(lane, 'finished');
    }
    for (const lane of pending()) if (lane.run.driverError) { (lane.run.previousDriverErrors ??= []).push(lane.run.driverError); delete lane.run.driverError; }
    for (const lane of lanes) await persist(lane);
  };
  // The round is resumable: a rerun reuses every receipt. Say why this driver stopped.
  const failed = async error => {
    for (const lane of pending()) { lane.run.driverError = { code: error.code || 'driver_failed', message: String(error.message).slice(0, 300), at_ms: now() }; await persist(lane); }
  };
  // A new access grant has a new inbox: attach every running lane to it and re-read each one's state.
  const rebrief = async () => { for (const lane of pending()) await launch(lane); rereadAll = true; };
  return { lanes, add, step, pending, failed, rebrief, touchAll: () => { rereadAll = true; }, cli: options.cli || defaultCli };
}

