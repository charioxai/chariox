import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './retired-names.mjs'

const retired = ['waiting', 'room'].join(' ')
const listed = count => `${JSON.stringify([{ path: 'apps/a.ts', count, why: 'before the rename' }])}\n`
const planted = (r, label) => {
  const result = r.run(check.name)
  assert.equal(result.status, 1, label)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`), `${label}: names the root fix`)
  return result.text
}

test('a file without a retired name passes', () => {
  const r = makeRepo({ 'apps/a.ts': 'const inbox = 1\n' })
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a new match in a file outside the list is red, in any spelling', () => {
  for (const spelling of [retired, 'WaitingRoom', retired.toUpperCase().replace(' ', '_')]) {
    const r = makeRepo({ 'apps/a.ts': 'const inbox = 1\n' })
    r.write('apps/a.ts', `const inbox = 1\n// ${spelling}\n`)
    assert.match(planted(r, spelling), /apps\/a\.ts:2: uses the retired name/)
    r.dispose()
  }
})

test('a retired file name is red', () => {
  const r = makeRepo({ 'apps/a.ts': 'x\n' })
  r.write('apps/waiting-room-view.ts', 'x\n')
  assert.match(planted(r, 'path'), /apps\/waiting-room-view\.ts:0: the file name uses the retired name/)
  r.dispose()
})

test('a listed file may keep its count but not grow it', () => {
  const r = makeRepo({ 'apps/a.ts': `// ${retired}\n`, 'scripts/checks/retired-names.allow.json': listed(1) })
  assert.equal(r.run(check.name).status, 0)
  r.write('apps/a.ts', `// ${retired}\n// ${retired}\n`)
  assert.match(planted(r, 'grown'), /apps\/a\.ts:\d: uses the retired name/)
  r.dispose()
})

test('a listed file that dropped a match must lower its count', () => {
  const r = makeRepo({ 'apps/a.ts': `// ${retired}\n// ${retired}\n`, 'scripts/checks/retired-names.allow.json': listed(2) })
  r.write('apps/a.ts', `// ${retired}\n`)
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /apps\/a\.ts:0: 1 left, the list says 2: lower it/)
  r.dispose()
})
