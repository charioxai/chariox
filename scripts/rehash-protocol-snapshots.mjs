// pnpm protocol:rehash: after a protocol bump (pnpm protocol:generate), rewrite every pinned snapshot hash the bump changed.
// It runs the tests of every kernel module that pins a SHA-256 literal, replaces each hash literal that no longer matches with the computed one, repeats until
// they pass, then records the protocol pair in fixtures/protocol-hash-pins.json. It refuses to bless a mismatch at an unchanged
// protocol pair: that is a shape change, and a shape change needs the bump first (AGENTS.md "Protocol changes").
// Plain cargo runs underneath, so wrap the whole command like any other cargo job: cargo-slot.sh pnpm protocol:rehash
import { spawnSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { HASH_PINS_PATH, hashPinsProblem } from './lib/protocol-hash-pins.mjs'
import { localProtocolVersion, relayPeerProtocolVersion, repositoryRoot } from './lib/protocol-versions.mjs'

const root = repositoryRoot.pathname
const ROUNDS = 3
const MISMATCH = /panicked at (\S+?):(\d+):\d+:\nassertion `left == right` failed(?:: [^\n]*)?\n  left: "([0-9a-f]{64})"\n right: "([0-9a-f]{64})"/g

// Pure: the file text with the one pinned literal replaced, or null when the file does not hold it exactly once.
export function replaceLiteral(text, computed, pinned) {
  return text.split(`"${pinned}"`).length === 2 ? text.replace(`"${pinned}"`, `"${computed}"`) : null
}

export function mismatches(output) {
  return [...output.matchAll(MISMATCH)].map(([, file, line, left, right]) => ({ file, line: Number(line), left, right }))
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
  return [...new Set(found.stdout.split('\n').filter(Boolean).map(modulePath))]
}

function runTests() {
  const result = spawnSync('cargo', ['test', '-q', '-p', 'chariox-kernel', '--lib', '--', ...hashModules()], { cwd: root, encoding: 'utf8', maxBuffer: 1 << 28, stdio: ['ignore', 'pipe', 'pipe'] })
  if (result.error) throw result.error
  return { status: result.status, output: `${result.stdout}\n${result.stderr}` }
}

function failedTests(output) {
  const section = output.split(/\nfailures:\n/).pop() ?? ''
  return section.split('\n').filter(line => /^    \S+::\S+$/.test(line)).map(line => line.trim())
}

function main() {
  const pinsText = (() => { try { return readFileSync(new URL(HASH_PINS_PATH, repositoryRoot), 'utf8') } catch { return null } })()
  const bumped = hashPinsProblem(pinsText, localProtocolVersion, relayPeerProtocolVersion) !== null
  let rewritten = 0
  for (let round = 1; round <= ROUNDS; round++) {
    const { status, output } = runTests()
    if (status === 0) {
      writeFileSync(new URL(HASH_PINS_PATH, repositoryRoot), JSON.stringify({ local: localProtocolVersion, relayPeer: relayPeerProtocolVersion }, null, 2) + '\n')
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
    for (const { file, line, left, right } of found) {
      const text = readFileSync(`${root}${file}`, 'utf8')
      const next = replaceLiteral(text, left, right)
      if (next === null) { console.error(`protocol:rehash: ${file}:${line}: the pinned hash ${right} is not one literal of that file; edit it by hand to ${left}`); process.exit(1) }
      writeFileSync(`${root}${file}`, next)
      rewritten++
    }
  }
  console.error(`protocol:rehash: still failing after ${ROUNDS} rounds; run cargo test -p chariox-kernel --lib -- <the modules named in scripts/rehash-protocol-snapshots.mjs>`)
  process.exit(1)
}

if (import.meta.url === `file://${process.argv[1]}`) main()
