// MP-08 / MP-10 / MP-11: delayed cancelled answers must fail the real drill oracle.
import test from 'node:test';
import assert from 'node:assert/strict';
import {verifyCancelledPromptRecovery} from './cancellation-validation.mjs';
const agentId='cancelled-agent',marker='cancelled-prompt-marker';
function scenario(phase){
 let rows=[],reads=0,recovered=false;
 const snapshot=async()=>{reads++;return {agentPanes:{[agentId]:[...rows]}};};
 const answer=()=>rows.push({role:'assistant',text:'Late reply '+marker});
 const input={agentId,marker,settle:snapshot,snapshot,wait:async ms=>{assert.equal(ms,8000);if(phase==='window')answer();},checkFold:async()=>{},recover:async()=>{recovered=true;if(phase==='recovery')answer();}};
 return {input,stats:()=>({reads,recovered})};
}
for(const phase of ['window','recovery'])test('MP-08 / MP-10 / MP-11 rejects cancelled answer arriving during '+phase,async()=>{
 const run=scenario(phase);
 await assert.rejects(verifyCancelledPromptRecovery(run.input),/Cancelled prompt received a provider answer/);
 assert.equal(run.stats().recovered,phase==='recovery');
});
test('MP-08 / MP-10 / MP-11 checks fresh state after both observation and recovery',async()=>{
 const run=scenario('none');
 assert.deepEqual(await verifyCancelledPromptRecovery(run.input),{cancelledAnswerAbsent:true,cancelledFoldAbsent:true,recoveryRecall:9});
 assert.equal(run.stats().reads,3);
});
