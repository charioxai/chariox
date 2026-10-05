// MP-08/MP-10: frozen TimeWarp solver executes on the explicit Room slice worker.
import test from 'node:test'
import assert from 'node:assert/strict'
import { Round2Room } from './round2/room.mjs'
test('MP-08/MP-10 shared Room spawn forwards explicit worker slice and HIGH effort',async()=>{
  let request
  const api={requests:{spawnAgentRequest:(...args)=>({spawn:args}),attachToSessionRequest:()=>({attach:true})},client:{send:async value=>{
    if(value.spawn){request=value;return {AgentSpawned:{agent:{id:'agent'}}}}
    return {SessionAttached:{attachment:{id:'attachment'}}}
  }}}
  const room=new Round2Room(api,{workspace:'/workspace',runId:'r2next-timewarp-test'});room.owned.sessionId='session'
  await room.spawn({provider:'codex',model:'gpt-6.1-sol',effort:'high',sliceRef:'slice-worker',accountProfile:'profile'})
  assert.equal(request.spawn[10],'slice-worker');assert.equal(request.spawn[5],'high')
})
