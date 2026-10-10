// MP-08/MP-10/MP-11: bounded timing records must survive batching/shutdown.
import test from 'node:test';import assert from 'node:assert/strict';
import {mkdtempSync,readFileSync,existsSync,rmSync} from 'node:fs';import {tmpdir} from 'node:os';import path from 'node:path';
import {displayTiming} from './kernel-browser-timing.mjs';
test('MP-08/MP-10 native diagnostics batch without losing order or final shutdown records',()=>{
 const root=mkdtempSync(path.join(tmpdir(),'chariox-mp24-timing-')),old=process.env.CHARIOX_BROWSER_DISPLAY_TIMING;
 const timing=displayTiming(root),file=path.join(root,'display-timing.jsonl');
 try{
  delete process.env.CHARIOX_BROWSER_DISPLAY_TIMING;timing('disabled',0,1);assert(!existsSync(file));
  process.env.CHARIOX_BROWSER_DISPLAY_TIMING='1';timing('capture',1,2);timing.batch([['convert',2,3],['encode',3,5]]);assert(!existsSync(file));
  for(let i=0;i<600;i++)timing('control',i,i+1);
  assert(existsSync(file),'bounded accumulation flushes before growing');timing.flush();
  const records=readFileSync(file,'utf8').trim().split('\n').map(JSON.parse);
  assert.equal(records.length,603);assert.deepEqual(records.slice(0,3).map(r=>[r.stage,r.duration_ms]),[['capture',1],['convert',1],['encode',2]]);
  assert.deepEqual(records.slice(3).map(r=>r.started_ms),Array.from({length:600},(_,i)=>i));
 }finally{timing.flush();if(old===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_TIMING;else process.env.CHARIOX_BROWSER_DISPLAY_TIMING=old;rmSync(root,{recursive:true,force:true})}
});
test('MP-08/MP-10 input/frame join records keep finite numbers only',()=>{
 const root=mkdtempSync(path.join(tmpdir(),'chariox-mp24-timing-')),old=process.env.CHARIOX_BROWSER_DISPLAY_TIMING;
 const timing=displayTiming(root),file=path.join(root,'display-timing.jsonl');
 try{
  delete process.env.CHARIOX_BROWSER_DISPLAY_TIMING;timing.event('frame_out',{at:1,sequence:1});timing.flush();assert(!existsSync(file));
  process.env.CHARIOX_BROWSER_DISPLAY_TIMING='1';
  timing.event('frame_out',{at:5,sequence:7,kind:1,published_ms:6,encoded_ms:undefined,text:'page data'});timing.flush();
  const [record]=readFileSync(file,'utf8').trim().split('\n').map(JSON.parse);
  assert.equal(record.stage,'frame_out');assert.equal(record.started_ms,5);assert.equal(record.sequence,7);assert.equal(record.published_ms,6);
  assert(!('text' in record)&&!('encoded_ms' in record)&&!('at' in record));
 }finally{timing.flush();if(old===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_TIMING;else process.env.CHARIOX_BROWSER_DISPLAY_TIMING=old;rmSync(root,{recursive:true,force:true})}
});
