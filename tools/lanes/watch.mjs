// MP-08 / MP-10 / MP-11: observe a turn through the real CLI event stream.
import { spawn } from 'node:child_process';
import { LaneError } from './lanes.mjs';

export function waitForIdle(input) {
  const timeout = input.timeout || 600;
  if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 1800) throw new LaneError('invalid_input', 'watch timeout must be 1..1800 seconds');
  return new Promise((ok, bad) => {
    const child = spawn(process.env.CHARIOX_BIN || 'chariox', ['watch', '--kernel', input.kernel, '--session', input.sessionId, '--attachment', input.attachmentId, '--from', '0'], { stdio: ['ignore', 'pipe', 'pipe'] });
    let buffer = '', seen = false, settled = false;
    const stop = () => {
      if (child.exitCode !== null || child.signalCode !== null || !child.pid) return;
      if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new LaneError('pid_ownership_refused');
      child.kill('SIGTERM');
    };
    const finish = (error, result) => {
      if (settled) return; settled = true; clearTimeout(timer);
      stop(); error ? bad(error) : ok(result);
    };
    const timer = setTimeout(() => finish(new LaneError('watch_timeout', 'turn has no idle receipt within the watch timeout')), timeout * 1000);
    child.on('error', () => finish(new LaneError('cli_unavailable')));
    child.on('close', () => finish(new LaneError('watch_ended', 'event stream ended before the target turn became idle')));
    child.stderr.resume();
    child.stdout.on('data', chunk => {
      buffer += chunk.toString();
      if (buffer.length > 4 * 1024 * 1024) return finish(new LaneError('cli_output_limit'));
      while (buffer.includes('\n') && !settled) {
        const end = buffer.indexOf('\n'); const line = buffer.slice(0, end); buffer = buffer.slice(end + 1);
        let frame; try { frame = JSON.parse(line); } catch { return finish(new LaneError('invalid_cli_json')); }
        const activity = frame.event?.agent_activity?.[input.agentId];
        if (!activity) continue;
        const active = activity.active_turn?.prompt_id;
        if (active === input.expectedPromptId) seen = true;
        if (seen && active && active !== input.expectedPromptId) return finish(new LaneError('prompt_changed', 'another prompt became active while waiting'));
        if (seen && !active && activity.status === 'idle' && activity.active_prompt_count === 0) finish(null, { eventId: frame.event_id, promptId: input.expectedPromptId, state: 'idle' });
      }
    });
  });
}
