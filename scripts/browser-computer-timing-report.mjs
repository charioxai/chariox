// MP-08/MP-10/MP-11: report opaque transport and native desktop timing without treating stale frames as input responses.
import{readFile,readdir,writeFile}from'node:fs/promises';
import{pathToFileURL}from'node:url';
import{transportHops}from'./browser-computer-timing.mjs';
export function summarizeComputerTiming(evidence){
const {result,packetRows:rows,viewerRows:views,stages}=evidence;const chains=[],excluded=[];
for(const cell of result.cells)for(const probe of cell.probes??[]){if(!Number.isFinite(probe.paint_ms))continue;
 const paint=views.filter(row=>row.stage==='viewer_paint'&&row.sequence===probe.sequence&&Math.abs(row.at_ms-probe.paint_ms)<100).sort((a,b)=>Math.abs(a.at_ms-probe.paint_ms)-Math.abs(b.at_ms-probe.paint_ms))[0];
 const clockUncertainty=Math.max(1,...views.filter(row=>row.viewer===paint?.viewer&&row.stage==='viewer_clock').map(row=>row.uncertainty_ms??0));
 const receive=views.filter(row=>row.stage==='viewer_receive'&&row.viewer===paint?.viewer&&row.sequence===probe.sequence&&row.at_ms<=probe.paint_ms).sort((a,b)=>b.at_ms-a.at_ms)[0];
 const written=rows.filter(row=>row.packet===receive?.packet&&row.stage==='kernel_write_start'&&row.at_ms<=receive?.at_ms).sort((a,b)=>b.at_ms-a.at_ms)[0];
 const frames=stages.filter(row=>row.stage==='desktop_frame_out'&&row.sequence===probe.sequence&&row.ended_ms<=written?.at_ms);
 const frame=frames.sort((a,b)=>b.ended_ms-a.ended_ms)[0];
 if(!frame||frame.captured_ms<probe.input_ms){excluded.push({site:cell.site,dpr:cell.dpr,kind:probe.kind,sequence:probe.sequence,reason:frame?'frame_captured_before_input':'unmatched_frame',frame_age_at_input_ms:frame?probe.input_ms-frame.captured_ms:null});continue;}
 const inject=stages.filter(row=>row.stage==='computer_x11'&&row.started_ms>=probe.input_ms&&row.injected_ms<=frame?.captured_ms).sort((a,b)=>a.started_ms-b.started_ms)[0];
 const dispatch=stages.find(row=>row.stage==='computer_dispatch'&&row.input===inject?.input);
 const received=rows.find(row=>row.packet===receive?.packet&&row.stage==='relay_read');

 const put=(a,b)=>Number.isFinite(a)&&Number.isFinite(b)&&b>=a?b-a:null;
 const candidates=rows.filter(row=>row.stage==='kernel_input_start'&&row.at_ms<=dispatch?.started_ms&&views.some(v=>v.viewer===paint?.viewer&&v.packet===row.packet&&v.stage==='viewer_send'&&v.at_ms>=probe.input_ms-clockUncertainty&&v.at_ms<=row.at_ms)&&rows.some(end=>end.packet===row.packet&&end.stage==='kernel_input_end'&&end.at_ms>=dispatch?.completed_ms));
 const input=candidates.length===1?candidates[0]:null;
 if(!input||!inject)excluded.push({site:cell.site,dpr:cell.dpr,kind:probe.kind,sequence:probe.sequence,reason:!inject?'unmatched_native_input':'ambiguous_or_unavailable_input_packet',candidate_count:candidates.length});
 const leg=stage=>rows.find(row=>input&&row.packet===input.packet&&row.stage===stage)?.at_ms;

 const sample=result.resources?.filter(row=>Date.parse(row.time)<=probe.input_ms).at(-1);
 chains.push({site:cell.site,dpr:cell.dpr,kind:probe.kind,sequence:probe.sequence,load1:sample?.load1??null,observed_pixel_latency_ms:probe.latency,post_injection_total_ms:candidates.length===1&&inject?probe.latency:null,
 input_packet_unique:candidates.length===1,post_input_capture:true,clock_uncertainty_ms:clockUncertainty,viewer_to_send_ms:put(probe.input_ms,leg('viewer_send')),uplink_ms:put(leg('viewer_send'),leg('relay_read')),relay_admission_ms:put(leg('relay_read'),leg('relay_write_end')),relay_to_kernel_ms:put(leg('relay_write_end'),leg('kernel_receive')),kernel_to_dispatch_ms:put(leg('kernel_receive'),dispatch?.started_ms),viewer_to_dispatch_ms:put(probe.input_ms,dispatch?.started_ms),dispatch_to_x11_ms:put(dispatch?.started_ms,inject?.started_ms),x11_ms:put(inject?.started_ms,inject?.injected_ms),inject_to_capture_ms:put(inject?.injected_ms,frame?.captured_ms),capture_protection_ms:put(frame?.captured_ms,frame?.published_ms),encode_ms:put(frame?.published_ms,frame?.encoded_ms),encode_to_kernel_write_ms:put(frame?.encoded_ms,written?.at_ms),kernel_to_relay_ms:put(written?.at_ms,received?.at_ms),relay_to_viewer_ms:put(received?.at_ms,receive?.at_ms),viewer_to_paint_ms:put(receive?.at_ms,probe.paint_ms)});
}
const summary=values=>{values=values.filter(Number.isFinite).sort((a,b)=>a-b);return{samples:values.length,p50_ms:values[Math.floor(values.length*.5)]??null,p95_ms:values[Math.min(values.length-1,Math.floor(values.length*.95))]??null}};
const stageSummary={};for(const row of stages)if(Number.isFinite(row.duration_ms))(stageSummary[row.stage]??=[]).push(row.duration_ms);
const resources=result.resources??[],loads=resources.map(row=>row.load1).filter(Number.isFinite).sort((a,b)=>a-b),loadMiddle=loads[Math.floor(loads.length*.5)];
const grouped={};for(const row of stages){if(!Number.isFinite(row.duration_ms))continue;const at=row.started_ms??row.captured_ms??row.ended_ms;const load=resources.filter(sample=>Date.parse(sample.time)<=at).at(-1)?.load1;if(!Number.isFinite(load))continue;const group=load<=loadMiddle?'lower_load':'higher_load';((grouped[group]??={})[row.stage]??=[]).push(row.duration_ms);}
const loadStages=Object.fromEntries(Object.entries(grouped).map(([group,values])=>[group,Object.fromEntries(Object.entries(values).map(([stage,values])=>[stage,summary(values)]))]));
return {items:['MP-08','MP-10','MP-11'],identities:{oss:result.oss,cloud:result.cloud},transport:transportHops(rows),excluded,exclusion_counts:Object.fromEntries([...new Set(excluded.map(row=>row.reason))].map(reason=>[reason,excluded.filter(row=>row.reason===reason).length])),chains,chain_summary:Object.fromEntries(Object.keys(chains[0]??{}).filter(key=>key.endsWith('_ms')).map(key=>[key,summary(chains.map(row=>row[key]))])),load_comparison:{split_load1:loadMiddle,range:[loads[0],loads.at(-1)],stages:loadStages,limitation:'Relative load bands from a shared busy host; neither band establishes idle performance or causation.'},stages:Object.fromEntries(Object.entries(stageSummary).map(([stage,values])=>[stage,summary(values)])),limits:['Transport samples join ciphertext digests, not plaintext. Frame sequence joins are restricted to the actual Computer viewer page. Native input-to-frame pairs omit ambiguous/unavailable timestamps. No quiet-host claim unless separately demonstrated.']};
}

