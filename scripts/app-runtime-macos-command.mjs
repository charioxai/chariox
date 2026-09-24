// Separate from Linux's hard-cgroup builder. This runs only trusted build tools.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { startMacResourceWatch, readMacBuildSample, resourceFailure } from './app-runtime-macos-watch.mjs';

const OWNER = fileURLToPath(new URL('./app-runtime-macos-command.py', import.meta.url));

// 'close' can follow 'exit' when another process holds the stdio socket. The
// PID stops being our signaling authority as soon as exit/reaping is observed.
export function ownedGroupSignaler(child, send = process.kill) {
  let exited = false;
  child.once('exit', () => { exited = true; });
  return signal => {
    if (!exited && child.pid) {
      try { send(-child.pid, signal); } catch (error) { if (error.code !== 'ESRCH') throw error; }
    }
  };
}

export async function runMacCommand(command, environment, plan, session, sample = readMacBuildSample) {
  const remaining = Math.floor(session.deadline - performance.now());
  if (remaining <= 0) throw new Error('macOS build deadline reached');
  if (session.signal?.aborted) throw new Error('macOS build stopped: interrupted');
  const child = spawn(plan.tools.python, ['-I', OWNER, command.program, ...command.args], {
    cwd: command.cwd, env: environment, detached: true, stdio: ['ignore', 'inherit', 'inherit', 'pipe'],
  });
  let closed = false; let failure; let timer; let report = ''; let reported = false; let watcher;
  const signalGroup = ownedGroupSignaler(child);
  const completionController = new AbortController();
  const closedPromise = new Promise(resolve => child.once('close', (code, signal) => {
    closed = true; clearTimeout(timer); resolve({ code, signal });
  }));
  function stop(reason) {
    if (closed || failure) return;
    failure = new Error(`macOS build stopped: ${reason}`);
    signalGroup('SIGTERM');
    timer = setTimeout(() => signalGroup('SIGKILL'), 3000);
  }
  const interrupted = () => stop('interrupted');
  process.once('SIGINT', interrupted); process.once('SIGTERM', interrupted);
  session.signal?.addEventListener('abort', interrupted, { once: true });
  child.once('error', () => stop('command-start'));
  const channel = child.stdio[3];
  channel.on('error', () => stop('command-status'));
  async function finishSuccess() {
    let timeout;
    try {
      // Keep the retained leader and monitor alive until no command descendant
      // remains. A tool that backgrounds work is a failed build, then killed.
      const observed = await Promise.race([
        sample(child.pid, plan.scratch, completionController.signal),
        new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error('telemetry')), plan.ciProfile.sampleTimeoutMs); }),
      ]);
      if (closed || failure) return;
      const reason = resourceFailure(observed, session.initialSwapBytes);
      if (reason !== null) { stop(reason); return; }
      session.observe(observed);
      if (observed.groupProcesses !== 1) { stop('surviving-descendants'); return; }
      if (performance.now() >= session.deadline) { stop('deadline'); return; }
      watcher.close(); channel.end('!');
    } catch { stop('completion-telemetry'); }
    finally { clearTimeout(timeout); completionController.abort(); }
  }
  channel.on('data', bytes => {
    report += bytes.toString('ascii');
    if (reported || report.length > 64) { stop('command-status'); return; }
    if (!report.endsWith('\n')) return;
    let parsed;
    try { parsed = JSON.parse(report); } catch { stop('command-status'); return; }
    if (Object.keys(parsed ?? {}).join() !== 'status' || !Number.isInteger(parsed.status)) { stop('command-status'); return; }
    reported = true;
    if (parsed.status !== 0) { stop('command-failed'); return; }
    if (!failure) void finishSuccess();
  });
  try {
    watcher = startMacResourceWatch({
      initialSwapBytes: session.initialSwapBytes, intervalMs: plan.ciProfile.sampleIntervalMs,
      sampleTimeoutMs: plan.ciProfile.sampleTimeoutMs, maxDurationMs: remaining,
      sample: async signal => {
        const observed = await sample(child.pid, plan.scratch, signal);
        session.observe(observed); return observed;
      }, stop,
    });
    if (session.signal?.aborted) interrupted();
    const result = await closedPromise;
    watcher.close();
    const guard = await watcher.done;
    if (failure || !reported || result.code !== 0 || guard.reason !== null) throw failure ?? new Error('macOS command owner exited without successful acknowledgement');
  } finally {
    watcher?.close();
    completionController.abort();
    if (!closed) { signalGroup('SIGKILL'); await closedPromise; }
    channel.destroy(); clearTimeout(timer);
    process.removeListener('SIGINT', interrupted); process.removeListener('SIGTERM', interrupted);
    session.signal?.removeEventListener('abort', interrupted);
  }
}
