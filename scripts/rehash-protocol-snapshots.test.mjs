import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { test } from 'node:test'
import { hashModules, isEntryPoint, mismatches, modulePath, replaceLiteral } from './rehash-protocol-snapshots.mjs'

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

test('the entry point is recognised when the checkout path has a space', () => {
  const script = '/home/user/My Projects/chariox/scripts/rehash-protocol-snapshots.mjs'
  assert.equal(isEntryPoint(pathToFileURL(script).href, script), true)
  assert.equal(isEntryPoint(pathToFileURL(script).href, '/home/user/other.mjs'), false)
  assert.equal(isEntryPoint(pathToFileURL(script).href, undefined), false)
})

test('the command runs cargo in a checkout whose path has a space, and records the pins there', () => {
  const base = mkdtempSync(join(tmpdir(), 'rehash '))
  try {
    const checkout = join(base, 'My Projects', 'chariox')
    const here = dirname(new URL(import.meta.url).pathname.replaceAll('%20', ' '))
    for (const path of ['scripts/rehash-protocol-snapshots.mjs', 'scripts/lib/protocol-hash-pins.mjs', 'scripts/lib/protocol-versions.mjs']) {
      mkdirSync(dirname(join(checkout, path)), { recursive: true })
      cpSync(join(here, '..', path), join(checkout, path))
    }
    mkdirSync(join(checkout, 'apps/kernel/src/local/api'), { recursive: true })
    mkdirSync(join(checkout, 'apps/kernel/src/transport'), { recursive: true })
    writeFileSync(join(checkout, 'apps/kernel/src/local/api/types.rs'), 'pub const LOCAL_DAEMON_PROTOCOL_VERSION: u32 = 7;\n')
    writeFileSync(join(checkout, 'apps/kernel/src/transport/relay_peer.rs'), 'pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 3;\n')
    mkdirSync(join(checkout, 'fixtures'))
    writeFileSync(join(checkout, 'fixtures/protocol-hash-pins.json'), JSON.stringify({ local: 7, relayPeer: 3 }))
    const bin = join(base, 'bin')
    mkdirSync(bin)
    writeFileSync(join(bin, 'cargo'), `#!/bin/sh\npwd > "${base}/cargo-cwd"\nexit 0\n`)
    chmodSync(join(bin, 'cargo'), 0o755)
    const run = spawnSync('node', [join(checkout, 'scripts/rehash-protocol-snapshots.mjs')], { cwd: base, encoding: 'utf8', env: { ...process.env, PATH: `${bin}:${process.env.PATH}` } })
    assert.equal(run.status, 0, run.stderr)
    assert.match(run.stdout, /protocol tests pass for 7\/3/)
    assert.ok(existsSync(join(base, 'cargo-cwd')), 'cargo was never run: the entry point was not recognised or the working directory does not exist')
    assert.equal(readFileSync(join(base, 'cargo-cwd'), 'utf8').trim(), checkout)
    assert.deepEqual(JSON.parse(readFileSync(join(checkout, 'fixtures/protocol-hash-pins.json'), 'utf8')), { local: 7, relayPeer: 3 })
  } finally { rmSync(base, { recursive: true, force: true }) }
})
