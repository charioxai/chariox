// MP-08 / MP-10: regressions for duplicate prompts after races or ambiguous RPC failures.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtemp, rm, readFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { submitOnce } from './webvoyager-admission.mjs'
async function fixture(fn) {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'benchwv-admission-'))
  try { await fn(directory) } finally { await rm(directory, { recursive: true }) }
}
test('MP-08 / MP-10 concurrent runners send exactly one prompt', () => fixture(async directory => {
  let sends = 0
  const run = runId => submitOnce({ directory, scope: 'full', taskId: 'Amazon--0', runId,
    submit: async () => { sends++; await new Promise(r => setTimeout(r, 20)); return 'ok' } })
  const results = await Promise.allSettled([run('one'), run('two')])
  assert.equal(sends, 1)
  assert.equal(results.filter(r => r.status === 'fulfilled').length, 1)
}))
test('MP-08 / MP-10 lost RPC acknowledgement never resends admitted prompt', () => fixture(async directory => {
  let sends = 0
  const options = { directory, scope: 'full', taskId: 'Amazon--0', runId: 'one',
    submit: async () => { sends++; throw Error('response lost after provider started') } }
  await assert.rejects(submitOnce(options))
  await assert.rejects(submitOnce({ ...options, runId: 'two' }))
  assert.equal(sends, 1)
  const receipt = JSON.parse(await readFile(`${directory}/full-Amazon--0.json`))
  assert.equal(receipt.state, 'reserved')
}))
test('MP-08 / MP-10 completed receipt remains immutable and smoke/full scopes differ', () => fixture(async directory => {
  const options = { directory, scope: 'smoke', taskId: 'Amazon--0', runId: 'one', submit: async () => ({ promptId: 'p1' }) }
  await submitOnce(options)
  const before = await readFile(`${directory}/smoke-Amazon--0.json`, 'utf8')
  await assert.rejects(submitOnce({ ...options, runId: 'two' }))
  assert.equal(await readFile(`${directory}/smoke-Amazon--0.json`, 'utf8'), before)
  await submitOnce({ ...options, scope: 'full' })
}))
