// MP-08 / MP-11: exercise the shipped capture helper without starting a drill.
import assert from 'node:assert/strict'
import test from 'node:test'
import vm from 'node:vm'
import path from 'node:path'
import {readFile} from 'node:fs/promises'
const source = await readFile(new URL('./live-byom-account-drill.mjs', import.meta.url), 'utf8')
const shotSource = source.match(/^const shot = .*$/m)[0]
test('MP-08 / MP-11 every diagnostic screenshot masks capability-bearing controls', async () => {
  const capabilityControls = {selector: 'controls'}, calls = []
  const page = {locator: () => capabilityControls, screenshot: async options => {calls.push(options)}}
  const shot = vm.runInNewContext(shotSource + '; shot', {path, E: '/evidence', report: {}, safe: String})
  await shot(page, 'failure-0')
  assert.deepEqual(Array.from(calls[0].mask ?? []), [capabilityControls])
  assert.match(source, /catch \(error\) \{[^]*await shot\(open, `failure-\$\{index\}`\)/)
})
