// MP-08 / MP-09 / MP-10 / MP-11: durable stop requests bind the current loopd round, never a classic receipt.
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { LaneError, save } from '../lanes.mjs';

const read = async path => JSON.parse(await readFile(path, 'utf8'));
export const stopRequestPath = (home, key, launchedAtMs) => join(home, 'rounds', `${key}.${launchedAtMs}.stop.json`);
export const matchesStop = (request, run) => request?.key === run.key && request.round === run.round && request.launchedAtMs === run.launchedAtMs
  && ['kernel', 'sessionId', 'agentId'].every(field => request.launch?.[field] === run.launch?.[field]);

export async function requestRoundStop(lane, options) {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]{0,38}$/.test(lane ?? '')) throw new LaneError('invalid_lane', 'Name the loopd lane to stop.');
  const run = await read(join(options.layout.agents, lane, 'LOOP_RUN.json')).catch(error => {
    if (error.code === 'ENOENT') throw new LaneError('round_not_running', `${lane} has no loopd round.`);
    throw error;
  });
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]{0,38}$/.test(run.round ?? '') || run.lane !== lane || run.key !== `${lane}-${run.round}` || !run.launchedAtMs || !run.launch?.agentId) throw new LaneError('round_not_running', `${lane} has no admitted loopd worker.`);
  if (options.expectedRound && run.round !== options.expectedRound) throw new LaneError('round_changed', `${lane} is now ${run.round}; requested ${options.expectedRound}, so no worker was touched.`);
  if (run.finishedAtMs) return { key: run.key, finished: true, finishReason: run.finishReason };
  const request = { key: run.key, round: run.round, launchedAtMs: run.launchedAtMs, launch: run.launch };
  await save(stopRequestPath(options.layout.home, run.key, run.launchedAtMs), request);
  return request;
}

export async function stopRound(lane, options = {}) {
  const home = options.home || process.env.CHARIOX_COORDINATOR_HOME;
  if (!home) throw new LaneError('missing_coordinator_home', 'Run through lanes-kernel.mjs exec so the loop home is set.');
  const agents = options.agents || process.env.LOOPD_AGENTS || '/root/.chariox/dev/browser-resume-20260930/agents';
  const request = await requestRoundStop(lane, { layout: { home, agents }, expectedRound: options.expectedRound });
  if (request.finished) return request;
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    // The archived round survives a successor overwriting LOOP_RUN.json after the lane is released.
    for (const file of [`LOOP_RUN.${request.round}.json`, 'LOOP_RUN.json']) {
      const run = await read(join(agents, lane, file)).catch(error => { if (error.code === 'ENOENT') return undefined; throw error; });
      if (run?.finishedAtMs && matchesStop(request, run)) return { key: run.key, finished: true, finishReason: run.finishReason, head: run.head, bundle: run.bundle };
    }
    await delay(250);
  }
  throw new LaneError('round_stop_pending', `Stop requested for ${request.key}; loopd has not confirmed cleanup. Inspect its log and the archived round, or retry round.mjs stop ${lane} ${request.round}; the request stays armed for that exact round.`);
}
