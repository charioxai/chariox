import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { test } from 'node:test'
import { makeRepo } from './lib/check-fixture.mjs'
import { hashModules, isEntryPoint, mismatches, modulePath, replaceLiteral } from './rehash-protocol-snapshots.mjs'

const a = 'a'.repeat(64)
const b = 'b'.repeat(64)
const panic = (file, left, right) => `thread 't' (1) panicked at ${file}:12:5:\nassertion \`left == right\` failed\n  left: "${left}"\n right: "${right}"\n`

test('a failed hash assertion yields the file, the computed hash and the pinned one', () => {
  const found = mismatches(panic('apps/kernel/src/x.rs', a, b) + 'noise\n' + panic('apps/kernel/src/y.rs', b, a))
  assert.deepEqual(found.map(({ file, left, right }) => [file, left, right]), [['apps/kernel/src/x.rs', a, b], ['apps/kernel/src/y.rs', b, a]])
})

test('a failed hash assertion without the old "assertion failed" header is a mismatch too', () => {
  const found = mismatches(`thread 't' (1) panicked at apps/kernel/src/z.rs:12:5:\n  left: "${a}"\n right: "${b}"\n`)
  assert.deepEqual(found.map(({ file, left, right }) => [file, left, right]), [['apps/kernel/src/z.rs', a, b]])
})

