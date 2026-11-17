import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './no-empty-catch.mjs'

const FILE = 'apps/cli/src/cache.ts'
const handled = 'export function load() {\n  try {\n    return read()\n  } catch (error) {\n    log(error)\n    return null\n  }\n}\n'
const listed = count => `${JSON.stringify([{ path: FILE, count, why: 'best-effort cleanup' }])}\n`
const planted = (r, expected) => {
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, expected)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`), 'names the root fix')
  r.dispose()
}

test('a catch that handles the error passes', () => {
  const r = makeRepo({ [FILE]: handled })
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('an empty catch is red in each spelling', () => {
  for (const block of ['catch {}', 'catch (error) {}', 'catch (error: unknown) {\n  }', 'catch{ }']) {
    const r = makeRepo({ [FILE]: handled })
    r.write(FILE, `export function close() {\n  try {\n    stop()\n  } ${block}\n}\n`)
    planted(r, /cache\.ts:\d: has an empty catch block/)
  }
})

test('a catch with a comment that says why passes', () => {
  const r = makeRepo({ [FILE]: handled })
  r.write(FILE, 'export function close() {\n  try {\n    stop()\n  } catch {\n    // the socket is already closed\n  }\n}\n')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a listed empty catch passes, a second one in the same file is red', () => {
  const body = 'export function close() {\n  try { stop() } catch {}\n}\n'
  const r = makeRepo({ [FILE]: body, 'scripts/checks/no-empty-catch.allow.json': listed(1) })
  assert.equal(r.run(check.name).status, 0)
  r.write(FILE, `${body}export function open() {\n  try { start() } catch {}\n}\n`)
  planted(r, /cache\.ts:\d: has an empty catch block/)
})

test('test files and scripts outside apps and packages are not scanned', () => {
  const r = makeRepo({ [FILE]: handled })
  r.write('apps/cli/src/cache.test.ts', 'try { x() } catch {}\n')
  r.write('scripts/tool.ts', 'try { x() } catch {}\n')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})
