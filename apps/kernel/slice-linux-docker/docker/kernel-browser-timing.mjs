// MD-DISPLAY-02/04: opt-in local stage diagnostics. Never record page data or IDs.
import { appendFileSync } from 'node:fs';
import path from 'node:path';
export const timestamp = () => performance.timeOrigin + performance.now();
export function displayTiming(root) {
  // MP-08/MP-10: bounded control diagnostics, one append per frame interval.
  // Per-stage open/write/close otherwise dominates the Node control CPU sample.
  let pending=[],bytes=0,timer;
  const flush=()=>{
    clearTimeout(timer);timer=null;
    if(!pending.length)return;
    appendFileSync(path.join(root,'display-timing.jsonl'),pending.join(''),{mode:0o600});
    pending=[];bytes=0;
  };
  const timing = (stage, started, finished) => {
    if (process.env.CHARIOX_BROWSER_DISPLAY_TIMING !== '1') return;
    const ended = finished ?? timestamp();
    const line=JSON.stringify({
      stage, started_ms: started, ended_ms: ended, duration_ms: ended - started,
    })+'\n';
    if(bytes+line.length>65536||pending.length>=256)flush();
    pending.push(line);bytes+=line.length;
    if(!timer){timer=setTimeout(flush,16);timer.unref();}
  };
  // MP-08/MP-10: one append for a helper batch; diagnostics must not open a
  // file for every row/stage in the software encode critical path.
  timing.batch = spans => {
    if(process.env.CHARIOX_BROWSER_DISPLAY_TIMING!=='1')return;
    for(const [stage,start,end]of spans)timing(stage,start,end);
  };
  // MP-10/MP-11: private fixed-label driver diagnostics, never frame transport.
  timing.hardware = diagnostic => {
    if(process.env.CHARIOX_BROWSER_DISPLAY_TIMING!=='1'||typeof diagnostic!=='string'||diagnostic.length>4096)return;
    appendFileSync(path.join(root,'display-hardware.jsonl'),JSON.stringify({diagnostic})+'\n',{mode:0o600});
  };
  timing.flush=flush;
  return timing;
}
