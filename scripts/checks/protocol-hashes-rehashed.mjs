import { u32Constant } from '../lib/rust-wire-shapes.mjs'
import { HASH_PINS_PATH, hashPinsProblem } from '../lib/protocol-hash-pins.mjs'

// Protocol changes: a protocol bump re-pins the snapshot hashes in the protocol tests, with one command.
// State, not diff: the pins file names the protocol pair the hashes were last rewritten for, and it has to be the pair in the code.
const LOCAL = { file: 'apps/kernel/src/local/api/types.rs', constant: 'LOCAL_DAEMON_PROTOCOL_VERSION' }
const RELAY_PEER = { file: 'apps/kernel/src/transport/relay_peer.rs', constant: 'RELAY_PEER_PROTOCOL_VERSION' }

export default {
  name: 'protocol-hashes-rehashed',
  ratchet: 'diff',
  rule: 'AGENTS.md "Protocol changes": a bump updates the snapshot and hash tests',
  rootFix: 'run pnpm protocol:rehash (through cargo-slot.sh like any cargo job) and commit the rewritten tests and fixtures/protocol-hash-pins.json',
  scan(ctx) {
    const [local, relayPeer] = [LOCAL, RELAY_PEER].map(({ file, constant }) => u32Constant(ctx.read(file) ?? '', constant))
    if (local === null || relayPeer === null) return []
    const problem = hashPinsProblem(ctx.read(HASH_PINS_PATH), local, relayPeer)
    return problem ? [{ file: HASH_PINS_PATH, line: 1, what: problem }] : []
  },
}
