import assert from 'node:assert/strict'
import { test } from 'node:test'
import { hashModules, mismatches, modulePath, replaceLiteral } from './rehash-protocol-snapshots.mjs'

const a = 'a'.repeat(64)
const b = 'b'.repeat(64)
const panic = (file, left, right) => `thread 't' (1) panicked at ${file}:12:5:\nassertion \`left == right\` failed\n  left: "${left}"\n right: "${right}"\n`

test('a failed hash assertion yields the file, the computed hash and the pinned one', () => {
  const found = mismatches(panic('apps/kernel/src/x.rs', a, b) + 'noise\n' + panic('apps/kernel/src/y.rs', b, a))
  assert.deepEqual(found.map(({ file, left, right }) => [file, left, right]), [['apps/kernel/src/x.rs', a, b], ['apps/kernel/src/y.rs', b, a]])
})

test('an assertion that is not two hashes is not a mismatch', () => {
  assert.deepEqual(mismatches('panicked at x.rs:1:1:\nassertion `left == right` failed\n  left: Number(1)\n right: Number(2)\n'), [])
})

test('the pinned literal is replaced once, and only when it is one literal', () => {
  assert.equal(replaceLiteral(`x("${b}")`, a, b), `x("${a}")`)
  assert.equal(replaceLiteral(`x("${b}") y("${b}")`, a, b), null)
  assert.equal(replaceLiteral('x()', a, b), null)
})

test('a source file becomes the module path the test filter needs', () => {
  assert.equal(modulePath('apps/kernel/src/runtime/router/tests/m23_metaagent_runtime_tools/cross_room_cards.rs'), 'runtime::router::tests::m23_metaagent_runtime_tools::cross_room_cards')
  assert.equal(modulePath('apps/kernel/src/local/api/tests.rs'), 'local::api::tests')
  assert.equal(modulePath('apps/kernel/src/runtime/state/mod.rs'), 'runtime::state')
})

test('the modules to run include the hash tests no name filter would find', () => {
  const modules = hashModules()
  assert.ok(modules.includes('runtime::router::tests::m23_metaagent_runtime_tools::cross_room_cards'))
  assert.ok(modules.includes('local::api::tests::protocol_shapes::workflow_gate'))
})
