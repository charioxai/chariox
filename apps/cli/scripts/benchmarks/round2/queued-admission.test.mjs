// MP-08/MP-10/MP-11: native queue promotion must not replay or misidentify a turn.
import test from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { Round2Room } from './room.mjs'
import { getTurn, waitForSettlement } from './kernel.mjs'
import { settlementRecord } from './export.mjs'
import { submitOnce } from '../webvoyager-admission.mjs'
import { mkdtemp, rm } from 'node:fs/promises'
const helpers=await import(process.env.CHARIOX_FRAGMENT_HELPERS_MODULE)
const prompt='Repeated task 😀'
const fragment=(text,start=0,total=Array.from(text).length)=>({entry_index:7,fragment_start:start,fragment_end:start+Array.from(text).length,total_chars:total,entry:{kind:'user_prompt',text}})
const native=(id,lifecycle='completed',text=prompt)=>({prompt_id:id,turn_id:`turn-${id}`,lifecycle,user_prompt:fragment(text),entries:[],blobs:[]})
function fixture({outcome='Queued',lifecycle='completed',ambiguous=false,fragmented=false}={}) {
 let submitted=false, sends=0
 const old=native('old'),older=native('older'),promoted=native(outcome==='Queued'?'promoted':'accepted',lifecycle)
 const calls=[]
 if(fragmented){const chars=Array.from(prompt);promoted.user_prompt=fragment(chars.slice(0,8).join(''),0,chars.length);promoted.blobs=[{blob_id:'original-prompt',kind:'user_prompt'}]}
 const requests=new Proxy({}, {get:(_,method)=>(...args)=>({method,args})})
 const api={helpers,requests,client:{send:async request=>{
  calls.push(request)
  if(request.method==='submitPromptRequest'){submitted=true;sends++;return {PromptSubmitted:{outcome:{[outcome]:{prompt:{id:'accepted'}}}}}}
  if(request.method==='getSessionHistoryOutlineRequest'){
   assert.deepEqual(request.args.slice(0,2),['session',['agent']])
   const baseline=request.args[3]?{turns:[older]}:{turns:[old],next_cursor:{before_sequence:5}}
   const current={turns:[old,promoted,...(ambiguous?[native('another')]:[])]}
   return {SessionHistoryOutline:{agents:[{agent_id:'foreign',turns:[native('foreign')]},{agent_id:'agent',...(submitted?current:baseline)}]}}
  }
  if(request.method==='getSessionHistoryBlobContentRequest')return {SessionHistoryBlobContent:{blob_id:'original-prompt',entries:[fragment(prompt)]}}
  throw Error('unexpected native fixture request')
 }}}
 const room=new Round2Room(api,{workspace:'/fixture',runId:'queued-fixture'});Object.assign(room.owned,{sessionId:'session',agentId:'agent',attachmentId:'attachment'})
 return {room,api,calls,promoted,sends:()=>sends}
}
for(const lifecycle of ['open','completed','failed','cancelled'])test(`MP-08/MP-10/MP-11 queued admission resolves native ${lifecycle} without resubmit`,async()=>{
 const f=fixture({lifecycle});const identity=await f.room.submit(prompt)
 assert.equal(identity.promptId,'accepted');assert.equal(identity.admissionOutcome,'Queued')
 assert.deepEqual(identity.baselinePromptIds,['old','older'])
 assert.equal(identity.submittedPromptSha256,createHash('sha256').update(prompt).digest('hex'))
 assert.equal(await getTurn(f.api,identity),f.promoted);assert.equal(identity.promotedPromptId,'promoted')
 assert.equal(identity.promptId,'accepted');assert.equal(f.sends(),1)
 const receipt=settlementRecord({...identity,turn:f.promoted,elapsedMs:0})
 assert.equal(receipt.promptId,'promoted');assert.equal(receipt.lifecycle,lifecycle)
 assert.equal(receipt.status,lifecycle==='completed'?'settled':'RED')
})
test('MP-08/MP-10/MP-11 complete fragmented Unicode user prompt resolves promotion',async()=>{
 const f=fixture({fragmented:true}),identity=await f.room.submit(prompt)
 assert.equal((await getTurn(f.api,identity)).prompt_id,'promoted');assert.equal(f.sends(),1)
})
test('MP-08/MP-10/MP-11 multiple new identical queued turns fail closed',async()=>{
 const f=fixture({ambiguous:true}),identity=await f.room.submit(prompt)
 await assert.rejects(getTurn(f.api,identity),/ambiguous/);assert.equal(f.sends(),1)
})
test('MP-08/MP-10/MP-11 queued completion settles before cancellation budgets',async()=>{
 const f=fixture(),identity=await f.room.submit(prompt)
 let clock=0,cancels=0
 const result=await waitForSettlement(f.api,identity,{maxMs:1000,now:()=>clock,pause:async ms=>{clock+=ms},cancel:async()=>{cancels++}})
 assert.equal(result.turn.prompt_id,'promoted');assert.equal(result.cancellation,null);assert.equal(cancels,0);assert.equal(f.sends(),1)
})
test('MP-08/MP-10/MP-11 Started outcome retains exact native admission ID',async()=>{
 const f=fixture({outcome:'Started'}),identity=await f.room.submit(prompt)
 assert.equal(identity.admissionOutcome,'Started');assert.equal((await getTurn(f.api,identity)).prompt_id,'accepted');assert.equal(f.sends(),1)
})
test('MP-08/MP-10/MP-11 queued identity never binds a different prompt or changes its promoted ID',async()=>{
 const f=fixture(),identity=await f.room.submit(prompt)
 f.promoted.user_prompt=fragment('Other task');assert.equal(await getTurn(f.api,identity),null)
 f.promoted.user_prompt=fragment(prompt);await getTurn(f.api,identity)
 f.promoted.prompt_id='replacement';await assert.rejects(getTurn(f.api,identity),/ambiguous/)
 assert.equal(f.sends(),1)
})
test('MP-08/MP-10/MP-11 WebVoyager durable reservation wraps queued admission exactly once',async()=>{
 const directory=await mkdtemp('/root/.chariox/dev/browser-resume-20260930/agents/r2next/round3/queued-test-')
 try {
  const f=fixture(),submit=request=>submitOnce({directory,scope:'fixture',taskId:'one',runId:'one',submit:()=>f.api.client.send(request)})
  const identity=await f.room.submit(prompt,{submit})
  assert.equal((await getTurn(f.api,identity)).prompt_id,'promoted')
  await assert.rejects(f.room.submit(prompt,{submit}),{code:'EEXIST'});assert.equal(f.sends(),1)
 }finally{await rm(directory,{recursive:true})}
})
