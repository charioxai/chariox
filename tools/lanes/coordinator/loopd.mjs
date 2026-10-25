// MP-08 / MP-09 / MP-10 / MP-11: the standing round service. One long-lived driver picks up the queue
// entries the coordinator drops and runs every lane through the one shared round engine and inbox.
import { readdir, readFile, rename, mkdir, copyFile, writeFile, access } from 'node:fs/promises';
import { join, basename, extname } from 'node:path';
import { setTimeout as sleepFor } from 'node:timers/promises';
import { LaneError, save, execute } from '../lanes.mjs';
import { createRound } from './engine.mjs';
import { openInbox } from './events.mjs';

const EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max'];
const NAME = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,38}$/;
const MODEL = /^[A-Za-z0-9][A-Za-z0-9._:/-]{0,79}$/;
const LAUNCH_ATTEMPTS = 5;
// What a worker leaves for the coordinator in agents/<lane>/; each round starts with none of them.
const HANDOFF_FILES = ['PUSH_READY.md', 'LANE_STATUS.md', 'ACCEPTANCE_UPDATE.json'];
const exists = path => access(path).then(() => true, error => { if (error.code === 'ENOENT') return false; throw error; });
const read = async path => JSON.parse(await readFile(path, 'utf8'));
const absolute = value => typeof value === 'string' && value.startsWith('/') && !value.includes('\0');
const invalid = (field, problem) => { throw new LaneError('invalid_queue_entry', `${field}: ${problem}`); };

// Check a queue entry once at the boundary; everything after trusts the result.
export function parseEntry(fileName, text) {
  let raw; try { raw = JSON.parse(text); } catch { invalid('file', 'is not valid JSON'); }
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) invalid('file', 'must be a JSON object');
  if (!NAME.test(raw.lane ?? '')) invalid('lane', 'must be a lane name (letters, digits, . _ -)');
  if (!NAME.test(raw.round ?? '')) invalid('round', 'must be a round id such as r35');
  if (fileName !== `${raw.lane}-${raw.round}.json`) invalid('file', `must be named ${raw.lane}-${raw.round}.json`);
  if (!['claude', 'codex'].includes(raw.provider)) invalid('provider', 'must be claude or codex');
  if (typeof raw.model !== 'string' || !MODEL.test(raw.model)) invalid('model', 'must be a model id');
  if (!EFFORTS.includes(raw.effort)) invalid('effort', `must be one of ${EFFORTS.join(', ')}`);
  if (!Number.isSafeInteger(raw.hardStopMinutes) || raw.hardStopMinutes < 1 || raw.hardStopMinutes > 1440) invalid('hardStopMinutes', 'must be a whole number of minutes, 1..1440');
  if (!absolute(raw.brief)) invalid('brief', 'must be an absolute path');
  if (raw.notes !== undefined && !absolute(raw.notes)) invalid('notes', 'must be an absolute path');
  const preamble = raw.preamble ?? [];
  if (!Array.isArray(preamble) || preamble.length > 8) invalid('preamble', 'must list up to 8 absolute file paths');
  preamble.forEach((path, index) => { if (!absolute(path)) invalid(`preamble[${index}]`, 'needs an absolute path'); });
  if (!Array.isArray(raw.worktrees) || !raw.worktrees.length || raw.worktrees.length > 4) invalid('worktrees', 'must list 1..4 worktrees; the first is the one the worker starts in');
  const worktrees = raw.worktrees.map((item, index) => {
    const worktree = typeof item === 'string' ? { path: item } : item;
    if (!worktree || !absolute(worktree.path)) invalid(`worktrees[${index}]`, 'needs an absolute path');
    if (worktree.base !== undefined && !/^[a-f0-9]{40}$/.test(worktree.base)) invalid(`worktrees[${index}].base`, 'must be a full 40-character commit id');
    return { path: worktree.path, base: worktree.base };
  });
  return { lane: raw.lane, round: raw.round, key: `${raw.lane}-${raw.round}`, provider: raw.provider, model: raw.model, effort: raw.effort, hardStopMinutes: raw.hardStopMinutes, brief: raw.brief, preamble, notes: raw.notes, worktrees };
}

