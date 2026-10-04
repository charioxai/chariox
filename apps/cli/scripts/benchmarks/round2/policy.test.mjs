// MP-08 / MP-10 / MP-11 H3/H4. First-party synthetic inputs, no benchmark gold.
import assert from 'node:assert/strict'
import test from 'node:test'
import * as shared from './index.mjs'

test('MP-08 / MP-10 / MP-11 H3 consent overlay permits only observed rejection', () => {
  const overlay = { visible: true, nonessential: null, clicks: [] }
  const click = action => {
    if (!shared.consentDecision(action).allowed) return
    overlay.clicks.push(action)
    overlay.nonessential = action !== 'reject_nonessential'
    overlay.visible = false
  }
  for (const action of ['accept_all', 'unknown', 'dismiss', 'authorization', 'preferences']) click(action)
  assert.deepEqual(overlay, { visible: true, nonessential: null, clicks: [] })
  click('reject_nonessential')
  assert.deepEqual(overlay, { visible: false, nonessential: false, clicks: ['reject_nonessential'] })
  assert.equal(shared.consentDecision('accept_all').reason, 'nonessential_consent_forbidden')
})

test('MP-08 / MP-10 / MP-11 H3/H4 prompt declares consent and UTC admission date', () => {
  const { prompt, policy } = shared.prepareRound2Prompt({ prompt: 'Find a current public name.', startedAt: '2026-10-03T23:59:59.000Z' })
  assert.match(prompt, /Always reject non-essential consent/)
  assert.match(prompt, /2026-10-03 \(UTC\)/)
  assert.match(prompt, /historical gold/i)
  assert.match(prompt, /skip that source/i)
  assert.equal(policy.runDate, '2026-10-03')
  assert.equal(policy.consent, 'reject_nonessential')
  assert.equal(policy.questionMode, 'live')
  assert.equal(policy.scoring, 'official_verdict_unchanged')
})

test('MP-08 / MP-10 / MP-11 H4 fixed questions retain their task date', () => {
  const { prompt, policy } = shared.prepareRound2Prompt({ prompt: 'Who held office in 2001?', startedAt: '2026-10-03T12:00:00Z', questionMode: 'fixed' })
  assert.equal(policy.runDate, '2026-10-03')
  assert.match(prompt, /Use the date specified in the task/)
  assert.doesNotMatch(prompt, /Answer live questions as of/)
  assert.throws(() => shared.prepareRound2Prompt({ prompt: 'fixture', startedAt: 'invalid' }), /date/)
  assert.throws(() => shared.prepareRound2Prompt({ prompt: 'fixture', startedAt: '2026-10-03T12:00:00Z', questionMode: 'gold' }), /question mode/)
})

test('MP-08 / MP-10 / MP-11 H4 name and set packaging preserve Unicode and membership', () => {
  assert.equal(shared.packageRound2Answer('  "Éva Fixture"\n', { kind: 'name' }), 'Éva Fixture')
  assert.deepEqual(shared.packageRound2Answer('["Éva Fixture", "李 Fixture"]', { kind: 'set' }), ['Éva Fixture', '李 Fixture'])
  assert.throws(() => shared.packageRound2Answer('The names are Éva and 李.', { kind: 'set' }), /answer shape/)
  assert.throws(() => shared.packageRound2Answer('["A", "A"]', { kind: 'set' }), /answer shape/)
  assert.throws(() => shared.packageRound2Answer('["A", 2]', { kind: 'set' }), /answer shape/)
})

test('MP-08 / MP-10 / MP-11 H4 number packaging keeps factual errors for the scorer', () => {
  // A wrong count remains exactly that count: packaging never consults gold.
  assert.equal(shared.packageRound2Answer('  17\n', { kind: 'number' }), '17')
  assert.equal(shared.packageRound2Answer('1.25e2', { kind: 'number' }), '1.25e2')
  assert.equal(shared.packageRound2Answer('""', { kind: 'number' }), '')
  for (const text of ['17 people', 'NaN', '1e999', '"17"', '9007199254740993']) {
    assert.throws(() => shared.packageRound2Answer(text, { kind: 'number' }), /answer shape/)
  }
})

test('MP-08 / MP-10 / MP-11 H4 records preserve wrong classifications and declared keys', () => {
  const contract = { kind: 'records', fields: ['name', 'category'] }
  assert.deepEqual(shared.packageRound2Answer('[{"name":"Fixture","category":"incorrect"}]', contract), [{ name: 'Fixture', category: 'incorrect' }])
  for (const text of ['[{"name":"Fixture"}]', '[{"name":"Fixture","category":"x","extra":1}]', '[null]']) {
    assert.throws(() => shared.packageRound2Answer(text, contract), /answer shape/)
  }
})

test('MP-08 / MP-10 / MP-11 H4 contract is explicit; free text remains byte-exact', () => {
  assert.equal(shared.packageRound2Answer('Résumé 👋.\n', { kind: 'text' }), 'Résumé 👋.\n')
  const { prompt } = shared.prepareRound2Prompt({ prompt: 'fixture', startedAt: '2026-10-03T12:00:00Z', answerContract: { kind: 'set' } })
  assert.match(prompt, /JSON array of unique strings/)
  assert.match(prompt, /No prose, citations, Markdown fences/)
  assert.throws(() => shared.packageRound2Answer('fixture', { kind: 'guess' }), /answer contract/)
  assert.throws(() => shared.prepareRound2Prompt({ prompt: 'fixture', startedAt: '2026-10-03T12:00:00Z', answerContract: { kind: 'records' } }), /answer contract/)
})

test('MP-08 / MP-10 / MP-11 H4 envelope verifies task identity without gold', () => {
  const contract = { kind: 'set', taskId: 'synthetic-1' }
  assert.deepEqual(shared.packageRound2Answer('{"id":"synthetic-1","answer":["Fixture"]}', contract), ['Fixture'])
  for (const text of ['{"id":"other","answer":["Fixture"]}', '{"id":"synthetic-1","answer":["Fixture"],"commentary":"x"}', '```json\n{"id":"synthetic-1","answer":["Fixture"]}\n```']) {
    assert.throws(() => shared.packageRound2Answer(text, contract), /answer shape/)
  }
})
