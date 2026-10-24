// MP-08 / MP-09 / MP-10 / MP-11: the Mac's round loop for several lanes, event-driven.
import { readFile, stat, writeFile, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { coordinate, save } from './loop.mjs';
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

export async function runRound(spec, options = {}) {
  const now = options.now || Date.now;
  const respond = options.respond || ((input) => execute([new URL('../lane-respond.mjs', import.meta.url).pathname], JSON.stringify(input), process.execPath));
  const coordinator = (operation, args) => coordinate(operation, args, options);
  const adapter = options.adapter || (async (op, input) => execute([new URL(`../lane-${op}.mjs`, import.meta.url).pathname], JSON.stringify(input), process.execPath));
  const home = options.home || spec.home;
  const roundDirectory = join(home, 'rounds');
  await mkdir(roundDirectory, { recursive: true });
  const lanes = [];
  for (const entry of spec.lanes) {
    let run; try { run = await read(entry.runFile); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    run ??= { lane: entry.lane, model: entry.model, provider: entry.provider, worktree: entry.worktree, brief: entry.brief, events: [], notes: [], permissions: [], unanswered: [], statusReads: 0 };
    if (run.driverError) { (run.previousDriverErrors ??= []).push(run.driverError); delete run.driverError; }
    lanes.push({ entry, run, finished: Boolean(run.finishReason) });
  }
  const persist = lane => save(lane.entry.runFile, lane.run);
  const log = (lane, kind, detail) => { lane.run.events.push({ at_ms: now(), kind, ...detail }); lane.run.events = lane.run.events.slice(-KEEP_EVENTS); };
  const byTarget = new Map();

  for (const lane of lanes) {
    if (lane.run.launchedAtMs) continue;
    const brief = await readFile(lane.entry.brief, 'utf8');
    const inbox = await stat(lane.entry.inbox).catch(error => { if (error.code === 'ENOENT') return { size: 0 }; throw error; });
    lane.run.briefSha256 = sha256(brief);
    lane.run.inboxOffset = inbox.size;
    lane.run.launchedAtMs = now();
    await persist(lane);
  }
  // Launch is idempotent: an identical resume reuses the original launch and its deadline.
  for (const lane of lanes) {
    if (lane.finished) continue;
    const record = await coordinator('brief', [lane.entry.lane, lane.entry.worktree, lane.entry.provider, lane.entry.brief]);
    lane.run.launch = { kernel: record.launch.kernel, sessionId: record.launch.sessionId, agentId: record.launch.agentId };
    byTarget.set(`${record.launch.kernel}/${record.launch.sessionId}/${record.launch.agentId}`, lane);
    log(lane, 'launched', { agentId: record.launch.agentId });
    await persist(lane);
  }

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

  const collect = async (lane, reason) => {
    try {
      if (reason === 'hard_stop') { lane.run.stop = await coordinator('stop', [lane.entry.lane]); lane.run.stopped = true; await archived(lane.run.stop?.removal?.archives); }
      const finished = await coordinator('lanefin', [lane.entry.lane]);
      lane.run.head = finished.bundle?.head;
      lane.run.bundle = finished.bundle?.path;
      lane.run.bundleSha256 = finished.bundle?.sha256;
      lane.run.files = Object.keys(finished.files || {});
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
  try {
    while (pending().length) {
      const batch = await coordinator('watch', [String(spec.wakeMinutes ?? 5)]);
      const touched = await handle(batch.events || []);
      for (const lane of pending()) for (const other of await deliverNotes(lane)) touched.add(other);
      for (const lane of pending()) {
        if (now() >= lane.run.launchedAtMs + lane.entry.hardStopMinutes * 60_000) { await collect(lane, 'hard_stop'); continue; }
        if (!touched.has(lane) || now() < lane.run.launchedAtMs + 60_000) continue;
        lane.run.statusReads++;
        let status;
        try { status = await coordinator('lanehealth', [lane.entry.lane]); } catch (error) { log(lane, 'status_error', { code: error.code || 'status_failed' }); continue; }
        if (status.state === 'idle' && !status.currentPrompt) await collect(lane, 'finished');
      }
      for (const lane of lanes) await persist(lane);
    }
  } catch (error) {
    // The round is resumable: a rerun reuses every receipt. Say why this driver stopped.
    for (const lane of pending()) { lane.run.driverError = { code: error.code || 'driver_failed', message: String(error.message).slice(0, 300), at_ms: now() }; await persist(lane); }
    throw error;
  }
  return lanes.map(lane => lane.run);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  try {
    const spec = await read(process.argv[2]);
    spec.home ??= process.env.CHARIOX_COORDINATOR_HOME;
    const runs = await runRound(spec);
    console.log(JSON.stringify(runs.map(run => ({ lane: run.lane, finishReason: run.finishReason, head: run.head }))));
  } catch (error) {
    console.error(JSON.stringify({ error: { code: error.code || 'round_failed', message: error.code ? error.message : 'Round failed; inspect retained receipts.' } }));
    process.exitCode = 1;
  }
}
