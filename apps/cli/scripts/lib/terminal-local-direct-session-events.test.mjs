// MP-10/MP-11: supplementary checks for the real multi-client log seam.
import assert from 'node:assert/strict'
import test from 'node:test'
import { terminalSessionEvents } from './terminal-local-direct-session-events.mjs'

const admission = (subject, at) => JSON.stringify({ message: 'local browser session admitted', subject, timestamp_ms: at })
const close = (subject, at) => JSON.stringify({ message: 'local browser session closed', subject, timestamp_ms: at, reason: 'local browser connection closed' })
const window = { startedAtMs: 100, finishedAtMs: 200 }

test('MP-10 browser reload does not fail a different paired TUI; its own closure does', () => {
  const lines = [admission('cli-measured', 99), close('browser-reloaded', 120), close('cli-measured', 201)]
  const result = terminalSessionEvents(lines, 'cli-measured', window)
  assert.equal(result.admissions.length, 1)
  assert.deepEqual(result.closures, [])
  assert.equal(result.otherClientClosures.length, 1)
  assert.equal(result.otherClientClosures[0].subjectSha256.length, 64)
  assert.equal(terminalSessionEvents([...lines, close('cli-measured', 200)], 'cli-measured', window).closures.length, 1)
})

test('MP-10 absent serving admission and unidentified in-window closures fail closed', () => {
  assert.throws(() => terminalSessionEvents([admission('other', 99)], 'cli-measured', window), /no kernel admission/)
  assert.throws(() => terminalSessionEvents([admission('cli-measured', 99), close(undefined, 120)], 'cli-measured', window), /identity missing/)
  assert.throws(() => terminalSessionEvents([], '', window), /subject required/)
})
