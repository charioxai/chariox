// MP-08/MP-10: exact native fragment helpers; inline, fragmented and failed history.
import test from 'node:test'
import assert from 'node:assert/strict'
import { auditTimeWarpHistory } from './timewarp-history.mjs'
const helpers = await import(process.env.CHARIOX_FRAGMENT_HELPERS_MODULE)
const event = (index, kind, text) => ({ entry_index: index, fragment_start: 0,
  fragment_end: Array.from(text).length, total_chars: Array.from(text).length,
  entry: { kind, text, provider_run_id: 'r', merge_key: null } })
const tool = event(1, 'provider_tool', JSON.stringify({id:'call', tool:'slice_browser_text',status:'completed'}))
const answer = event(2, 'provider_output', 'answer 😀')
const turn = { entries:[tool, answer], blobs:[], lifecycle:'completed', summary:answer }
test('MP-08/MP-10 inline native tools count without a history blob', () => {
  const audit = auditTimeWarpHistory(turn, [tool, answer], helpers)
  assert.equal(audit.tools.length, 1); assert.equal(audit.tools[0].tool, 'slice_browser_text')
  assert.equal(audit.answer, 'answer 😀')
})
test('MP-08/MP-10 fragmented original tools assemble once; missing pieces remain RED', () => {
  const text=Array.from(tool.entry.text), split=20
  const part=(start,end)=>({...tool,fragment_start:start,fragment_end:end,entry:{...tool.entry,text:text.slice(start,end).join('')}})
  assert.equal(auditTimeWarpHistory(turn,[part(0,split),part(split,text.length),part(0,split),answer],helpers).tools.length,1)
  assert.throws(()=>auditTimeWarpHistory(turn,[part(0,split),answer],helpers))
})
test('MP-08/MP-10 inline provider errors cannot become a valid completed turn', () => {
  assert.equal(auditTimeWarpHistory(turn,[tool,answer,event(3,'provider_error','provider failed')],helpers).providerError,true)
})
