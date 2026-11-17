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

test('catch {} in a comment or in string data is not an empty catch', () => {
  const r = makeRepo({ 'apps/web/a.ts': 'export const x = 1\n' })
  r.write('apps/web/a.ts', [
    '// Avoid catch {} because it hides errors',
    '/* try { f() } catch {} */',
    'export const example = "catch {}"',
    "export const other = 'try { f() } catch (e) {}'",
    'export const block = `',
    'try { f() } catch {}',
    '`',
    'export const expression = `${1}` + /catch {}/.source',
    '',
  ].join('\n'))
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a real empty catch after a comment and a string is still red, on its own line', () => {
  const r = makeRepo({ 'apps/web/a.ts': 'export const x = 1\n' })
  r.write('apps/web/a.ts', '// catch {}\nexport const s = "catch {}"\nexport function f() {\n  try { g() } catch {}\n}\n')
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /a\.ts:4: has an empty catch block: catch \{\}/)
  r.dispose()
})

test('an empty catch inside a String.raw program is still red: that text is code for another runtime', () => {
  const r = makeRepo({ 'apps/web/a.ts': 'export const x = 1\n' })
  r.write('apps/web/a.ts', 'export const script = String.raw`\ntry { run() } catch {}\n`\nexport const note = `\ntry { run() } catch {}\n`\n')
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /a\.ts:2: has an empty catch block/)
  assert.doesNotMatch(result.text, /a\.ts:5/)
  r.dispose()
})

test('comments and string data inside a String.raw program are not code, its real code still is', () => {
  const r = makeRepo({ 'apps/web/a.ts': 'export const x = 1\n' })
  r.write('apps/web/a.ts', [
    'export const script = String.raw`',
    '// Avoid catch {} because it hides errors',
    '/* try { f() } catch {} */',
    "const sample = 'try { f() } catch (e) {}'",
    'const text = "catch {}"',
    'const pattern = /catch {}/',
    'try { run() } catch {}',
    '`',
    '',
  ].join('\n'))
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /a\.ts:7: has an empty catch block/)
  assert.equal(result.text.match(/has an empty catch block/g).length, 1)
  r.dispose()
})

test('a String.raw program with only comments and data about catch blocks passes', () => {
  const r = makeRepo({ 'apps/web/a.ts': 'export const x = 1\n' })
  r.write('apps/web/a.ts', 'export const script = String.raw`\n// catch {} is banned\nconst s = "catch {}"\ntry { run() } catch (e) { report(e) }\n`\n')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a regex literal after the closing parenthesis of if, while, for or with is a regex, so its quote cannot hide an empty catch', () => {
  for (const head of ['if (ready)', 'while (more())', 'for (const part of parts)', 'with (scope)']) {
    const r = makeRepo({ 'apps/web/a.ts': 'export const x = 1\n' })
    r.write('apps/web/a.ts', `export function f() {\n  ${head} /'/.test(text); try { g() } catch {}\n}\n`)
    const result = r.run(check.name)
    assert.equal(result.status, 1, head)
    assert.match(result.text, /a\.ts:2: has an empty catch block/, head)
    r.dispose()
  }
})

test('a slash after a closing parenthesis that is not a condition stays a division', () => {
  const r = makeRepo({ 'apps/web/a.ts': 'export const x = 1\n' })
  r.write('apps/web/a.ts', "export const mean = (a + b) / 2 + '/'; export const last = f(a) / 2 + '/'\nexport function g() {\n  try { h() } catch {}\n}\n")
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /a\.ts:3: has an empty catch block/)
  r.dispose()
})
