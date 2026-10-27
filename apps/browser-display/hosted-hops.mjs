// MP-08/MP-10/MP-11: standing r5 numeric-only hop join and serial V/S runner.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {spawn} from 'node:child_process';
import {pathToFileURL} from 'node:url';
import path from 'node:path';
import {typingMetric} from './hosted-hops/probe-config.mjs';
export const metric = values => {
 const v=values.filter(Number.isFinite).sort((a,b)=>a-b);
 return {n:v.length,p50_ms:v.length?v[Math.ceil(v.length*.5)-1]:null,p95_ms:v.length?v[Math.ceil(v.length*.95)-1]:null};
};
const one = rows => {if(rows.length!==1)throw Error('ambiguous_or_missing_join');return rows[0]};
const inWindow=(value,lo,hi)=>Number.isFinite(value)&&value>=lo&&value<=hi;
const reasons=['stale_document','epoch','region_revision','serial_skipped','shift_hold'];
export function rejections(rows,lo,hi){
 if(!rows.some(r=>r.stage==='native_rejected'&&r.ended_ms<=hi))return null;
 const counts=Object.fromEntries(reasons.map(r=>[r,0])),previous=new Map();
 for(const row of [...rows].sort((a,b)=>a.ended_ms-b.ended_ms)){
  if(row.stage!=='native_rejected'||!Number.isSafeInteger(row.reason)||!reasons[row.reason]||!Number.isSafeInteger(row.count)||row.count<0)continue;
  const key=`${row.worker}:${row.reason}`,last=previous.get(key)??0;
  if(inWindow(row.ended_ms,lo,hi))counts[reasons[row.reason]]+=Math.max(0,row.count-last);
  previous.set(key,row.count);
 }
 return counts;
}
export function joinV(cell,source,relay=[]){
 const samples=[],rejected=[],stages=cell.metrics?.stages??[];
 for(const click of cell.clicks??[]){try{
  const seq=click.frame?.sequence,t=click.input,visible=click.visible,draw=click.frame?.drawn;
  const f=one(source.filter(r=>r.stage==='frame_out'&&r.sequence===seq&&inWindow(r.ended_ms,t-1000,visible)));
  const d=one(source.filter(r=>r.stage==='input_dispatch'&&r.kind===1&&inWindow(r.started_ms,t,visible)));
  const send=one(stages.filter(r=>r.stage==='worker_decode_send'&&r.sequence===seq&&inWindow(r.at,f.ended_ms,draw)));
  const reply=one(stages.filter(r=>r.stage==='worker_decode_reply'&&r.worker===send.worker&&r.id===send.id&&inWindow(r.at,send.at,draw)));
  const spans={input_to_host_dispatch:d.started_ms-t,host_inject:d.duration_ms,inject_to_capture:f.started_ms-d.ended_ms,capture_to_publish:f.published_ms-f.started_ms,publish_to_encoded:f.encoded_ms-f.published_ms,encoded_to_frame_out:f.ended_ms-f.encoded_ms,worker_job_decode:reply.at-send.at,worker_reply_to_commit:draw-reply.at,commit_to_native_target:visible-draw,input_to_native_target:visible-t};
  if(relay.length){
   const rx=one(relay.filter(r=>r.stage==='relay_rx'&&r.event_id===seq&&inWindow(r.at_ms,f.ended_ms,send.at)));
   const point=stage=>one(relay.filter(r=>r.stage===stage&&r.event_id===seq&&inWindow(r.at_ms,rx.at_ms,send.at))).at_ms;
   const enq=point('relay_enqueue'),deq=point('relay_dequeue'),written=point('relay_write_complete');
   Object.assign(spans,{frame_out_to_relay_rx:rx.at_ms-f.ended_ms,relay_admission:enq-rx.at_ms,relay_queue:deq-enq,relay_socket_write:written-deq,relay_write_to_worker_combined_network_dispatch:send.at-written});
  }else spans.frame_out_to_worker_combined_transport_dispatch=send.at-f.ended_ms;
  if(Object.values(spans).some(v=>!Number.isFinite(v)))throw Error('missing_numeric_boundary');
  samples.push({sequence:seq,spans_ms:spans});
 }catch(error){rejected.push({sequence:click.frame?.sequence??null,reason:error.message});}}
 return {samples,rejected};
}
export function joinS(cell,native,server){
 const samples=[],rejected=[],stages=cell.metrics?.stages??[];
 for(const click of cell.clicks??[]){try{
  const t=click.input,visible=click.visible;
  const paint=stages.filter(r=>r.stage==='video_present'&&inWindow(r.at,t,visible)).at(-1);if(!paint)throw Error('missing_paint');
  const done=one(stages.filter(r=>r.stage==='decode_done'&&Math.abs(r.stamp-paint.stamp)<1&&inWindow(r.at,t-1000,paint.at)));
  const start=one(stages.filter(r=>r.stage==='decode_start'&&Math.abs(r.stamp-paint.stamp)<1&&inWindow(r.at,t-1000,done.at)));
  const fid=done.frame_id,rx=one(stages.filter(r=>r.stage==='receive'&&r.frame_id===fid&&inWindow(r.at,done.at-300,done.at)));
  const n=one(native.filter(r=>r.frame_id===fid&&inWindow(r.encode_end_ms,rx.at-300,rx.at)));
  const d=one(server.filter(r=>r.stage==='input_dispatch'&&r.kind===1&&r.pressed===1&&inWindow(r.started_ms,t,t+200)));
  const fan=one(server.filter(r=>r.stage==='fanout'&&r.frame_id===fid&&inWindow(r.at_ms,n.encode_end_ms,rx.at)));
  const callback=one(server.filter(r=>r.stage==='encoded_callback'&&r.frame_id===fid&&inWindow(r.at_ms,n.encode_end_ms,fan.at_ms)));
  const spans={input_to_host_dispatch:d.started_ms-t,host_inject:d.duration_ms,inject_to_capture:n.capture_start_ms-d.ended_ms,capture:n.capture_end_ms-n.capture_start_ms,publish_to_encode_start:n.encode_start_ms-n.published_ms,encode:n.encode_end_ms-n.encode_start_ms,encoded_to_callback:callback.at_ms-n.encode_end_ms,callback_to_fanout:fan.at_ms-callback.at_ms,fanout_to_receive_combined_queue_network:rx.at-fan.at_ms,receive_to_decode_start:start.at-rx.at,decode:done.at-start.at,decode_to_expected_display:paint.at-done.at,expected_display_to_native_target:visible-paint.at,input_to_native_target:visible-t};
  if(server.some(r=>r.stage==='video_offer')){
   const offer=one(server.filter(r=>r.stage==='video_offer'&&r.frame_id===fid&&inWindow(r.started_ms,fan.at_ms,rx.at)));
   const send=one(server.filter(r=>r.stage==='socket_send'&&r.video===1&&r.frame_id===fid&&inWindow(r.started_ms,offer.ended_ms,rx.at)));
   Object.assign(spans,{fanout_to_offer:offer.started_ms-fan.at_ms,offer_to_socket_queue:send.started_ms-offer.ended_ms,socket_write:send.duration_ms,socket_end_to_receive_combined_network:rx.at-send.ended_ms});
  }
  if(Object.values(spans).some(v=>!Number.isFinite(v)))throw Error('missing_numeric_boundary');
  samples.push({frame_id:fid,spans_ms:spans});
 }catch(error){rejected.push({input:click.input,reason:error.message});}}
 return {samples,rejected};
}
export function resourceWindow(rows,lo,hi){
 const samples=[];
 for(let i=1;i<rows.length;i++){
  const a=rows[i-1],b=rows[i];if(!inWindow(b.at_ms,lo,hi)||!Array.isArray(a.cpu)||!Array.isArray(b.cpu))continue;
  // guest/guest_nice are already included in user/nice. iowait is idle.
  const delta=b.cpu.slice(0,8).map((v,n)=>v-a.cpu[n]),total=delta.reduce((s,v)=>s+v,0);
  if(total>0&&delta.every(v=>v>=0))samples.push(100*(total-delta[3]-delta[4])/total);
 }
 const cpu=metric(samples);return {start_ms:lo,end_ms:hi,host_cpu_percent:cpu,valid:cpu.n>0&&cpu.p95_ms<=40,minimum_memory_available_kib:Math.min(...rows.filter(r=>inWindow(r.at_ms,lo,hi)).map(r=>r.memory_available_kib).filter(Number.isFinite))||null};
}
const json=async p=>JSON.parse(await readFile(p,'utf8'));
const lines=async p=>p?(await readFile(p,'utf8')).split('\n').filter(Boolean).map(JSON.parse):[];
async function execute(command,env){
 const child=spawn(command[0],command.slice(1),{env:{...process.env,NODE_OPTIONS:'--max-old-space-size=4096',...env},stdio:'inherit'});
 const stop=()=>{if(!Number.isSafeInteger(child.pid)||child.pid<=1)throw Error('MP-11 unsafe hop probe PID');child.kill('SIGTERM')};process.once('SIGTERM',stop);
 try{await new Promise((resolve,reject)=>{child.once('error',reject);child.once('exit',code=>code===0?resolve():reject(Error(`MP-10 probe exit ${code}`)))});}finally{process.removeListener('SIGTERM',stop);}
}
export async function run(manifestFile,out){
 const manifest=await json(manifestFile);await mkdir(out,{recursive:true});
 const outputs={V:[],S:[]},resources=[];
 for(const run of manifest.runs){
  if(!['V','S'].includes(run.path)||!run.identity?.oss||!run.identity?.cloud||!run.identity?.binary_sha256)throw Error('MP-10 exact source/binary identity required');
  if(run.command)await execute(run.command,run.env);
  const report=await json(run.report),source=await lines(run.source),relay=await lines(run.relay),native=await lines(run.native),server=await lines(run.server),host=await lines(run.resources);
  for(const cell of report.cells){
   const joined=run.path==='V'?joinV(cell,source,relay):joinS(cell,native,server);
   const times=[...(cell.clicks??[]).flatMap(c=>[c.input,c.visible]),...(cell.typing??[]).flatMap(c=>[c.physical?.input_at,c.physical?.presented_at]),...(cell.scroll??[]).flatMap(s=>[s.begin,s.end])].filter(Number.isFinite);
   const window=resourceWindow(host,Math.min(...times),Math.max(...times));resources.push({path:run.path,repeat:run.repeat,dpr:cell.dpr,...window});
   const typing=typingMetric(cell.typing??[]);
   const endpoints={click:metric((cell.clicks??[]).map(c=>c.latency??c.physical?.latency_ms)),type:metric(typing.samples)};
   const reasonsFor=times=>times.length?rejections(source,Math.min(...times),Math.max(...times)):null;
   const workload_rejections={click:reasonsFor((cell.clicks??[]).flatMap(c=>[c.input,c.visible]).filter(Number.isFinite)),type:reasonsFor((cell.typing??[]).flatMap(c=>[c.physical?.input_at,c.physical?.presented_at]).filter(Number.isFinite))};
   const keys=new Set(joined.samples.flatMap(s=>Object.keys(s.spans_ms)));
   const record={identity:run.identity,site:run.site??'wikipedia',repeat:run.repeat,dpr:cell.dpr,status:window.valid?'MEASURED_NOT_ACCEPTED':'VOID_RESOURCE_EVIDENCE',endpoints,sample_counts:{click_attempts:cell.clicks?.length??0,type_attempts:typing.attempts},recording:report.recording===true,workload_rejections,hops:Object.fromEntries([...keys].map(k=>[k,metric(joined.samples.map(s=>s.spans_ms[k]))])),...joined,scroll:(cell.scroll??[]).map(s=>({delta:s.delta,hz:s.hz,fps:s.fps,input_count:s.input_count,cpu:s.cpu??null,rejected:rejections(source,s.begin,s.end)})),resources:window};
   outputs[run.path].push(record);
   console.log(JSON.stringify({items:['MP-08','MP-10','MP-11'],path:run.path,repeat:run.repeat,dpr:cell.dpr,status:record.status,endpoints,workload_rejections,scroll:record.scroll}));
  }
 }
 const qualification=['Nearest-rank percentiles; never sum stage percentiles.','Replay reproduces joins only, not new live acceptance. Missing host telemetry or CPU P95 >40% voids a run.','Frame selection is the r5 requested-target X11 observer; negative overlapping capture spans are retained.','Worker job includes messaging; combined network/dispatch spans remain combined.','Full Decision G requires 3 repeats of >=20 click/type samples per site and density plus masking, cores, no PNG and stability gates.'];
 for(const p of ['V','S'])await writeFile(path.join(out,`${p}-hops.json`),JSON.stringify({items:['MP-08','MP-10','MP-11'],qualification,cells:outputs[p]},null,2));
 await writeFile(path.join(out,'quiet-resource-windows.json'),JSON.stringify(resources,null,2));
 return outputs;
}
if(process.argv[1]&&import.meta.url===pathToFileURL(process.argv[1]).href){
 if(process.argv.length!==4)throw Error('MP-08/MP-10/MP-11 usage: node hosted-hops.mjs MANIFEST.json EVIDENCE_DIRECTORY');
 await run(process.argv[2],process.argv[3]);
}
