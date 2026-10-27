// MP-08/MP-10: repeated takeover must verify one new physical click.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm,readFile} from 'node:fs/promises';
import {runWebTakeoverProof} from './wp07p-web-takeover-proof.mjs';
test('MP-08/MP-10 return takeover increments the existing counter and keeps its own receipt',async()=>{
 const dir=await mkdtemp('/root/.chariox/dev/browser-resume-20260930/agents/providerse/takeover-test-');let clicked=false;
 const action={action_id:'a3',sequence:3,actor_id:'user:local',mode:'computer',runtime_generation:1,cancellation_requested:false,kind:'pointer_click',state:'completed',targets:[{kind:'desktop'}],submitted_at_ms:1,started_at_ms:1,finished_at_ms:2,outcome:{status:'completed'}};
 const client={send:async req=>req.history?{RoomEnvironmentActionHistoryListed:{page:{actions:clicked?[action]:[]}}}:{RoomEnvironmentState:{environment:{actors:[{actor_id:'user:local',kind:'human'}]}}}};
 try{
  await runWebTakeoverProof({client,requests:{listRoomEnvironmentActionHistoryRequest:()=>({history:true}),getRoomEnvironmentStateRequest:()=>({})},sessionId:'s',proofLabel:'return',web:{observe:async()=>{clicked=true;return {ok:true}}},waitFor:async fn=>{const r=await fn();assert.ok(r);return r},waitForBrowserText:async needle=>{const text='POINTER_CLICK_COUNT='+ (clicked?3:2);assert.ok(text.includes(needle),'expected physical counter '+text+', got '+needle);return text},waitForLocalNotice:async()=>{},waitForRemoteNotice:async()=>{},publicEvidenceRoot:dir});
  const r=JSON.parse(await readFile(dir+'/web-return-takeover-proof.json'));assert.equal(r.status,'PASS');assert.equal(r.beforeClickCount,2);assert.equal(r.expectedClickCount,3);
 }finally{await rm(dir,{recursive:true,force:true});}
});
