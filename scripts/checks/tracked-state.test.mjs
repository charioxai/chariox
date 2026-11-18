import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './tracked-state.mjs'

const planted = (r, label) => {
  const result = r.run(check.name)
  assert.equal(result.status, 1, label)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`), `${label}: names the root fix`)
  return result.text
}

test('source files and small images pass', () => {
  const r = makeRepo({ 'apps/x/a.ts': 'x\n', 'docs/small.png': Buffer.alloc(1000).toString('latin1') })
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a tracked log is red', () => {
  const r = makeRepo({ 'apps/x/a.ts': 'x\n' })
  r.write('apps/x/kernel.log', 'boot\n')
  assert.match(planted(r, 'log'), /apps\/x\/kernel\.log:0: tracked log, recording or database file/)
  r.dispose()
})

test('tracked evidence is red', () => {
  const r = makeRepo({ 'apps/x/a.ts': 'x\n' })
  r.write('docs/evidence/run1/summary.json', '{}\n')
  assert.match(planted(r, 'evidence'), /docs\/evidence\/run1\/summary\.json:0: tracked state directory content \(evidence\)/)
  r.dispose()
})

test('a screenshot over 200 KB is red', () => {
  const r = makeRepo({ 'apps/x/a.ts': 'x\n' })
  r.write('docs/shot.png', 'x'.repeat(210 * 1024))
  assert.match(planted(r, 'png'), /docs\/shot\.png:0: tracked image over 200 KB \(210 KB\)/)
  r.dispose()
})
