// MD-DISPLAY-02/04: opt-in local stage diagnostics. Never record page data or IDs.
import { appendFileSync } from 'node:fs';
import path from 'node:path';
export const timestamp = () => performance.timeOrigin + performance.now();
/** `operation`'s result; a stage that took over a quarter second is named in the timing log (fixed labels only). */
export async function slowMark(timing, stage, operation) {
  const at = timestamp();
  try { return await operation; } finally { if (timestamp() - at > 250) timing?.(stage, at); }
}

// MP-08/MP-10/MP-11: the host controller receives a private packet directory,
// not CHARIOX_HOME. Keep the trace there without widening its environment.
export function roomDisplayTiming(environment=process.env) {
  const root=environment.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT??environment.CHARIOX_HOME;
  return typeof root==='string'&&path.isAbsolute(root)?displayTiming(root):()=>{};
}
export function displayTiming(root) {
  // MP-08/MP-10: bounded control diagnostics, one append per frame interval.
  // Per-stage open/write/close otherwise dominates the Node control CPU sample.
  let pending=[],bytes=0,written=0,timer;
  const maxBytes=64*1024*1024;
  const flush=()=>{
    clearTimeout(timer);timer=null;
    if(!pending.length)return;
    if(written+bytes<=maxBytes){appendFileSync(path.join(root,'display-timing.jsonl'),pending.join(''),{mode:0o600});written+=bytes;}
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
  // MP-08/MP-10: per-input/per-frame join records (fixed stage, finite
  // numbers only) so input -> dispatch -> readback -> frame can be matched.
  timing.event = (stage, fields) => {
    if(process.env.CHARIOX_BROWSER_DISPLAY_TIMING!=='1')return;
    const values=Object.fromEntries(Object.entries(fields).filter(([,v])=>Number.isFinite(v)));
    const ended=timestamp(),started=values.at??ended;delete values.at;
    const line=JSON.stringify({stage,started_ms:started,ended_ms:ended,duration_ms:ended-started,...values})+'\n';
    if(bytes+line.length>65536||pending.length>=256)flush();
    pending.push(line);bytes+=line.length;
    if(!timer){timer=setTimeout(flush,16);timer.unref();}
  };
  timing.flush=flush;
  return timing;
}

// MP-08/MP-10/MP-11: private scheduler attribution; never serialize a request,
// identifier, key, coordinate or payload into the diagnostic log.
export function timedHostRequests(host, scheduleRequest) {
  const queued = new WeakMap();
  return {
    scheduleRequest(request) {
      const target=request.method==='host.browser'&&request.params?.op==='display_input_target';
      const display=request.method==='host.browser'&&request.params?.op==='screenshot'&&typeof request.params?.display_subscription_id==='string';
      if (target||display||['host.browser','host.computer'].includes(request.method)&&request.params?.op==='input')
        queued.set(request,{at:timestamp(),stage:target?'host_input_target_queue':display?'host_display_queue':'host_input_queue'});
      return scheduleRequest(request);
    },
    handleRequest(request, options) {
      const started = queued.get(request);
      if (started !== undefined) { queued.delete(request); host.timing(started.stage, started.at); }
      const target=request.method==='host.browser'&&request.params?.op==='display_input_target';
      const display=request.method==='host.browser'&&request.params?.op==='screenshot'&&typeof request.params?.display_subscription_id==='string';
      if(!target&&!display)return host.handle(request,options);
      // Native region input resolves its active Tab before ledger admission.
      // Attribute that separate seam without logging a binding, key or payload.
      const kind=display?host.displays?.get(request.params.display_subscription_id)?.regionTarget?.kind:null;
      const sourceKind={browser_chrome:1,region:2,browser_auxiliary:3,visible_page:4,browser_window:5}[kind]??0;
      const at=timestamp(),complete=()=>display?host.timing.event?.('host_display',{at,source_kind:sourceKind}):host.timing('host_input_target',at);
      let result;
      try{result=host.handle(request,options)}catch(error){complete();throw error}
      if(result&&typeof result.then==='function')return Promise.resolve(result).finally(complete);
      complete();return result;
    },
  };
}
