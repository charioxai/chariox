import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./live-relay-freeform-multi-user-drill.mjs', import.meta.url))

test('multi-user drill observes the full prompt completion from full and transparent clients', async () => {
  const source = await readFile(script, 'utf8')
  const fullPromptIndex = source.indexOf('const fullPrompt = unwrap(')
  const user3BaselineIndex = source.indexOf('const user3FullCompletionBaseline')
  const user4BaselineIndex = source.indexOf('const user4FullCompletionBaseline')

  assert.match(source, /events4 = await subscribeForCompletions\(user4, session\.id, attachment4\.id\)/)
  assert.match(source, /waitForCompletion\(user3, session\.id, attachment3\.id, events3, user3OwnerPromptCompletionBaseline/)
  assert.match(source, /waitForCompletion\(user4, session\.id, attachment4\.id, events4, user4OwnerPromptCompletionBaseline/)
  assert.ok(user3BaselineIndex >= 0 && user3BaselineIndex < fullPromptIndex)
  assert.ok(user4BaselineIndex >= 0 && user4BaselineIndex < fullPromptIndex)
  assert.match(source, /waitForCompletion\(user3, session\.id, attachment3\.id, events3, user3FullCompletionBaseline/)
  assert.match(source, /waitForCompletion\(user4, session\.id, attachment4\.id, events4, user4FullCompletionBaseline/)
  assert.match(source, /fullPromptCompletionEvidence\.user3\.delta === 1/)
  assert.match(source, /fullPromptCompletionEvidence\.user4\.delta === 1/)
})

test('multi-user drill reports transparent completion evidence without weakening owner counts', async () => {
  const source = await readFile(script, 'utf8')

  assert.match(source, /user1CompletionCount === 2/)
  assert.match(source, /user2CompletionCount === 1/)
  assert.match(source, /fullPromptCompletionEvidence,/)
  assert.match(source, /user4: user4CompletionCount/)
  assert.match(source, /user4: eventCounts\(events4\)/)
  assert.match(source, /full and transparent collaborators each observe exactly one completion/)
})
