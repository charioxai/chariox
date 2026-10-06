// MD-DISPLAY-02/04: opt-in local stage diagnostics. Never record page data or IDs.
import { appendFileSync } from 'node:fs';
import path from 'node:path';
export const timestamp = () => performance.timeOrigin + performance.now();
export function displayTiming(root) {
  const timing = (stage, started, finished) => {
    if (process.env.CHARIOX_BROWSER_DISPLAY_TIMING !== '1') return;
    const ended = finished ?? timestamp();
    appendFileSync(path.join(root, 'display-timing.jsonl'), JSON.stringify({
      stage, started_ms: started, ended_ms: ended, duration_ms: ended - started,
    }) + '\n', { mode: 0o600 });
  };
  // MP-08/MP-10: one append for a helper batch; diagnostics must not open a
  // file for every row/stage in the software encode critical path.
  timing.batch = spans => {
    if(process.env.CHARIOX_BROWSER_DISPLAY_TIMING!=='1')return;
    appendFileSync(path.join(root,'display-timing.jsonl'),spans.map(([stage,start,end])=>JSON.stringify({stage,started_ms:start,ended_ms:end,duration_ms:end-start})+'\n').join(''),{mode:0o600});
  };
  return timing;
}
