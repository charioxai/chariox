// MP-08 / MP-10: timing evidence must belong to the controlled provider turn.
import assert from 'node:assert/strict'
import test from 'node:test'
import { interruptTiming } from './workflow-interrupt-evidence.mjs'

test('MP-08 timing excludes old controls and foreign provider completions', () => {
  const sent = (at, providerRunId, turnId) => ({at,providerRunId,turnId,message:'codex turn interrupt sent trace'})
  const ended = (at, providerRunId, turnId) => ({at,providerRunId,turnId,message:'codex turn completion received trace'})
  const result = interruptTiming([
    sent(90,'run','old'), ended(95,'run','old'), sent(110,'run','active'),
    ended(115,'other-run','active'), ended(120,'run','old'), ended(140,'run','active'),
  ], 100)
  assert.equal(result.stopToInterruptSentMs,10)
  assert.equal(result.stopToTurnEndedMs,40)
  assert.equal(result.ended.turnId,'active')
})

test('MP-10 no send or no matching completion remains missing evidence', () => {
  assert.equal(interruptTiming([],100).sent,undefined)
  const result=interruptTiming([{at:110,providerRunId:'run',turnId:'active',message:'codex turn interrupt sent trace'}],100)
  assert.equal(result.stopToInterruptSentMs,10)
  assert.equal(result.ended,undefined)
  assert.equal(result.stopToTurnEndedMs,undefined)
})

test('MP-08 stale ID retry measures first send and final interrupted identity', () => {
  const result=interruptTiming([
    {at:110,providerRunId:'run',turnId:'submitted',message:'codex turn interrupt sent trace'},
    {at:115,providerRunId:'run',turnId:'submitted',message:'codex turn completion received trace'},
    {at:120,providerRunId:'run',turnId:'actual',message:'codex turn interrupt sent trace'},
    {at:125,providerRunId:'foreign',turnId:'another',message:'codex turn interrupt sent trace'},
    {at:140,providerRunId:'run',turnId:'actual',message:'codex turn completion received trace'},
  ],100)
  assert.equal(result.stopToInterruptSentMs,10)
  assert.equal(result.stopToTurnEndedMs,40)
  assert.equal(result.lastSent.turnId,'actual')
})
