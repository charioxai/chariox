// The pinned snapshot hashes in the protocol tests are for one protocol pair; fixtures/protocol-hash-pins.json records which.
// `pnpm protocol:rehash` rewrites the hashes and the pins after a bump. The generator check and the rule check share this message.
export const HASH_PINS_PATH = 'fixtures/protocol-hash-pins.json'

// null when the pins match the protocol, else the sentence that names the command.
export function hashPinsProblem(pinsText, local, relayPeer) {
  let pins
  try { pins = JSON.parse(pinsText ?? '') } catch { return `${HASH_PINS_PATH} is missing or unreadable; run pnpm protocol:rehash` }
  if (pins.local === local && pins.relayPeer === relayPeer) return null
  return `the protocol is ${local}/${relayPeer} but the snapshot hashes in the protocol tests are pinned for ${pins.local}/${pins.relayPeer}; run pnpm protocol:rehash`
}
