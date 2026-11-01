// MP-08 / MP-10 / MP-11: stale source answers must never pass a target recall.
import test from 'node:test'
import assert from 'node:assert/strict'
import { ContextValidationTurnCompletion, scoreContextValidationRecall } from './context-switch-validation-records.mjs'
const expected = { decision: 'retain native session', proposal: 'inspect cancellation' }
const stale = [
  { role: 'user', text: 'source workload' },
  { role: 'assistant', text: JSON.stringify(expected) },
  { role: 'user', text: 'recall CURRENT' },
  { role: 'assistant', text: 'I cannot recall it. CURRENT' },
]
test('excludes the source answer and answers after the next prompt', () => {
  const rows = [...stale, { role: 'user', text: 'later task' }, { role: 'assistant', text: JSON.stringify(expected) }]
  assert.equal(scoreContextValidationRecall(rows, 'CURRENT', expected).score, 0)
})
test('reports exact current recall and actual tool use', () => {
  const rows = [...stale.slice(0, 3), { role: 'tool', text: 'read source' }, { role: 'assistant', text: JSON.stringify(expected) }]
  const result = scoreContextValidationRecall(rows, 'CURRENT', expected)
  assert.equal(result.score, 2)
  assert.equal(result.toolCount, 1)
  assert.equal(result.userCount, 1)
})
test('rejects duplicate or absent probe submissions', () => {
  assert.equal(scoreContextValidationRecall([...stale, stale[2]], 'CURRENT', expected).userCount, 2)
  assert.equal(scoreContextValidationRecall(stale, 'MISSING', expected).score, 0)
})
test('MP-08 MP-10 MP-11 unknown records do not hide correctly recalled values', () => {
  const rows = [...stale.slice(0, 3), { role: 'assistant', text: JSON.stringify({ decision: expected.decision, proposal: null }) }]
  const result = scoreContextValidationRecall(rows, 'CURRENT', expected)
  assert.equal(result.score, 1)
  assert.equal(result.total, 2)
  assert.equal(result.checks.proposal, false)
})

test('MP-08 / MP-10 / MP-11 a completion marker cannot settle text while late chunks still change it',()=>{
 const completion=new ContextValidationTurnCompletion();
 assert.equal(completion.observe({busy:true,answer:'partial',now:0}),false);
 assert.equal(completion.observe({busy:false,answer:'spliced text END',now:100}),false);
 assert.equal(completion.observe({busy:false,answer:'complete text END',now:1000}),false);
 assert.equal(completion.observe({busy:false,answer:'complete text END',now:5999}),false);
 assert.equal(completion.observe({busy:false,answer:'complete text END',now:6000}),true);
 assert.equal(completion.sawWorking,true);
 completion.reset();
 assert.equal(completion.observe({busy:false,answer:'complete text END',now:7000}),false);
 assert.equal(completion.observe({busy:true,answer:'complete text END',now:12000}),false);
 assert.equal(completion.observe({busy:false,answer:'complete text END',now:13000}),false);
 assert.equal(completion.observe({busy:false,answer:'complete text END',now:18000}),true);
});