test('a wrapped hash failure keeps the registered libtest name', () => {
  const output = '---- local::api::wrapped_protocol stdout ----\n' + panic('apps/kernel/src/local/api.rs', a, b)
  assert.equal(mismatches(output)[0].test, 'local::api::wrapped_protocol')
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
    for (const path of ['scripts/rehash-protocol-snapshots.mjs', 'scripts/lib/protocol-hash-pins.mjs', 'scripts/lib/protocol-versions.mjs', 'scripts/lib/rust-production-source.mjs', 'scripts/lib/rust-scan.mjs']) {
      mkdirSync(dirname(join(checkout, path)), { recursive: true })
      cpSync(join(here, '..', path), join(checkout, path))
    }
    mkdirSync(join(checkout, 'apps/kernel/src/local/api'), { recursive: true })
    mkdirSync(join(checkout, 'apps/kernel/src/transport'), { recursive: true })
    writeFileSync(join(checkout, 'apps/kernel/src/local/api/types.rs'), 'pub const LOCAL_DAEMON_PROTOCOL_VERSION: u32 = 7;\n')
    writeFileSync(join(checkout, 'apps/kernel/src/transport/relay_peer.rs'), 'pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 3;\n')
    mkdirSync(join(checkout, 'fixtures'))
    writeFileSync(join(checkout, 'fixtures/protocol-hash-pins.json'), JSON.stringify({ local: 7, relayPeer: 3 }))
    writeFileSync(join(checkout, 'apps/kernel/src/fixture.rs'), '"'+b+'";\n')
    assert.equal(spawnSync('git', ['init', '-q'], { cwd: checkout }).status, 0)
    assert.equal(spawnSync('git', ['add', '-A'], { cwd: checkout }).status, 0)
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

// Exercise the canonical entry point with fixture Cargo output, never a compiler.
function guardedFixture(t, { pinsText = '{"local":7,"relayPeer":3}', local = 8, scenario = 'success', sameFile = false } = {}) {
  const protocol = 'apps/kernel/src/local/api/tests/protocol_shapes/row403.rs'
  const unrelated = sameFile ? protocol : 'apps/kernel/src/runtime/row403_vault.rs'
  const secretPin = 'd'.repeat(64), secretComputed = 'c'.repeat(64)
  const protocolBody = `${scenario === 'wrapped' ? '' : '#[test]'}\nfn protocol_hash() {\n let wire = serde_json::json!({"local": LOCAL_DAEMON_PROTOCOL_VERSION});\n assert_eq!(format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())), "${b}");\n}\n`
  const unrelatedBody = `#[test]\nfn vault_hash() {\n assert_eq!(format!("{:x}", Sha256::digest(b"unrelated")), "${secretPin}");\n}\n`
  const sources = {
    'apps/kernel/src/local/api/types.rs': `pub const LOCAL_DAEMON_PROTOCOL_VERSION: u32 = ${local};\n`,
    'apps/kernel/src/transport/relay_peer.rs': 'pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 3;\n',
    [protocol]: protocolBody + (scenario === 'wrapped' ? '#[test]\nfn wrapped_protocol() { protocol_hash(); }\n' : '') + (sameFile ? unrelatedBody : ''),
    ...(!sameFile ? { [unrelated]: unrelatedBody } : {}),
  }
  if (pinsText !== null) sources['fixtures/protocol-hash-pins.json'] = pinsText
  const here = dirname(new URL(import.meta.url).pathname.replaceAll('%20', ' '))
  for (const path of ['scripts/rehash-protocol-snapshots.mjs', 'scripts/lib/protocol-hash-pins.mjs', 'scripts/lib/protocol-versions.mjs', 'scripts/lib/rust-production-source.mjs', 'scripts/lib/rust-scan.mjs']) {
    sources[path] = readFileSync(join(here, '..', path), 'utf8')
  }
  const r = makeRepo(sources)
  t.after(() => r.dispose())
  chmodSync(r.root, 0o755)
  mkdirSync(join(r.root, 'run-evidence'))
  chmodSync(join(r.root, 'run-evidence'), 0o777)
  const bin = join(r.root, 'bin')
  mkdirSync(bin)
  writeFileSync(join(bin, 'cargo'), `#!${process.execPath}
const fs=require('node:fs');
fs.writeFileSync('run-evidence/cargo-called','called');
const protocol=${JSON.stringify(protocol)},unrelated=${JSON.stringify(unrelated)},scenario=${JSON.stringify(scenario)};
if(scenario==='success')process.exit(0);
const pending=fs.readFileSync(protocol,'utf8').includes(${JSON.stringify(`"${b}"`)});
const other=fs.readFileSync(unrelated,'utf8').includes(${JSON.stringify(`"${secretPin}"`)});
const failures=[];let output='';
const nl=${JSON.stringify('\n')};
const moduleOf=file=>file.slice('apps/kernel/src/'.length,-3).split('/').join('::');
const mismatch=(file,name,left,right)=>{output+=["thread '"+moduleOf(file)+'::'+name+"' (1) panicked at "+file+':4:2:','  left: "'+left+'"',' right: "'+right+'"',''].join(nl);failures.push(moduleOf(file)+'::'+name)};
if(pending&&scenario==='wrapped')output+='---- '+moduleOf(protocol)+'::wrapped_protocol stdout ----'+nl;
if(pending)mismatch(protocol,scenario==='wrapped'?'wrapped_protocol':'protocol_hash',${JSON.stringify(a)},${JSON.stringify(b)});
if((scenario==='mixed-hash'||scenario==='same-file')&&other)mismatch(unrelated,'vault_hash',${JSON.stringify(secretComputed)},${JSON.stringify(secretPin)});
if(scenario==='mixed-nonhash'){output+='an ordinary unrelated assertion failed'+nl;failures.push(moduleOf(unrelated)+'::ordinary_failure')}
if(!failures.length)process.exit(0);
process.stdout.write(output+nl+'failures:'+nl+failures.map(name=>'    '+name).join(nl)+nl);process.exit(101);
`)
  chmodSync(join(bin, 'cargo'), 0o755)
  const syntax = spawnSync(process.execPath, ['--check', join(bin, 'cargo')], { encoding: 'utf8' })
  assert.equal(syntax.status, 0, syntax.stderr)
  const pins = join(r.root, 'fixtures/protocol-hash-pins.json')
  const paths = [...new Set(['fixtures/protocol-hash-pins.json', protocol, unrelated])]
  const snapshot = () => Object.fromEntries(paths.map(path => [path, existsSync(join(r.root, path)) ? readFileSync(join(r.root, path), 'utf8') : null]))
  return {
    root: r.root, pins, protocol, unrelated, snapshot,
    run: ({ readGuard = false, ...options } = {}) => spawnSync(process.execPath, [...(readGuard ? [
      '--permission', '--allow-child-process',
      `--allow-fs-read=${join(r.root, 'scripts')}`,
      `--allow-fs-read=${join(r.root, 'apps/kernel/src/local/api/types.rs')}`,
      `--allow-fs-read=${join(r.root, 'apps/kernel/src/transport/relay_peer.rs')}`,
    ] : []), join(r.root, 'scripts/rehash-protocol-snapshots.mjs')], {
      cwd: r.root, encoding: 'utf8', env: { ...process.env, PATH: `${bin}:${process.env.PATH}` }, ...options,
    }),
    called: () => existsSync(join(r.root, 'run-evidence/cargo-called')),
  }
}

for (const [name, pinsText] of [
  ['missing', null], ['malformed', '{'], ['null', 'null'], ['array', '[]'],
  ['missing peer', '{"local":7}'], ['wrong type', '{"local":"7","relayPeer":3}'],
  ['invalid version', '{"local":0,"relayPeer":3}'],
]) test(`${name} pins fail before Cargo or any file changes`, t => {
  const f = guardedFixture(t, { pinsText })
  const before = f.snapshot(), result = f.run()
  assert.ifError(result.error)
  assert.equal(result.status, 1, result.stderr)
  assert.equal(f.called(), false, 'metadata must be validated before tests can authorize a rewrite')
  assert.deepEqual(f.snapshot(), before)
})

test('unreadable pins fail before Cargo or any file changes', t => {
  const f = guardedFixture(t)
  const before = f.snapshot()
  const result = f.run({ readGuard: true })
  assert.ifError(result.error)
  assert.equal(result.status, 1, result.stderr)
  assert.match(result.stderr, /fixtures\/protocol-hash-pins\.json/)
  assert.equal(f.called(), false)
  assert.deepEqual(f.snapshot(), before)
})

test('a matching version never rewrites failed expectations', t => {
  const f = guardedFixture(t, { local: 7, scenario: 'protocol' })
  const before = f.snapshot(), result = f.run()
  assert.equal(result.status, 1)
  assert.deepEqual(f.snapshot(), before)
})

test('a matching successful version preserves the pins bytes too', t => {
  const f = guardedFixture(t, { local: 7, pinsText: '{"relayPeer":3,"local":7}\n' })
  const before = f.snapshot(), result = f.run()
  assert.equal(result.status, 0, result.stderr)
  assert.deepEqual(f.snapshot(), before)
})

test('a valid intended bump changes only its protocol expectation and pins', t => {
  const f = guardedFixture(t, { scenario: 'protocol' })
  const before = f.snapshot(), result = f.run(), after = f.snapshot()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(after[f.protocol], before[f.protocol].replace(`"${b}"`, `"${a}"`))
  assert.equal(after[f.unrelated], before[f.unrelated])
  assert.deepEqual(JSON.parse(after['fixtures/protocol-hash-pins.json']), { local: 8, relayPeer: 3 })
})

test('a valid bump rewrites a protocol helper reported through its registered wrapper', t => {
  const f = guardedFixture(t, { scenario: 'wrapped' })
  const before = f.snapshot(), result = f.run(), after = f.snapshot()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(after[f.protocol], before[f.protocol].replace(`"${b}"`, `"${a}"`))
  assert.equal(after[f.unrelated], before[f.unrelated])
})

for (const scenario of ['mixed-hash', 'same-file', 'mixed-nonhash']) test(`${scenario} failures stay failures without changing expectations`, t => {
  const f = guardedFixture(t, { scenario, sameFile: scenario === 'same-file' })
  const before = f.snapshot(), result = f.run()
  assert.equal(result.status, 1, result.stderr)
  assert.deepEqual(f.snapshot(), before)
})
