// MD-DISPLAY-02: live presentation cadence and independently labelled fidelity.
import {verifySettled,verifyContinuousSettled,waitQuiet} from './drill-settle.mjs';
import {cpuSpan} from './drill-cpu.mjs';
import { distribution } from './drill-metrics.mjs';
import {measureMotionTyping} from './drill-motion-input.mjs';
export async function measureWorkload({page,workload,pair,pause,resource,durationMs}) {
 const before=await page.evaluate(()=>({bytes:mdWireBytes,frames:mdFrames.length,presentations:mdPresentations?.length??0}));
 const cpuBefore=(await resource()).cpu;
 const started=performance.now(),cadence=[],samples=[],livePairs=[];
 let wheelStats;const wheel=workload==='wheel30'||workload==='wheel60',wheelHz=workload==='wheel60'?60:30;
 if(wheel&&process.env.MD_WINDOW==='1')await page.evaluate(hz=>{window.mdWheel={pending:new Set(),sent:0,dropped:0,error:null};mdWheel.timer=setInterval(()=>{if(mdWheel.pending.size>=4){mdWheel.dropped++;return}mdWheel.sent++;const job=mdStream.input({kind:'scroll',x:700,y:600,delta_x:0,delta_y:120});mdWheel.pending.add(job);job.catch(()=>mdWheel.error='MD-DISPLAY: wheel input failed').finally(()=>mdWheel.pending.delete(job));},1000/hz);mdWheel.hz=hz},wheelHz);
 if(workload!=='scroll'&&!wheel)await page.evaluate(()=>mdStream.input({kind:'click',x:60,y:88}));
 let last=performance.now();const continuous=process.env.MD_WINDOW==='1';
 if(continuous)await page.evaluate(()=>mdStream.start());
 // MP-08/MP-10: opt-in, separate from the unchanged phase29 workload matrix.
 // Keep scroll/wheel active until every typing echo has been measured.
 const typing=process.env.MD_TYPE_DURING_MOTION==='1'&&continuous?measureMotionTyping(page):null;
 typing?.catch(()=>{});
 while(performance.now()-started<durationMs){
  if(workload==='scroll')await page.evaluate(()=>mdStream.input({kind:'scroll',x:700,y:600,delta_x:0,delta_y:240}));
  const frame=continuous?null:await page.evaluate(()=>mdStream.next());
  if(continuous)await pause(33);
  if(frame){const now=performance.now();cadence.push(now-last);last=now;samples.push({kind:frame.kind,sequence:frame.sequence,presented_ms:now-started})}
  // Live readback pairs include temporal/compositor drift. Never call them
  // codec PSNR or pixel exactness of the captured encoded source.
  if(samples.length===1||samples.length===4)samples.at(-1).live_pair=await pair(`moving-${samples.length}`);
  if(continuous&&livePairs.length<2&&performance.now()-started>(livePairs.length+1)*1000)livePairs.push(await pair('moving-live-'+livePairs.length));
  await resource();
 }
 const typingSamples=typing?await typing:null;
 const cpuAfter=(await resource()).cpu;
 let freezeRequestedMs=performance.timeOrigin+performance.now();
 if(wheel&&continuous)wheelStats=await page.evaluate(async()=>{clearInterval(mdWheel.timer);await Promise.allSettled([...mdWheel.pending]);if(mdWheel.error)throw Error(mdWheel.error);return {sent:mdWheel.sent,dropped:mdWheel.dropped,target_hz:mdWheel.hz,max_in_flight:4}});
 const motionEnd=performance.now(),after=await page.evaluate(()=>({bytes:mdWireBytes,frames:mdFrames.slice(),presentations:mdPresentations?.slice()??[]}));
 if(continuous){for(const p of after.presentations.slice(before.presentations)){samples.push({sequence:p.sequence,presented_ms:p.drawn_ms});}for(let i=1;i<samples.length;i++)cadence.push(samples[i].presented_ms-samples[i-1].presented_ms);}
 // MP-08/MP-10: clicked fixtures keep moving until this stop input arrives.
 if(workload!=='scroll'&&!wheel){freezeRequestedMs=performance.timeOrigin+performance.now();await page.evaluate(()=>mdStream.input({kind:'click',x:60,y:88}));}
 const settleStart=performance.now();let first,frozen,repair;
 if(continuous){
  // Credits stay active through the freeze. Readbacks prove exact pixels and
  // carry the canvas presentation snapshot taken atomically with its PNG.
  // The first post-stop presentation is read from the client log; a harness
  // readback here would hold the capture gate in front of exact repair.
  await waitQuiet(()=>page.evaluate(()=>({count:mdPresentations.length,kind:mdPresentations.at(-1)?.kind})));
  first=await page.evaluate(stop=>({kind:mdPresentations.find(p=>p.drawn_ms>stop)?.kind??'unchanged'}),freezeRequestedMs);
  repair=await verifyContinuousSettled(()=>pair('motion-continuous-settled'));
  await page.evaluate(()=>mdStream.stop());
 }else{
  await pause(300);first=await page.evaluate(()=>mdStream.next());frozen=await pair('motion-frozen-first');
  repair=await verifySettled(()=>page.evaluate(()=>mdStream.next()),attempt=>pair('motion-settled-verification-'+attempt));
 }
 const refinements=repair.polls,exact=repair.fidelity,exactPresentedMs=exact.presentation_ms??exact.presentation_drawn_ms;
 if(!exact.lossless)throw Error('MD-DISPLAY: moving workload did not settle exactly');
 return {workload,cpu:cpuSpan(cpuBefore,cpuAfter),wheel_stats:wheelStats,...(typingSamples?{typing:{condition:'concurrent active motion; separate supplementary workload',samples:typingSamples,latency:distribution(typingSamples.map(s=>s.latency_ms))}}:{}),duration_ms:motionEnd-started,presented_frames:samples.length,effective_fps:samples.length*1000/(motionEnd-started),effective_content_fps:after.presentations.slice(before.presentations).filter(p=>p.content_changed).length*1000/(motionEnd-started),
  cadence:distribution(cadence),samples,live_pairs:livePairs,received_application_bytes:after.bytes-before.bytes,application_mbps:(after.bytes-before.bytes)*8/(motionEnd-started)/1000,
  event_mbps:after.frames.slice(before.frames).reduce((n,frame)=>n+frame.bytes,0)*8/(motionEnd-started)/1000,frame_kinds:after.frames.slice(before.frames).map(frame=>frame.kind),freeze_first:{kind:first?.kind??'unchanged',fidelity:frozen},
  settle_ms:performance.now()-settleStart,settle_present_ms:Math.max(0,exactPresentedMs-freezeRequestedMs),settle_definition:continuous?'Continuous credits remain active through freeze and exact repair. settle_present_ms is the presentation bound atomically to independently verified exact RGB, minus freeze request. settle_ms includes readback/verification and stop ownership. No manual300ms pause or prediction counts.':'Manual-credit legacy comparison:300ms pause plus drain/readback; presentation bound to exact RGB snapshot.',refinements,verification_attempts:repair.verification_attempts,settled_fidelity:exact,
  limits:'MD-DISPLAY: bounded continuous or sequential credits, rAF/canvas presentation proxy; live pairs have temporal drift. Video is a real HTMLVideoElement playing a 30fps canvas captureStream, not DRM/network media. Measurement readbacks/resource sampling reduce cadence. application_mbps includes unpaced diagnostic capture responses; event_mbps counts only encrypted frame events.'};
}
