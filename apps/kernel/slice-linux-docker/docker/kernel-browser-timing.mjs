// MD-DISPLAY-02/04: opt-in local stage diagnostics. Never record page data or IDs.
import { appendFileSync } from 'node:fs';
import path from 'node:path';
export const timestamp = () => performance.timeOrigin + performance.now();
export function displayTiming(root) {
  return (stage, started) => {
    if (process.env.CHARIOX_BROWSER_DISPLAY_TIMING !== '1') return;
    const ended = timestamp();
    appendFileSync(path.join(root, 'display-timing.jsonl'), JSON.stringify({
      stage, started_ms: started, ended_ms: ended, duration_ms: ended - started,
    }) + '\n', { mode: 0o600 });
  };
}
