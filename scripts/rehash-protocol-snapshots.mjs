// pnpm protocol:rehash: after a protocol bump (pnpm protocol:generate), rewrite every pinned snapshot hash the bump changed.
// It runs the tests of every kernel module that pins a SHA-256 literal, replaces each hash literal that no longer matches with the computed one, repeats until
// they pass, then records the protocol pair in fixtures/protocol-hash-pins.json. It refuses to bless a mismatch at an unchanged
// protocol pair: that is a shape change, and a shape change needs the bump first (AGENTS.md "Protocol changes").
// Plain cargo runs underneath, so wrap the whole command like any other cargo job: cargo-slot.sh pnpm protocol:rehash
import { spawnSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { HASH_PINS_PATH, parseHashPins } from './lib/protocol-hash-pins.mjs'
import { localProtocolVersion, relayPeerProtocolVersion, repositoryRoot } from './lib/protocol-versions.mjs'
import { rustCode } from './lib/rust-production-source.mjs'
import { closing } from './lib/rust-scan.mjs'

// A URL path is percent-encoded (My%20Projects); the working directory and the file paths below must be the decoded filesystem path.
const root = fileURLToPath(repositoryRoot)
const ROUNDS = 3
// Older toolchains print "assertion `left == right` failed" before the two values; newer ones print the values at once.
const MISMATCH = /panicked at (\S+?):(\d+):\d+:\n(?:assertion `left == right` failed(?:: [^\n]*)?\n)?  left: "([0-9a-f]{64})"\n right: "([0-9a-f]{64})"/g

// Pure: the file text with the one pinned literal replaced, or null when the file does not hold it exactly once.
export function replaceLiteral(text, computed, pinned) {
  return text.split(`"${pinned}"`).length === 2 ? text.replace(`"${pinned}"`, `"${computed}"`) : null
}

export function mismatches(output) {
  return [...output.matchAll(MISMATCH)].map(match => {
    const [, file, line, left, right] = match
    const prefix = output.slice(0, match.index)
    // A large-stack test may panic on a named child thread. Libtest's section
    // identifies the registered test whose failure must remain accounted for.
    const section = [...prefix.matchAll(/^---- ([A-Za-z_]\w*(?:::[A-Za-z_]\w*)+) stdout ----$/gm)].at(-1)
    const thread = prefix.match(/thread '([A-Za-z_]\w*(?:::[A-Za-z_]\w*)+)'(?: \(\d+\))? $/)
    return { file, line: Number(line), left, right, test: section?.[1] ?? thread?.[1] }
  })
}

// The test filter of a module path for a kernel source file: apps/kernel/src/a/b.rs -> a::b, a/mod.rs -> a.
export function modulePath(file) {
  const parts = file.replace(/^apps\/kernel\/src\//, '').replace(/\.rs$/, '').split('/')
  if (parts.at(-1) === 'mod') parts.pop()
  return parts.join('::')
}

// Every kernel module that pins a SHA-256 literal: whatever the bump changes, one of these tests pins it. A name filter would miss
// a hash test called xroom_02_owner_lists_pending_cards_with_their_choices.
export function hashModules() {
  const found = spawnSync('git', ['grep', '-lE', '"[0-9a-f]{64}"', '--', 'apps/kernel/src/*.rs'], { cwd: root, encoding: 'utf8' })
  if (found.error) throw found.error
  if (found.status !== 0) throw new Error(`cannot identify pinned modules: ${found.stderr || 'no tracked hash expectations found'}`)
  return [...new Set(found.stdout.split('\n').filter(Boolean).map(modulePath))]
}

function runTests(modules) {
  const result = spawnSync('cargo', ['test', '-q', '-p', 'chariox-kernel', '--lib', '--', ...modules], { cwd: root, encoding: 'utf8', maxBuffer: 1 << 28, stdio: ['ignore', 'pipe', 'pipe'] })
  if (result.error) throw result.error
  return { status: result.status, output: `${result.stdout}\n${result.stderr}` }
}

// A version bump may update a protocol serialization expectation, never an arbitrary
// digest elsewhere in the same module. Inspect the assertion that owns this literal.
function protocolExpectation(file, source, pinned) {
  const at = source.indexOf(`"${pinned}"`)
  if (at < 0 || source.indexOf(`"${pinned}"`, at + 1) >= 0) return null
  const code = rustCode(source)
  let owner
  for (const fn of code.matchAll(/\bfn\s+([A-Za-z_]\w*)\s*\(/g)) {
    const args = code.indexOf('(', fn.index), argsEnd = closing(code, args)
    const open = code.indexOf('{', argsEnd + 1), end = closing(code, open)
    if (argsEnd >= 0 && open >= 0 && !code.slice(argsEnd, open).includes(';') && at > open && at < end) {
      owner = { name: fn[1], open, end }
    }
  }
  if (!owner) return null
  const body = code.slice(owner.open, owner.end)
  const versioned = text => /\b(?:LOCAL_DAEMON_PROTOCOL_VERSION|RELAY_PEER_PROTOCOL_VERSION)\b/.test(text)
  if (!versioned(body)) return null
  for (const call of code.slice(owner.open, owner.end).matchAll(/\b(assert_eq|assert_round_trip|assert_hash)\s*!?\s*\(/g)) {
    const start = owner.open + call.index, open = code.indexOf('(', start), end = closing(code, open)
    if (!(at > open && at < end)) continue
    const assertion = code.slice(start, end)
    if (call[1] !== 'assert_eq') {
      if (/LocalDaemon(?:Request|Response)/.test(code) && /Sha256::digest/.test(code)) return `${modulePath(file)}::${owner.name}`
      continue
    }
    if (!/Sha256::digest/.test(assertion)) continue
    const serialized = assertion.match(/serde_json::to_vec\(\s*&?\s*([A-Za-z_]\w*)\s*\)/)
    const bytes = assertion.match(/Sha256::digest\(\s*&?\s*([A-Za-z_]\w*)\s*\)/)
    const variable = serialized?.[1] ?? bytes?.[1]
    if (!variable) {
      if (/serde_json::to_vec/.test(assertion) && versioned(assertion)) return `${modulePath(file)}::${owner.name}`
      continue
    }
    const prefix = code.slice(owner.open, start)
    const binding = [...prefix.matchAll(new RegExp(`\\blet\\s+(?:mut\\s+)?${variable}\\s*(?::[^=;]+)?=`, 'g'))].at(-1)
    if (!binding) continue
    const from = owner.open + binding.index, to = code.indexOf(';', from)
    if (to < 0 || to > start) continue
    const initializer = code.slice(from, to)
    if (serialized && versioned(initializer)) return `${modulePath(file)}::${owner.name}`
    if (/include_(?:str|bytes)!/.test(initializer) && /\.json"/.test(source.slice(from, to))) return `${modulePath(file)}::${owner.name}`
  }
  return null
}

function failedTests(output) {
  const section = output.split(/\nfailures:\n/).pop() ?? ''
  return section.split('\n').filter(line => /^    \S+::\S+$/.test(line)).map(line => line.trim())
}

function main() {
  let pins
  try { pins = parseHashPins(readFileSync(new URL(HASH_PINS_PATH, repositoryRoot), 'utf8')) } catch (failure) {
    console.error(`protocol:rehash: ${HASH_PINS_PATH}: cannot validate the known baseline (${failure.code ?? failure.message}); restore valid pins before running pnpm protocol:rehash`)
    process.exit(1)
  }
  const bumped = pins.local !== localProtocolVersion || pins.relayPeer !== relayPeerProtocolVersion
  if (localProtocolVersion < pins.local || relayPeerProtocolVersion < pins.relayPeer) {
    console.error('protocol:rehash: a protocol decrease is not an intended bump; restore the known baseline')
    process.exit(1)
  }
  const modules = hashModules()
  let rewritten = 0
  for (let round = 1; round <= ROUNDS; round++) {
    const { status, output } = runTests(modules)
    if (status === 0) {
      if (bumped) writeFileSync(new URL(HASH_PINS_PATH, repositoryRoot), JSON.stringify({ local: localProtocolVersion, relayPeer: relayPeerProtocolVersion }, null, 2) + '\n')
      console.log(`protocol:rehash: ${rewritten} hash(es) rewritten; the protocol tests pass for ${localProtocolVersion}/${relayPeerProtocolVersion}`)
      return
    }
    const found = mismatches(output)
    if (!bumped) {
      console.error(`protocol:rehash: the protocol tests fail at the unchanged protocol ${localProtocolVersion}/${relayPeerProtocolVersion}. A changed hash without a bump is a shape change: raise LOCAL_DAEMON_PROTOCOL_VERSION / RELAY_PEER_PROTOCOL_VERSION, run pnpm protocol:generate, then pnpm protocol:rehash.\n${failedTests(output).join('\n')}`)
      process.exit(1)
    }
    if (!found.length) {
      console.error(`protocol:rehash: ${failedTests(output).length || 'some'} test(s) fail and none is a hash mismatch it can rewrite:\n${failedTests(output).join('\n')}\n${output.slice(-3000)}`)
      process.exit(1)
    }
    const edits = new Map(), expected = new Set()
    for (const { file, line, left, right, test: reportedTest } of found) {
      if (!/^apps\/kernel\/src\/.*\.rs$/.test(file) || file.split('/').includes('..')) {
        console.error(`protocol:rehash: ${file}:${line}: not a tracked kernel protocol expectation`); process.exit(1)
      }
      const text = edits.get(file) ?? readFileSync(`${root}${file}`, 'utf8')
      const test = protocolExpectation(file, text, right)
      const baseline = spawnSync('git', ['show', `HEAD:${file}`], { cwd: root, encoding: 'utf8' })
      if (!test || baseline.status !== 0 || baseline.stdout.split(`"${right}"`).length !== 2) {
        console.error(`protocol:rehash: ${file}:${line}: not a known, version-related protocol expectation; keep this failure and fix it at its source`); process.exit(1)
      }
      if (reportedTest && (!reportedTest.startsWith(`${modulePath(file)}::`) || !new RegExp(`\\bfn\\s+${reportedTest.split('::').at(-1)}\\s*\\(`).test(rustCode(text)))) {
        console.error(`protocol:rehash: ${file}:${line}: the failed test does not belong to this protocol fixture`); process.exit(1)
      }
      expected.add(reportedTest ?? test)
      const next = replaceLiteral(text, left, right)
      if (next === null) { console.error(`protocol:rehash: ${file}:${line}: the pinned hash ${right} is not one literal of that file; edit it by hand to ${left}`); process.exit(1) }
      edits.set(file, next)
    }
    const failures = failedTests(output)
    if (!failures.length || failures.some(test => !expected.has(test))) {
      console.error(`protocol:rehash: unrelated or unidentified tests still fail; no expectations changed:\n${failures.join('\n')}`); process.exit(1)
    }
    for (const [file, next] of edits) writeFileSync(`${root}${file}`, next)
    rewritten += found.length
  }
  console.error(`protocol:rehash: still failing after ${ROUNDS} rounds; run cargo test -p chariox-kernel --lib -- <the modules named in scripts/rehash-protocol-snapshots.mjs>`)
  process.exit(1)
}

// process.argv[1] is a filesystem path and import.meta.url a URL: compare them as URLs, so a checkout path with a space still matches.
export const isEntryPoint = (metaUrl, argv1) => argv1 !== undefined && metaUrl === pathToFileURL(argv1).href

if (isEntryPoint(import.meta.url, process.argv[1])) main()