export async function readComputerTiming(dir,relay){
const rows=[],views=[];
for(const name of await readdir(dir))if(/^viewer-timing-\d+\.jsonl$/.test(name)){for(const line of(await readFile(dir+'/'+name,'utf8')).split('\n').filter(Boolean)){const row=JSON.parse(line);rows.push(row);views.push({...row,viewer:name});}}
for(const path of [dir+'/kernel.log',...(relay?[relay]:[])])for(const line of(await readFile(path,'utf8')).split('\n')){const start=line.indexOf('MP-10-PACKET-TIMING ');if(start>=0)try{rows.push(JSON.parse(line.slice(start+20)))}catch{}}
const stages=[];for(const name of await readdir(dir))if(/^display-timing-\d+\.jsonl$/.test(name))for(const line of(await readFile(dir+'/'+name,'utf8')).split('\n').filter(Boolean))try{stages.push(JSON.parse(line))}catch{}
return {result:JSON.parse(await readFile(dir+'/result.json')),packetRows:rows,viewerRows:views,stages};
}
if(process.argv[1]&&import.meta.url===pathToFileURL(process.argv[1]).href){
 const dir=process.argv[2];if(!dir)throw Error('MP-10: pass an evidence directory and optional relay log');
 const evidence=await readComputerTiming(dir,process.argv[3]);const report=summarizeComputerTiming(evidence);await writeFile(dir+'/hop-summary.json',JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({items:report.items,packetRows:evidence.packetRows.length,stageRows:evidence.stages.length,chains:report.chains.length,exclusions:report.exclusion_counts}));
}
