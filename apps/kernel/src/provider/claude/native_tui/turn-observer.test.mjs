import assert from 'node:assert/strict'
import test from 'node:test'
import {register} from './turn-observer.mjs'

test('MP-08 MP-10 aborted first response carries the exact prompt and hook fence', async () => {
  const handlers = new Map(), writes = []
  register((event, handler) => handlers.set(event, handler), {context: '/owned/context'})
  const $ = {fs: {
    read: async path => path.endsWith('.approval-origin') ? 'prompt-123' : JSON.stringify({id:'hook-123'}),
    write: async (path, text) => writes.push({path, value:JSON.parse(text)}),
  }}
  const next = async e => e
  assert(handlers.has('turn.start'), 'official turn must bind before completion')
  await handlers.get('turn.start')($, {turnId:'turn-123'}, next)
  await handlers.get('turn.complete')($, {turnId:'turn-123',reason:'aborted',isAborted:true}, next)
  assert.equal(writes.length, 1)
  assert.equal(writes[0].path, '/owned/context.native-abort.json')
  assert.equal(writes[0].value.prompt_id, 'prompt-123')
  assert.equal(writes[0].value.hook_context_request_id, 'hook-123')
  assert.equal(writes[0].value.reason, 'aborted')
  assert(Number.isSafeInteger(writes[0].value.at_ms))
})

test('MP-11 other turns, subagents and answers cannot acknowledge cancellation', async () => {
  const handlers = new Map(), writes = []
  register((event, handler) => handlers.set(event, handler), {context:'/owned/context'})
  const $ = {fs:{read:async path => path.endsWith('.approval-origin')?'prompt-123':JSON.stringify({id:'hook-123'}),write:async (...args)=>writes.push(args)}}
  const next=async e=>e
  assert(handlers.has('turn.start'))
  await handlers.get('turn.start')($,{turnId:'turn-123'},next)
  for(const e of [{turnId:'foreign',reason:'aborted',isAborted:true},{turnId:'turn-123',reason:'answer',isAborted:false},{turnId:'turn-123',agentId:'child',reason:'aborted',isAborted:true}])
    await handlers.get('turn.complete')($,e,next)
  assert.equal(writes.length,0)
})
