// MD-DISPLAY-02: live presentation cadence and independently labelled fidelity.
import { distribution } from './drill-metrics.mjs';
export async function measureWorkload({page,workload,pair,pause,resource,durationMs}) {
 const before=await page.evaluate(()=>({bytes:mdWireBytes,frames:mdFrames.length}));
 const started=performance.now(),cadence=[],samples=[];
 if(workload!=='scroll')await page.evaluate(()=>mdStream.input({kind:'click',x:60,y:88}));
 let last=performance.now();
 while(performance.now()-started<durationMs){
  if(workload==='scroll')await page.evaluate(()=>mdStream.input({kind:'scroll',x:700,y:600,delta_x:0,delta_y:240}));
  const frame=await page.evaluate(()=>mdStream.next());
  if(frame){const now=performance.now();cadence.push(now-last);last=now;samples.push({kind:frame.kind,sequence:frame.sequence,presented_ms:now-started})}
  // Live readback pairs include temporal/compositor drift. Never call them
  // codec PSNR or pixel exactness of the captured encoded source.
  if(samples.length===1||samples.length===4)samples.at(-1).live_pair=await pair(`moving-${samples.length}`);
  await resource();
 }
 const motionEnd=performance.now(),after=await page.evaluate(()=>({bytes:mdWireBytes,frames:mdFrames.slice()}));
 if(workload!=='scroll')await page.evaluate(()=>mdStream.input({kind:'click',x:60,y:88}));
 await pause(300);const settleStart=performance.now();
 // Record the frozen first-frame quality separately from its exact repair.
 const first=await page.evaluate(()=>mdStream.next());const frozen=await pair('motion-frozen-first');
 let refinements=0,exact=frozen;
 while(!exact.lossless&&refinements<300){await page.evaluate(()=>mdStream.next());refinements++;exact=await pair('motion-settled')}
 if(!exact.lossless)throw Error('MD-DISPLAY: moving workload did not settle exactly');
 return {workload,duration_ms:motionEnd-started,presented_frames:samples.length,effective_fps:samples.length*1000/(motionEnd-started),
  cadence:distribution(cadence),samples,received_application_bytes:after.bytes-before.bytes,application_mbps:(after.bytes-before.bytes)*8/(motionEnd-started)/1000,
  frame_kinds:after.frames.slice(before.frames).map(frame=>frame.kind),freeze_first:{kind:first?.kind??'unchanged',fidelity:frozen},
  settle_ms:performance.now()-settleStart,refinements,settled_fidelity:exact,
  limits:'MD-DISPLAY: sequential credits, rAF/canvas presentation proxy; live pairs have temporal drift. Video is a real HTMLVideoElement playing a 30fps canvas captureStream, not DRM/network media. Measurement readbacks/resource sampling reduce cadence.'};
}
