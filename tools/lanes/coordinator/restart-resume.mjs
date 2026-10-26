// MP-08 / MP-09 / MP-10 / MP-11: retry only failures of a turn interrupted by kernel startup.
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { save } from '../lanes.mjs';

const SPACING_MS = 60_000;
const RECOVERY_WINDOW_MS = 360_000;
const rateLimited = reason => /response_too_many_failed_attempts|rate_limit_exceeded|429 Too Many Requests/i.test(reason ?? '');
const interrupted = reason => reason === 'provider run was already ended during liveness reconciliation';

function failureFor(lane, status, promptId, sinceMs) {
  const turn = status.completedTurn;
  if (turn?.promptId === promptId) {
    if (turn.status !== 'failed' || turn.completedAtMs < sinceMs) return;
    const reason = turn.providerTermination?.reason;
    if (rateLimited(reason)) return { kind: 'rate_limited', atMs: turn.completedAtMs };
    if (turn.providerTermination?.category === 'runtime_failure' && interrupted(reason)) return { kind: 'interrupted', atMs: turn.completedAtMs };
    if (turn.providerTermination) return;
  }
  // A failed recovery can precede attachment to the new inbox. When it is delivered, the kernel's
  // task event names the exact interrupted prompt; an unrelated worker's error is never enough.
  for (const event of lane.run.events.toReversed()) {
    if (event.taskId !== promptId || event.at_ms < sinceMs) continue;
    if (rateLimited(event.summary)) return { kind: 'rate_limited', atMs: event.at_ms };
  }
}

export async function resumeAfterRestart({ lane, lanes, status, kernelStartedAtMs, now, home, send, persist, log }) {
  const run = lane.run;
  if (run.restartRecovery) run.restartRecovery.pending = false;
  if (status.state !== 'error' || status.currentPrompt || status.busy) return false;
  if (!Number.isSafeInteger(kernelStartedAtMs) || kernelStartedAtMs <= run.launchedAtMs) return false;
  let recovery = run.restartRecovery;
  if (recovery?.kernelStartedAtMs !== kernelStartedAtMs) recovery = undefined;
  const promptId = recovery?.promptId ?? status.admittedPromptId ?? run.currentPromptId ?? lane.receipt.promptId;
  if (!recovery && status.admittedPromptAtMs >= kernelStartedAtMs) return false;
  // A provider can fail before the admission reply reaches loopd.
  const sinceMs = recovery?.sentAtMs ? recovery.attemptedAtMs : kernelStartedAtMs;
  const failure = failureFor(lane, status, promptId, sinceMs);
  if (!failure) return false;
  if (!recovery && failure.atMs > kernelStartedAtMs + RECOVERY_WINDOW_MS) return false;
  // Once a resume was admitted, only its 429 may cause another submission. Other errors need a person.
  if (recovery?.sentAtMs && failure.kind !== 'rate_limited') return false;
  recovery ??= run.restartRecovery = { kernelStartedAtMs, promptId, attempt: 1 };
  recovery.pending = true;
  const schedulePath = join(home, 'restart-resume.json');
  let schedule;
  try { schedule = JSON.parse(await readFile(schedulePath, 'utf8')); }
  catch (error) { if (error.code !== 'ENOENT') throw error; }
  const lastAttempt = Math.max(schedule?.attemptedAtMs ?? 0, ...lanes.map(item => item.run.restartRecovery?.attemptedAtMs ?? 0));
  if (now() < Math.max(lastAttempt + SPACING_MS, recovery.retryAtMs ?? 0)) return true;
  // A repeatedly refused worker cannot monopolize the slot ahead of one that has never been resumed.
  if (lanes.some(other => other !== lane && !other.finished && !other.stopping && other.run.restartRecovery?.pending
    && (other.run.restartRecovery.attemptedAtMs ?? 0) < (recovery.attemptedAtMs ?? 0))) return true;
  if (recovery.sentAtMs) {
    recovery.attempt++;
    delete recovery.sentAtMs;
  }
  const file = join(home, 'rounds', `${lane.entry.lane}.restart-${kernelStartedAtMs}-${recovery.attempt}.md`);
  await writeFile(file, `## Kernel restart ${kernelStartedAtMs}, recovery ${recovery.attempt}\nThe kernel restart cut your call. Check detached jobs before rerunning work, then continue your lane brief.\n`, { mode: 0o600 });
  recovery.attemptedAtMs = now();
  recovery.retryAtMs = now() + SPACING_MS;
  // Save before sending: a lost answer replays the existing direct-steer receipt, never a new prompt.
  await persist(lane);
  await save(schedulePath, { attemptedAtMs: recovery.attemptedAtMs });
  try {
    const result = await send(lane.entry.lane, file, recovery.promptId);
    if (!result.promptId) throw new Error('Restart resume returned no prompt id; inspect the direct delivery receipt.');
    recovery.promptId = result.promptId;
    recovery.sentAtMs = now();
    recovery.pending = false;
    run.currentPromptId = result.promptId;
    delete recovery.error;
    log(lane, 'restart_resumed', { attempt: recovery.attempt, promptId: result.promptId, mode: result.mode });
  } catch (error) {
    recovery.error = error.code || 'resume_failed';
    log(lane, 'restart_resume_failed', { attempt: recovery.attempt, code: recovery.error, retryAtMs: recovery.retryAtMs });
  }
  await persist(lane);
  return true;
}
