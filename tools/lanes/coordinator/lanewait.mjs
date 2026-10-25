// MP-08 / MP-09 / MP-10 / MP-11: the coordinator's own event wait. It runs beneath the coordinator's access grant, so it has an
// inbox of its own: it never takes events from loopd. One call blocks until a worker ends a turn, finishes, fails, raises a
// decision loopd does not answer, or goes silent, prints one line per event and returns.
import { readFile, readdir, appendFile, rename, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { LaneError, save } from '../lanes.mjs';
import { coordinate, defaultCli } from './loop.mjs';
import { classifyDecision } from './decisions.mjs';

const read = async path => JSON.parse(await readFile(path, 'utf8'));
const readOr = async (path, fallback) => read(path).catch(error => { if (error.code === 'ENOENT') return fallback; throw error; });
const KEEP_REPLAY = 200;
const SUBSCRIBE_ATTEMPTS = 4;
const SUBSCRIBE_RETRY_MS = 3000;

export const laneAndRound = key => { const match = /^(.+)-(r\d+\w*)$/.exec(key); return match ? { lane: match[1], round: match[2] } : { lane: key, round: '' }; };
export const formatLine = line => `${new Date(line.at_ms).toISOString().slice(11, 19)}Z ${line.lane} ${line.round || '-'} ${line.kind} ${line.summary}`;

// What an event means to the coordinator. Routine permission requests and working/added/resolved noise stay with loopd.
function describe(event) {
  const value = event.value ?? {};
  if (event.type === 'agent') {
    if (event.summary === 'idle') return { kind: 'turn_end', summary: 'the worker ended its turn' };
    if (event.summary === 'error' || /^Provider run failed/.test(event.summary ?? '')) return { kind: 'error', summary: String(value.reason ?? event.summary).slice(0, 200) };
    if (/^Owner cancelled/.test(value.reason ?? '')) return { kind: 'cancelled', summary: 'the worker was cancelled' };
  }
  if (event.type === 'session' && value.state === 'agent_removed') return { kind: 'finished', summary: 'the worker was stopped and removed' };
}

export async function lanewait(options) {
  const { home, receipts, agents, cli = defaultCli, adapter, now = Date.now, minutes = 15, sliceSeconds = 5, healthMs = 5 * 60_000, settleMs = 10_000, log = () => {} } = options;
  const sleep = options.sleep || (ms => new Promise(resolve => setTimeout(resolve, ms)));
  await mkdir(join(home, 'events'), { recursive: true });
  const statePath = join(home, 'state.json');
  const state = await readOr(statePath, { cursor: 0, seq: 0, flags: {}, decisions: {}, tracked: {}, lastHealthMs: 0 });
  const lines = [];
  const emit = async (lane, kind, summary, extra = {}) => {
    const { lane: name, round } = laneAndRound(lane);
    const line = { seq: ++state.seq, at_ms: now(), lane: name, round, key: lane, kind, summary, ...extra };
    lines.push(line);
    await appendFile(join(home, 'emitted.jsonl'), JSON.stringify(line) + '\n', { mode: 0o600 });
  };
  const persist = () => save(statePath, state);
  const coordinator = (operation, args) => coordinate(operation, args, { home: receipts, adapter, cli, now });

  const records = async () => (await Promise.all((await readdir(receipts)).filter(f => /^[A-Za-z0-9][A-Za-z0-9_.-]*\.json$/.test(f) && !/\.(launch|finish|delivery|direct)\.json$/.test(f) && !['config.json', 'subscription.json', 'state.json'].includes(f))
    .map(f => read(join(receipts, f)).catch(() => undefined)))).filter(r => r?.lane && r.launch);
  // The lanes loopd has launched and not stopped; their Room sessions are what this inbox follows.
  const running = async () => (await records()).filter(r => !r.stopped && !state.tracked[r.lane]?.finished);

  let synced = { at: 0, keys: '' };
  // An inbox starts empty and follows nothing: subscribe to every running lane's sessions, and start over when the grant is new.
  const sync = async () => {
    let { subscription } = await cli(['events', 'subscription']);
    if (!subscription || subscription.ended) throw new LaneError('subscription_unavailable', 'The coordinator grant has no inbox; run: lanes-kernel.mjs grant coordinator');
    if (state.subscriptionId && state.subscriptionId !== subscription.subscription_id) {
      await rename(statePath, join(home, `state.${now()}.json`)).catch(() => {});
      Object.assign(state, { cursor: 0, flags: {}, decisions: {}, tracked: {}, lastHealthMs: 0 });
      log('inbox_reset', {});
    }
    state.subscriptionId = subscription.subscription_id;
    const launched = await running();
    synced = { at: now(), keys: launched.map(r => r.lane).sort().join() };
    for (const record of launched) {
      if (!state.tracked[record.lane]) state.lastHealthMs = 0; // a round that ended before it was followed shows in the next read
      state.tracked[record.lane] ??= { startedMs: now() };
      for (const session of [record.launch.sessionId, record.launch.operationsSessionId].filter(Boolean)) {
        if (subscription.targets?.some(t => t.kernel === record.launch.kernel && t.session_id === session && !t.agent_id)) continue;
        // Right after a kernel restart a restored Room is refused for a few seconds; only a refusal that lasts is reported.
        for (let attempt = 1; ; attempt++) {
          try { await cli(['events', 'subscribe', '--kernel', record.launch.kernel, '--session', session]); break; }
          catch (error) {
            if (attempt < SUBSCRIBE_ATTEMPTS) { await sleep(SUBSCRIBE_RETRY_MS); continue; }
            await emit(record.lane, 'error', `cannot follow the worker's room: ${error.code || 'subscribe_failed'}`);
            break;
          }
        }
      }
    }
    await persist();
  };

  // A stopped worker is still followed until its removal event has been seen.
  const targets = async () => {
    const map = new Map();
    for (const record of await records()) if (state.tracked[record.lane] && !state.tracked[record.lane].finished) map.set(`${record.launch.kernel}/${record.launch.sessionId}/${record.launch.agentId}`, record.lane);
    return map;
  };

  // loopd writes the round's result a moment after the removal event; the coordinator gets it in the same line when it is there.
  const finishedSummary = async key => {
    const { lane, round } = laneAndRound(key);
    for (let waited = 0; ; waited += 1000) {
      const run = await readOr(join(agents, lane, 'LOOP_RUN.json'), undefined).catch(() => undefined);
      if (run?.round === round && run.finishedAtMs) return { summary: `${run.finishReason}${run.head ? ` head ${run.head.slice(0, 10)}` : ''}${run.bundle ? ', bundle collected' : ''}${run.driverError ? `, driver error ${run.driverError.code}` : ''}`, reason: run.finishReason, head: run.head };
      if (waited >= settleMs) return { summary: 'removed; loopd is still collecting', reason: 'collecting' };
      await sleep(1000);
    }
  };
  const finish = async (key, summary, extra) => {
    if (state.tracked[key]?.finished) return;
    await emit(key, 'finished', summary, extra);
    (state.tracked[key] ??= {}).finished = true;
    const record = await readOr(join(receipts, `${key}.json`), undefined).catch(() => undefined);
    if (record?.launch) for (const session of [record.launch.sessionId, record.launch.operationsSessionId].filter(Boolean)) {
      await cli(['events', 'unsubscribe', '--kernel', record.launch.kernel, '--session', session]).catch(() => {});
    }
  };

  const interactionLine = async (key, interaction) => {
    if ((state.decisions[key] ??= []).includes(interaction.id)) return;
    const verdict = classifyDecision(interaction);
    if (verdict.routine) return;
    state.decisions[key].push(interaction.id);
    await emit(key, 'decision', `${interaction.title ?? 'decision'} [${verdict.reason}] choices: ${(interaction.choices ?? []).join(', ') || 'none'}`, { interactionId: interaction.id, reason: verdict.reason, choices: interaction.choices ?? [] });
  };

  const handle = async (events, map) => {
    for (const event of events) {
      if (event.cursor <= state.cursor) continue; // delivered before an interrupted acknowledgement
      if (event.type === 'gap') { await emit('inbox', 'attention', 'events were lost; every lane is read again'); state.lastHealthMs = 0; continue; }
      const key = map.get(`${event.kernel}/${event.session_id}/${event.agent_id}`);
      if (!key) continue;
      if (event.type === 'interaction' && event.value?.state === 'pending') {
        const status = await coordinator('lanehealth', [key]).catch(() => undefined);
        const interaction = status?.interactions?.find(item => item.id === event.refs?.interaction_id);
        if (interaction) await interactionLine(key, interaction);
        else if (!(state.decisions[key] ??= []).includes(event.refs?.interaction_id)) {
          state.decisions[key].push(event.refs?.interaction_id);
          await emit(key, 'decision', `${event.summary} [unclassified]`, { interactionId: event.refs?.interaction_id, reason: 'unclassified', choices: [] });
        }
        continue;
      }
      const meaning = describe(event);
      if (meaning?.kind === 'finished') { const done = await finishedSummary(key); await finish(key, done.summary, { reason: done.reason, head: done.head }); }
      else if (meaning) await emit(key, meaning.kind, meaning.summary, event.refs?.run_id ? { runId: event.refs.run_id } : {});
    }
  };

  // The safety net for what events do not say: a worker that went quiet, a round past its limit, a decision raised unseen, a finish
  // whose removal event was missed.
  const health = async () => {
    state.lastHealthMs = now();
    for (const key of Object.keys(state.tracked)) {
      if (state.tracked[key].finished) continue;
      const { lane, round } = laneAndRound(key);
      const run = await readOr(join(agents, lane, 'LOOP_RUN.json'), undefined).catch(() => undefined);
      if (run?.round === round && run.finishedAtMs) { await finish(key, `${run.finishReason}${run.head ? ` head ${run.head.slice(0, 10)}` : ''}`, { reason: run.finishReason, head: run.head }); continue; }
      let status;
      try { status = await coordinator('lanehealth', [key]); } catch { continue; }
      const flags = (state.flags[key] ??= {});
      for (const flag of ['LONG-ROUND', 'SILENT']) {
        if (status.flags?.includes(flag) && !flags[flag]) { flags[flag] = now(); await emit(key, flag === 'SILENT' ? 'silent' : 'long_round', flag === 'SILENT' ? `no activity for ${Math.round((now() - status.lastActivityMs) / 60_000)} min while working` : 'past its round limit'); }
        else if (!status.flags?.includes(flag)) delete flags[flag];
      }
      for (const interaction of status.interactions ?? []) await interactionLine(key, interaction);
    }
  };

  const deadline = now() + minutes * 60_000;
  await sync();
  while (!lines.length) {
    if (now() - state.lastHealthMs >= healthMs) { await health(); await persist(); if (lines.length) break; }
    const left = Math.floor((deadline - now()) / 1000);
    if (left <= 0) break;
    const batch = await cli(['events', 'wait', '--timeout', String(Math.min(sliceSeconds, left))]);
    if (!Array.isArray(batch.events) || !Number.isSafeInteger(batch.next_cursor)) throw new LaneError('invalid_event_batch');
    if (batch.events.length) {
      await save(join(home, 'events', `${now()}-${randomUUID()}.json`), batch);
      if (batch.events.some(e => e.type === 'subscription_ended')) { await emit('inbox', 'attention', 'the coordinator grant ended; run: lanes-kernel.mjs grant coordinator'); break; }
      await handle(batch.events, await targets());
      state.cursor = Math.max(state.cursor, batch.next_cursor);
      await persist();
      await cli(['events', 'ack', String(batch.next_cursor)]);
    } else if (!batch.timed_out) continue;
    // A lane that started meanwhile is followed from here on; the slice is short so its first events are not missed.
    if (!lines.length && (now() - synced.at >= 30_000 || (await running()).map(r => r.lane).sort().join() !== synced.keys)) await sync();
  }
  await persist();
  return lines;
}

export async function replay(home, count) {
  const text = await readFile(join(home, 'emitted.jsonl'), 'utf8').catch(error => { if (error.code === 'ENOENT') return ''; throw error; });
  return text.split('\n').filter(Boolean).slice(-Math.min(count, KEEP_REPLAY)).map(line => JSON.parse(line));
}

export async function main(argv = process.argv.slice(2), env = process.env) {
  const flag = (name, fallback) => { const index = argv.indexOf(name); return index >= 0 ? argv[index + 1] : fallback; };
  const home = flag('--home', env.LANEWAIT_HOME);
  const receipts = flag('--receipts', env.CHARIOX_COORDINATOR_HOME);
  const agents = flag('--agents', env.LOOPD_AGENTS || '/root/.chariox/dev/browser-resume-20260930/agents');
  if (!home || !receipts) throw new LaneError('missing_home', 'Run beneath the coordinator seat: lanes-kernel.mjs exec --seat coordinator -- node lanewait.mjs');
  const json = argv.includes('--json');
  const print = lines => { for (const line of lines) console.log(json ? JSON.stringify(line) : formatLine(line)); };
  if (flag('--replay')) { print(await replay(home, Number(flag('--replay')))); return 0; }
  const minutes = Number(flag('--minutes', 240));
  if (!Number.isFinite(minutes) || minutes <= 0 || minutes > 1440) throw new LaneError('invalid_minutes', '--minutes takes 1..1440');
  const lines = await lanewait({ home, receipts, agents, minutes });
  print(lines);
  return lines.length ? 0 : 3;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  try { process.exitCode = await main(); }
  catch (error) { console.error(JSON.stringify({ error: { code: error.code || 'lanewait_failed', message: error.code ? error.message : 'lanewait failed; inspect the watch directory.' } })); process.exitCode = 1; }
}
