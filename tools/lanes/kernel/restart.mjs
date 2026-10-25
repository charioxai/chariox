// MP-08 / MP-09 / MP-10 / MP-11: preserve every admitted round across the coordinator's planned restart.
import { readdir, readFile, writeFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
const read = async path => JSON.parse(await readFile(path, 'utf8'));

export async function runningRounds(agents) {
  const rounds = [];
  for (const entry of await readdir(agents, { withFileTypes: true })) {
    if (!entry.isDirectory() || entry.name.startsWith('_')) continue;
    const path = join(agents, entry.name, 'LOOP_RUN.json');
    let run; try { run = await read(path); } catch (error) { if (error.code === 'ENOENT') continue; throw error; }
    if (run.launchedAtMs && !run.finishedAtMs) rounds.push({ path, ...run });
  }
  return rounds;
}

export async function restartWithSandbox({ state, agents, preflight, stopLoopd, startLoopd, loopdRunning = async () => false, stop, start, inspect, report = console.log,
  now = Date.now, sleep = ms => new Promise(resolve => setTimeout(resolve, ms)), timeoutMs = 360_000 }) {
  // Resolve prerequisites before interrupting any service. stopLoopd finishes its current step before the snapshot.
  const account = await preflight();
  const wasRunning = await loopdRunning();
  let loopd, before, previousLog;
  const workers = new Map();
  const logPath = join(state, 'loopd.log');
  try {
    loopd = await stopLoopd();
    before = await runningRounds(agents);
    for (const run of before) {
      const worker = await inspect(run);
      if (worker.state === 'error' || !worker.provider || !worker.permission) throw new Error(`round ${run.key} has no healthy worker or permission level`);
      workers.set(run.key, worker);
    }
    await writeFile(join(state, 'restart-rounds.json'), JSON.stringify(before.map(({ path, key, round, launch, launchedAtMs }) => ({ path, key, round, launch, launchedAtMs }))), { mode: 0o600 });
    await writeFile(join(state, 'kernel-environment.json'), JSON.stringify({ IS_SANDBOX: '1' }) + '\n', { mode: 0o600 });
    previousLog = await readFile(logPath, 'utf8').catch(error => { if (error.code === 'ENOENT') return ''; throw error; });
  } catch (error) {
    if (loopd?.stopped || wasRunning) {
      try { await startLoopd(); } catch (restore) {
        throw new Error(`restart precheck failed: ${error.message}; loopd restore failed: ${restore.message}`, { cause: error });
      }
    }
    throw new Error(`restart precheck failed: ${error.message}; kernels were not restarted; loopd ${loopd?.stopped || wasRunning ? "restored" : "left as it was"}`, { cause: error });
  }
  const restartedAtMs = now();
  await stop();
  await start();
  const deadline = now() + timeoutMs;
  const pending = new Map(before.map(run => [run.key, run]));
  while (pending.size && now() < deadline) {
    const lines = (await readFile(logPath, 'utf8')).slice(previousLog.length).trim().split('\n');
    const pickedUp = new Set(lines.flatMap(line => { try { const e = JSON.parse(line); return e.kind === 'picked_up' ? [e.key] : []; } catch { return []; } }));
    for (const [key, old] of pending) {
      let run = await read(old.path);
      if (run.key !== key || run.round !== old.round) {
        run = await read(join(dirname(old.path), `LOOP_RUN.${old.round}.json`)).catch(() => { throw new Error(`round identity changed: ${key}; its archive is missing`); });
      }
      if (run.key !== key || run.round !== old.round || run.launchedAtMs !== old.launchedAtMs || JSON.stringify(run.launch) !== JSON.stringify(old.launch)) throw new Error(`round identity changed: ${key}; inspect restart-rounds.json`);
      if (run.driverError) throw new Error(`round ${key} failed to resume: ${run.driverError.code}`);
      if (!pickedUp.has(key)) continue;
      if (run.finishedAtMs && run.finishReason !== 'finished') throw new Error(`round ${key} ended during restart: ${run.finishReason}`);
      const worker = run.finishedAtMs ? { ...workers.get(key), providerActivityAtMs: undefined, completedAtMs: run.completedTurn?.completedAtMs, completedSuccessfully: run.completedTurn?.status === 'completed' } : await inspect(run);
      if (worker.state === 'error' || !worker.provider || !worker.permission) throw new Error(`round ${key} has no healthy worker or permission level`);
      if (worker.provider !== workers.get(key).provider || worker.permission !== workers.get(key).permission) throw new Error(`worker launch selection changed: ${key}`);
      // Receipt replay and replayed open decisions prove attachment, not provider recovery.
      const freshRun = worker.providerActivityAtMs > restartedAtMs
        && worker.providerActivityAtMs > (workers.get(key).providerActivityAtMs ?? 0);
      const completed = worker.completedAtMs > restartedAtMs && worker.completedSuccessfully;
      if (!freshRun && !completed) continue;
      report(`${key} provider=${worker.provider} permission=${worker.permission} account=${worker.provider === 'claude' ? worker.account ?? 'unreported' : 'n/a'} resumed=true${run.finishedAtMs ? ' completed=true' : ''}`);
      pending.delete(key);
    }
    if (pending.size) await sleep(500);
  }
  if (pending.size) throw new Error(`rounds did not resume: ${[...pending.keys()].join(', ')}; inspect restart-rounds.json and loopd.log`);
  report(`restart verified: ${before.length} running rounds carried; Claude YOLO applies to next rounds; existing permissions retained`);
  return { resumed: before.map(run => run.key), account, nextClaudePermission: 'yolo' };
}
