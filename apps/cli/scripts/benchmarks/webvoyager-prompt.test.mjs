// MP-08/MP-10: instructions remain byte-exact to the round1 frozen runner.
import test from 'node:test'
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { webVoyagerPrompt } from './webvoyager-task.mjs'

test('all frozen WebVoyager prompts preserve round1 instructions and public task text', { skip: !process.env.WEBVOYAGER_FROZEN_SOURCE || !process.env.WEBVOYAGER_FROZEN_SELECTION }, async () => {
  const source = await readFile(process.env.WEBVOYAGER_FROZEN_SOURCE, 'utf8')
  const literal = source.match(/const prompt = (`[^`]+`)/)?.[1]
  assert(literal, 'MP-10 frozen prompt boundary missing')
  const frozenPrompt = new Function('task', `return ${literal}`)
  const selection = JSON.parse(await readFile(process.env.WEBVOYAGER_FROZEN_SELECTION, 'utf8'))
  assert.equal(selection.tasks.length, 643)
  for (const task of selection.tasks) assert.equal(webVoyagerPrompt(task), frozenPrompt(task), task.id)
})
