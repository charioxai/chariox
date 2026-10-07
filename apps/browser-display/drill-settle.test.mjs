import test from 'node:test';import assert from 'node:assert/strict';
import {verifyContinuousSettled,drainRepairs,verifySettled,waitQuiet} from './drill-settle.mjs';
test('MD-DISPLAY late native paint gets bounded verification; persistent mismatch stays RED',async()=>{
 let attempts=0;
 const result=await verifySettled(async()=>null,async()=>({lossless:++attempts===2}));
 assert.equal(result.verification_attempts,2);
 await assert.rejects(verifySettled(async()=>null,async()=>({lossless:false}),2),/bounded verification/);
});
test('MD-DISPLAY repair validation waits for unchanged, never assumes first batch exact',async()=>{
 const frames=[{kind:'tiles',sequence:2},{kind:'tiles',sequence:3},null,null];
 const result=await drainRepairs(async()=>frames.shift(),300,async()=>{});assert.equal(result.polls,4);assert.equal(result.last.sequence,3);
 await assert.rejects(drainRepairs(async()=>({kind:'tiles'}),3),/did not converge/);
 await assert.rejects(drainRepairs(async()=>{throw Error('capture failed')}),/capture failed/);
});

test('MD-DISPLAY settling crosses idle verification deadline before accepting unchanged',async()=>{
 const frames=[null,{kind:'tiles',sequence:4},null,null],waits=[];
 const result=await drainRepairs(async()=>frames.shift(),10,async ms=>waits.push(ms));
 assert.deepEqual(waits,[300,300]);assert.equal(result.polls,4);assert.equal(result.last.sequence,4);
});

test('MD-DISPLAY continuous settle needs exact RGB plus a bound presentation, without manual credits',async()=>{
 let time=0,calls=0;const r=await verifyContinuousSettled(async()=>{calls++;return {lossless:calls>=2,presentation_ms:calls===3?42:undefined}},{now:()=>time,wait:async ms=>time+=ms});assert.equal(calls,3);assert.equal(r.fidelity.presentation_ms,42);assert.equal(r.polls,0);
});
test('MD-DISPLAY continuous settle bounds a non-converging display',async()=>{let time=0;await assert.rejects(verifyContinuousSettled(async()=>({lossless:false}),{timeoutMs:250,now:()=>time,wait:async ms=>time+=ms}),/did not converge/)});
test('MP-08 settle verification waits for a quiet lossless presentation before any readback',async()=>{
 let time=0;const log=[{count:5,kind:'video'},{count:6,kind:'video'},{count:6,kind:'video'},{count:7,kind:'tiles'}];
 const latest=async()=>log[Math.min(log.length-1,Math.floor(time/100))];
 await waitQuiet(latest,{now:()=>time,wait:async ms=>time+=ms});
 assert.ok(time>=300+600,'quiet only after the last lossless presentation');
 time=0;await waitQuiet(async()=>({count:1,kind:'video'}),{timeoutMs:500,now:()=>time,wait:async ms=>time+=ms});assert.ok(time>=500,'bounded');
});
