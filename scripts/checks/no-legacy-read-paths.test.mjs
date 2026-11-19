import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './no-legacy-read-paths.mjs'

const KIND = 'apps/kernel/src/session/kind.rs'
const aliased = 'pub enum Origin {\n    #[serde(alias = "arroba")]\n    Chariox,\n}\n'
const plain = 'pub enum Origin {\n    Chariox,\n}\n'
const listed = (path, count) => `${JSON.stringify([{ path, count, why: 'reads the name from before the rename' }])}\n`
const planted = (r, expected) => {
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, expected)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`), 'names the root fix')
  r.dispose()
}

test('a tree without aliases or legacy declarations passes', () => {
  const r = makeRepo({ [KIND]: plain, 'packages/x/src/a.ts': 'export const current = 1\n' })
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a new serde alias in a file outside the list is red', () => {
  const r = makeRepo({ [KIND]: plain })
  r.write(KIND, aliased)
  planted(r, /session\/kind\.rs:2: keeps a serde alias that accepts an old name/)
})

test('a multi-line serde attribute with an alias is red', () => {
  const r = makeRepo({ [KIND]: plain })
  r.write(KIND, 'pub struct S {\n    #[serde(\n        default,\n        alias = "workflow_watchdogs",\n    )]\n    schedules: Vec<u8>,\n}\n')
  planted(r, /session\/kind\.rs:4: keeps a serde alias/)
})

test('a listed alias passes, a second alias in the same file is red', () => {
  const r = makeRepo({ [KIND]: aliased, 'scripts/checks/no-legacy-read-paths.allow.json': listed(KIND, 1) })
  assert.equal(r.run(check.name).status, 0)
  r.write(KIND, `${aliased}pub enum Second {\n    #[serde(alias = "old")]\n    New,\n}\n`)
  planted(r, /session\/kind\.rs:\d: keeps a serde alias/)
})

test('an entry added to the list in this change is red', () => {
  const r = makeRepo({ [KIND]: plain, 'scripts/checks/no-legacy-read-paths.allow.json': listed('apps/kernel/src/other.rs', 1) })
  r.write(KIND, aliased)
  r.write('scripts/checks/no-legacy-read-paths.allow.json', `${JSON.stringify([{ path: KIND, count: 1, why: 'reads the name from before the rename' }])}\n`)
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /session\/kind\.rs is not in the list at the merge base/)
  r.dispose()
})

test('a legacy-named declaration is red, in Rust and in top-level TypeScript', () => {
  const r = makeRepo({ [KIND]: plain })
  r.write(KIND, 'fn legacy_history_cutoff() -> u64 {\n    0\n}\n')
  planted(r, /session\/kind\.rs:1: declares legacy_history_cutoff, a legacy path/)
  const t = makeRepo({ 'packages/x/src/a.ts': 'export const current = 1\n' })
  t.write('packages/x/src/a.ts', 'export function legacyPromptFor(id: string) {\n  return id\n}\n')
  planted(t, /a\.ts:1: declares legacyPromptFor, a legacy path/)
})

test('a local variable called legacy, a test file and a comment pass', () => {
  const r = makeRepo({ 'packages/x/src/a.ts': 'export const current = 1\n' })
  r.write('packages/x/src/a.ts', 'export function read(text: string) {\n  const legacy = text.match(/x/)\n  return legacy\n}\n')
  r.write('packages/x/src/a.test.ts', 'function legacyCase() {}\n')
  r.write(KIND, '// #[serde(alias = "old")] was removed\n#[cfg(test)]\nmod tests {\n    #[serde(alias = "x")]\n    fn legacy_case() {}\n}\n')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('several aliases in one attribute count one each, so adding one to a listed attribute is red', () => {
  const r = makeRepo({ [KIND]: aliased, 'scripts/checks/no-legacy-read-paths.allow.json': listed(KIND, 1) })
  assert.equal(r.run(check.name).status, 0)
  r.write(KIND, 'pub enum Origin {\n    #[serde(alias = "arroba", alias = "at")]\n    Chariox,\n}\n')
  planted(r, /session\/kind\.rs:2: keeps a serde alias/)
})

test('an alias written in a multi-line attribute with other options still counts once', () => {
  const r = makeRepo({ [KIND]: aliased, 'scripts/checks/no-legacy-read-paths.allow.json': listed(KIND, 1) })
  r.write(KIND, 'pub enum Origin {\n    #[serde(\n        rename = "chariox",\n        alias = "arroba",\n    )]\n    Chariox,\n}\n')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a legacy declaration inside a template literal or a comment is data, not a declaration', () => {
  const r = makeRepo({ 'packages/x/src/a.ts': 'export const current = 1\n' })
  r.write('packages/x/src/a.ts', [
    'export const example = `',
    'function legacyExample() {}',
    'export const legacyOther = 1',
    '`',
    '/*',
    'export function legacyInComment() {}',
    '*/',
    "export const text = 'a'",
    '',
  ].join('\n'))
  assert.equal(r.run(check.name).status, 0)
  r.write('packages/x/src/a.ts', 'export const example = `${1}`\nexport function legacyReal() {}\n')
  planted(r, /a\.ts:2: declares legacyReal, a legacy path/)
})
