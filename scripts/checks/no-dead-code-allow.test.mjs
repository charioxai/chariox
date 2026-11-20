import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './no-dead-code-allow.mjs'

const FILE = 'apps/kernel/src/runtime/worker.rs'
const clean = 'fn used() {}\n'
const listed = count => `${JSON.stringify([{ path: FILE, count, why: 'reached only on one platform' }])}\n`
const planted = (r, expected) => {
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, expected)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`), 'names the root fix')
  r.dispose()
}

test('a tree without lint allowances passes', () => {
  const r = makeRepo({ [FILE]: `#[allow(clippy::too_many_arguments)]\n${clean}` })
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a new allow(dead_code) is red', () => {
  const r = makeRepo({ [FILE]: clean })
  r.write(FILE, `#[allow(dead_code)]\nfn unused() {}\n${clean}`)
  planted(r, /worker\.rs:1: allows dead_code instead of deleting the unused code/)
})

test('the module-wide, mixed, cfg_attr and unused forms are red', () => {
  for (const [attribute, lint] of [
    ['#![allow(dead_code, reason = "later")]', 'dead_code'],
    ['#[allow(clippy::too_many_arguments, dead_code)]', 'dead_code'],
    ['#[cfg_attr(not(target_os = "linux"), allow(unused_mut, unused_variables))]', 'unused_variables'],
    ['#[allow(unused_imports)]', 'unused_imports'],
  ]) {
    const r = makeRepo({ [FILE]: clean })
    r.write(FILE, `${attribute}\nfn unused() {}\n`)
    planted(r, new RegExp(`worker\\.rs:1: allows ${lint} `))
  }
})

test('a listed allowance passes, a second one in the same file is red', () => {
  const r = makeRepo({ [FILE]: '#[allow(dead_code)]\nfn a() {}\n', 'scripts/checks/no-dead-code-allow.allow.json': listed(1) })
  assert.equal(r.run(check.name).status, 0)
  r.write(FILE, '#[allow(dead_code)]\nfn a() {}\n#[allow(dead_code)]\nfn b() {}\n')
  planted(r, /worker\.rs:\d: allows dead_code/)
})

test('an allowance in test code, a comment or a string passes', () => {
  const r = makeRepo({ [FILE]: clean })
  r.write(FILE, '// #[allow(dead_code)]\nconst S: &str = "#[allow(dead_code)]";\n#[cfg(test)]\nmod tests {\n    #[allow(dead_code)]\n    fn helper() {}\n}\n')
  r.write('apps/kernel/src/runtime/tests/support.rs', '#[allow(dead_code)]\nfn helper() {}\n')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})
