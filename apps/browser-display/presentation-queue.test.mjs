// MP-08/MP-10/MP-11: repair/recovery barriers cannot be passed by native video.
import test from 'node:test';import assert from 'node:assert/strict';import {PresentationQueue} from './presentation-queue.mjs';
const tick=()=>new Promise(r=>setImmediate(r));
test('MP-08/MP-10 only three same-document videos prepare ahead, with an exact-repair barrier',async()=>{
 const prepared=[],shown=[];let finish;
 const q=new PresentationQueue({prepareVideo:f=>prepared.push(f.sequence)});
 const consume=async f=>{shown.push(f.sequence);if(f.sequence===1)await new Promise(r=>finish=r)};
 const frames=[1,2,3,4,5,6].map(sequence=>({sequence,kind:sequence===5?'tiles':'video',document_id:'d',width:1280,height:800,codec:'vp8'}));
 const jobs=frames.map(f=>q.submit(f,consume));await tick();assert.deepEqual(prepared,[1,2,3]);finish();await Promise.all(jobs);await q.tail;
 assert.deepEqual(prepared,[1,2,3,4,6]);assert.deepEqual(shown,[1,2,3,4,5,6]);
});
test('MP-11 recovery and document changes fence off decode-ahead until ordered consumption',async()=>{
 const prepared=[];let recovering=true;
 const q=new PresentationQueue({prepareVideo:f=>prepared.push(f.sequence)},frame=>!recovering||frame.key);
 const consume=async f=>{if(f.key)recovering=false};
 const frames=[{sequence:1,kind:'video',key:false,document_id:'d'},{sequence:2,kind:'video',key:true,document_id:'new'},{sequence:3,kind:'video',key:false,document_id:'new'}];
 const jobs=frames.map(f=>q.submit(f,consume));assert.deepEqual(prepared,[]);await Promise.all(jobs);await q.tail;assert.deepEqual(prepared,[2,3]);
});