export async function serve(options) {
  const { layout, config, signal, wakeMinutes = 0.25, idleMs = 5000, retryMs = 30_000 } = options;
  const now = options.now || Date.now;
  const sleep = options.sleep || (ms => sleepFor(ms, undefined, { signal }).catch(() => {}));
  const git = options.git || (argv => execute(argv, '', 'git', true));
  const log = options.log || (() => {});
  const queue = layout.queue, home = layout.home;
  const round = createRound({ ...options, home, config });
  const active = new Map();
  const attempts = new Map();
  for (const directory of ['', 'done', 'rejected']) await mkdir(join(queue, directory), { recursive: true });
  await mkdir(join(home, 'rounds'), { recursive: true });
  config.repository ??= join(home, 'imported.git');
  config.laneProfiles ??= {}; config.laneDirectories ??= {}; config.bases ??= {};

  // The previous round's handoff files are set aside once, when the new round is first admitted (before its worker
  // exists), so a resume never moves the files the running round has written.
  const archiveHandoff = async (entry, laneDirectory) => {
    const previous = await read(join(laneDirectory, 'LOOP_RUN.json')).catch(error => { if (error.code === 'ENOENT') return undefined; throw error; });
    const label = previous ? previous.round ?? `legacy-${previous.launchedAtMs}` : `before-${entry.round}`;
    for (const name of HANDOFF_FILES) {
      if (!await exists(join(laneDirectory, name))) continue;
      const extension = extname(name);
      await rename(join(laneDirectory, name), join(laneDirectory, `${basename(name, extension)}.${label}${extension}`));
    }
  };

  // Workers get only their brief, so the shared rules named by the entry are appended to it. The result is written once:
  // a resumed round must see the brief it was launched with, even if a shared file changed meanwhile.
  const composeBrief = async entry => {
    if (!entry.preamble.length) return entry.brief;
    const composed = join(home, 'rounds', `${entry.key}.brief.md`);
    if (await exists(composed)) return composed;
    const parts = [entry.brief, ...entry.preamble];
    const texts = await Promise.all(parts.map(path => readFile(path, 'utf8').catch(error => { if (error.code === 'ENOENT') throw new LaneError('preamble_unavailable', `${path} does not exist; fix the entry's brief or preamble list.`); throw error; })));
    await writeFile(composed, `${texts.map(text => text.trimEnd()).join('\n\n')}\n`, { mode: 0o600 });
    return composed;
  };

  // Bases are fixed at the first sighting of an entry, so a restarted service bundles the same round.
  const prepare = async entry => {
    const specPath = join(home, 'rounds', `${entry.key}.spec.json`);
    const laneDirectory = join(layout.agents, entry.lane);
    let spec; try { spec = await read(specPath); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    if (!spec) {
      await archiveHandoff(entry, laneDirectory);
      await git(['init', '--bare', '-q', config.repository]);
      const bases = [];
      for (const [index, worktree] of entry.worktrees.entries()) {
        const base = worktree.base || String(await git(['-C', worktree.path, 'rev-parse', 'HEAD'])).trim();
        try { await git(['-C', config.repository, 'fetch', '--no-tags', '-q', worktree.path, worktree.base || 'HEAD']); }
        catch { throw new LaneError('base_unavailable', `worktrees[${index}]: cannot read ${base} from ${worktree.path}; check the path and the commit.`); }
        bases.push(base);
      }
      spec = { bases };
      await save(specPath, spec);
    }
    // A file left by another round (or by a driver that predates round ids) stays next to the new one.
    const previous = await read(join(laneDirectory, 'LOOP_RUN.json')).catch(error => { if (error.code === 'ENOENT') return undefined; throw error; });
    if (previous && previous.round !== entry.round) await copyFile(join(laneDirectory, 'LOOP_RUN.json'), join(laneDirectory, `LOOP_RUN.${previous.round ?? `legacy-${previous.launchedAtMs}`}.json`));
    config.laneProfiles[entry.key] = { model: entry.model, effort: entry.effort, deadlineSeconds: (entry.hardStopMinutes + 15) * 60 };
    config.laneDirectories[entry.key] = laneDirectory;
    config.bases[entry.key] = spec.bases[0];
    return {
      lane: entry.key, name: entry.lane, round: entry.round, provider: entry.provider, model: entry.model, effort: entry.effort,
      worktree: entry.worktrees[0].path, brief: await composeBrief(entry), inbox: entry.notes || join(laneDirectory, 'REVIEW_INBOX.md'),
      runFile: join(laneDirectory, 'LOOP_RUN.json'), lastMessageFile: join(laneDirectory, 'last-message.md'), hardStopMinutes: entry.hardStopMinutes,
      extraBundles: entry.worktrees.slice(1).map((worktree, index) => ({ name: `${basename(worktree.path).replace(/[^A-Za-z0-9_.-]/g, '-')}`, worktree: worktree.path, base: spec.bases[index + 1], repository: config.repository })),
    };
  };

  const reject = async (name, error) => {
    await rename(join(queue, name), join(queue, 'rejected', name));
    await writeFile(join(queue, 'rejected', `${name}.reason.txt`), `${error.code || 'rejected'}: ${error.message}\nFix the entry and drop it into the queue again.\n`, { mode: 0o600 });
    log('rejected', { name, code: error.code, message: error.message });
  };

  const give_up = async (entry, error) => {
    const laneDirectory = join(layout.agents, entry.lane);
    await save(join(laneDirectory, 'LOOP_RUN.json'), { lane: entry.lane, key: entry.key, round: entry.round, provider: entry.provider, model: entry.model, effort: entry.effort, events: [], launchError: { code: error.code || 'launch_failed', message: String(error.message).slice(0, 300) }, finishReason: 'launch_failed', finishedAtMs: now() });
    await rename(join(queue, `${entry.key}.json`), join(queue, 'done', `${entry.key}.json`));
    log('launch_failed', { key: entry.key, code: error.code });
  };

  // A round that was launched and has not finished is still running, whichever service started it.
  const unfinished = async entry => {
    const run = await read(join(layout.agents, entry.lane, 'LOOP_RUN.json')).catch(error => { if (error.code === 'ENOENT') return undefined; throw error; });
    return Boolean(run && run.round === entry.round && run.launchedAtMs && !run.finishReason);
  };

  const scan = async () => {
    const entries = [];
    for (const name of (await readdir(queue)).filter(file => file.endsWith('.json')).sort()) {
      const key = name.slice(0, -5);
      if (active.has(key) || (attempts.get(key)?.nextAtMs ?? 0) > now()) continue;
      try { entries.push(parseEntry(name, await readFile(join(queue, name), 'utf8'))); } catch (error) { if (error.code !== 'invalid_queue_entry') throw error; await reject(name, error); }
    }
    // Restored rounds go first, so a waiting entry that sorts earlier cannot take their lane.
    const resumable = new Set();
    for (const entry of entries) if (await unfinished(entry)) resumable.add(entry.key);
    entries.sort((a, b) => Number(resumable.has(b.key)) - Number(resumable.has(a.key)));
    for (const entry of entries) {
      const { key } = entry;
      // One round per lane at a time: a second entry for the lane waits in the queue.
      if ([...active.values()].some(item => item.entry.lane === entry.lane)) continue;
      try {
        const lane = await round.add(await prepare(entry));
        active.set(key, { entry, lane });
        attempts.delete(key);
        log('picked_up', { key });
      } catch (error) {
        const count = (attempts.get(key)?.count ?? 0) + 1;
        attempts.set(key, { count, nextAtMs: now() + retryMs });
        log('launch_error', { key, count, code: error.code, message: error.message });
        if (count >= LAUNCH_ATTEMPTS) await give_up(entry, error);
      }
    }
  };

  const finalize = async () => {
    for (const [key, { entry, lane }] of active) {
      if (!lane.finished) continue;
      const laneDirectory = join(layout.agents, entry.lane);
      await copyFile(join(laneDirectory, 'LOOP_RUN.json'), join(laneDirectory, `LOOP_RUN.${entry.round}.json`));
      await rename(join(queue, `${key}.json`), join(queue, 'done', `${key}.json`));
      active.delete(key);
      log('finished', { key, reason: lane.run.finishReason });
    }
  };

  // A new grant (kernel restart) has a new inbox: the old one is gone, so keep it aside and re-read every lane.
  const resetStaleInbox = async () => {
    const path = join(home, 'subscription.json');
    let saved; try { saved = await read(path); } catch (error) { if (error.code === 'ENOENT') return; throw error; }
    const { subscription } = await round.cli(['events', 'subscription']);
    if (subscription && saved.subscription_id !== subscription.subscription_id) {
      await rename(path, join(home, `subscription.${now()}.json`));
      round.touchAll();
      log('inbox_reset', {});
    }
  };

  await resetStaleInbox();
  // Settle the inbox before the first lane: targets of workers stopped by an earlier service are dropped now.
  await openInbox(home, round.cli).catch(error => log('inbox_unavailable', { code: error.code, message: error.message }));
  while (!signal?.aborted) {
    try {
      await scan();
      await finalize();
      if (!round.pending().length) { await sleep(idleMs); continue; }
      try { await round.step(wakeMinutes); }
      catch (error) {
        if (error.code === 'subscription_changed') { await resetStaleInbox(); await round.rebrief(); continue; }
        await round.failed(error);
        log('driver_error', { code: error.code, message: error.message });
        await sleep(retryMs);
      }
      await finalize();
    } catch (error) {
      log('service_error', { code: error.code, message: error.message });
      await sleep(retryMs);
    }
  }
  return [...active.keys()];
}

// Entry point for `round.mjs serve`: one service per lanes-kernel home, stopped by SIGTERM at a safe point.
export async function main(env = process.env) {
  const home = env.CHARIOX_COORDINATOR_HOME;
  if (!home) throw new LaneError('missing_coordinator_home', 'Set CHARIOX_COORDINATOR_HOME to the loop home (lanes-kernel.mjs exec sets it).');
  const agents = env.LOOPD_AGENTS || '/root/.chariox/dev/browser-resume-20260930/agents';
  const controller = new AbortController();
  for (const signal of ['SIGTERM', 'SIGINT']) process.on(signal, () => controller.abort());
  const log = (kind, detail) => console.log(JSON.stringify({ at: new Date().toISOString(), kind, ...detail }));
  log('started', { pid: process.pid, queue: join(agents, '_loop', 'queue') });
  const running = await serve({ layout: { agents, queue: join(agents, '_loop', 'queue'), home }, config: await read(join(home, 'config.json')), signal: controller.signal, log });
  log('stopped', { stillRunning: running });
}
