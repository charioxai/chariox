import test from 'node:test';import assert from 'node:assert/strict';
import {drainRepairs} from './drill-settle.mjs';
test('MD-DISPLAY repair validation waits for unchanged, never assumes first batch exact',async()=>{
 const frames=[{kind:'tiles',sequence:2},{kind:'tiles',sequence:3},null];
 const result=await drainRepairs(async()=>frames.shift());assert.equal(result.polls,3);assert.equal(result.last.sequence,3);
 await assert.rejects(drainRepairs(async()=>({kind:'tiles'}),3),/did not converge/);
 await assert.rejects(drainRepairs(async()=>{throw Error('capture failed')}),/capture failed/);
});
