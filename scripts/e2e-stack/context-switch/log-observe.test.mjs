// MP-08 / MP-10 / MP-11: supplementary warning diagnostics retain timing, discard private fields.
import test from 'node:test';import assert from 'node:assert/strict';
import {mkdtemp,mkdir,writeFile,rm} from 'node:fs/promises';import {tmpdir} from 'node:os';import {join} from 'node:path';import {spawnSync} from 'node:child_process';
test('compaction failure timing survives nested details without private values',async()=>{
 const root=await mkdtemp(join(tmpdir(),'context-log-'));try{
  await mkdir(root+'/home/logs',{recursive:true});await writeFile(root+'/context-switch-owned.json','{}');
  const privateMarker='synthetic-value-must-not-appear';await writeFile(root+'/home/logs/test-daemon-one.ndjson',JSON.stringify({timestamp_ms:42,component:'daemon.provider_context_handoff',message:'failed to compact the session before a smaller context window',fields:{error:'deadline '+privateMarker,details:{context_tokens:160000,elapsed_ms:300000,private_note:privateMarker}}})+'\n');
  const output=spawnSync('python3',[new URL('./log-observe.py',import.meta.url).pathname,root],{encoding:'utf8'});assert.equal(output.status,0,output.stderr);assert(!output.stdout.includes(privateMarker));const row=JSON.parse(output.stdout).rows[0];assert.equal(row.errorClass,'timeout');assert.equal(row.fields.elapsed_ms,300000);assert.equal(row.fields.context_tokens,160000);
 }finally{await rm(root,{recursive:true,force:true})}
});
