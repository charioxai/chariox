// MP-08/MP-10/MP-11 supplementary guards for numeric joins and resource gating.
import test from 'node:test';import assert from 'node:assert/strict';
import {metric,joinV,rejections,resourceWindow,run} from './hosted-hops.mjs';
test('MP-10 nearest rank never averages or sums stage percentiles',()=>{
 assert.deepEqual(metric([1,2,3,4]),{n:4,p50_ms:2,p95_ms:4});
 assert.deepEqual(metric([]),{n:0,p50_ms:null,p95_ms:null});
});
test('MP-10 joins reject an ambiguous source frame rather than selecting a favorable sample',()=>{
 const f={stage:'frame_out',sequence:1,started_ms:12,ended_ms:20,published_ms:14,encoded_ms:18};
 const d={stage:'input_dispatch',kind:1,started_ms:10,ended_ms:11,duration_ms:1};
 const cell={clicks:[{input:9,visible:30,frame:{sequence:1,drawn:25}}],metrics:{stages:[{stage:'worker_decode_send',worker:1,id:2,sequence:1,at:21},{stage:'worker_decode_reply',worker:1,id:2,at:24}]}};
 assert.equal(joinV(cell,[f,d]).samples.length,1);
 assert.equal(joinV(cell,[f,f,d]).samples.length,0);
 assert.equal(joinV(cell,[f,f,d]).rejected.length,1);
});
test('MP-10 host load above40% or missing samples voids the run',()=>{
 const row=(at,cpu)=>({at_ms:at,cpu,memory_available_kib:20_000_000});
 assert.equal(resourceWindow([],0,200).valid,false);
 assert.equal(resourceWindow([row(0,[0,0,0,0,0,0,0,0]),row(100,[20,0,0,80,0,0,0,0])],0,200).valid,true);
 assert.equal(resourceWindow([row(0,[0,0,0,0,0,0,0,0]),row(100,[41,0,0,59,0,0,0,0])],0,200).valid,false);
});
test('MP-11 reason deltas isolate workload windows and restarted workers',()=>{
 const row=(at,worker,count)=>({stage:'native_rejected',ended_ms:at,worker,reason:3,count});
 assert.equal(rejections([row(1,2,5),row(5,2,7),row(6,3,2),row(20,2,10)],4,10).serial_skipped,4);
});

test('MP-10 typing endpoints are included in the host resource window',async()=>{
 const {mkdtemp,writeFile,rm}=await import('node:fs/promises'),{tmpdir}=await import('node:os');
 const root=await mkdtemp(tmpdir()+'/vidlat-hops-');
 try{
  await writeFile(root+'/report.json',JSON.stringify({cells:[{dpr:1,typing:[{source_equals_public_c:true,physical:{input_at:4000,presented_at:6000,latency_ms:2000}}]}]}));
  await writeFile(root+'/resources.jsonl',[{at_ms:3000,cpu:[0,0,0,0,0,0,0,0]},{at_ms:5000,cpu:[80,0,0,20,0,0,0,0]}].map(JSON.stringify).join('\n'));
  await writeFile(root+'/manifest.json',JSON.stringify({runs:[{path:'V',identity:{oss:'source',cloud:'cloud',binary_sha256:'binary'},report:root+'/report.json',resources:root+'/resources.jsonl'}]}));
  const result=await run(root+'/manifest.json',root+'/out');
  assert.equal(result.V[0].resources.start_ms,4000);assert.equal(result.V[0].resources.end_ms,6000);assert.equal(result.V[0].resources.valid,false);
 }finally{await rm(root,{recursive:true,force:true});}
});
