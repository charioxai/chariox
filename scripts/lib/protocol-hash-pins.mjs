// The pinned snapshot hashes in the protocol tests are for one protocol pair; fixtures/protocol-hash-pins.json records which.
// `pnpm protocol:rehash` rewrites the hashes and the pins after a bump. The generator check and the rule check share this message.
export const HASH_PINS_PATH = 'fixtures/protocol-hash-pins.json'

export function parseHashPins(pinsText) {
  let pins
  try { pins = JSON.parse(pinsText ?? '') } catch {
    throw new Error(`${HASH_PINS_PATH} is missing, unreadable or malformed; restore its known baseline before running pnpm protocol:rehash`)
  }
  const version = value => Number.isInteger(value) && value > 0 && value <= 0xffffffff
  if (!pins || Array.isArray(pins) || typeof pins !== 'object'
      || Object.keys(pins).sort().join(',') !== 'local,relayPeer'
      || !version(pins.local) || !version(pins.relayPeer)) {
    throw new Error(`${HASH_PINS_PATH} must contain only valid local and relayPeer u32 versions; restore its known baseline before running pnpm protocol:rehash`)
  }
  return pins
}

// null when the pins match the protocol, else the sentence that names the command.
export function hashPinsProblem(pinsText, local, relayPeer) {
  let pins
  try { pins = parseHashPins(pinsText) } catch (failure) { return failure.message }
  if (pins.local === local && pins.relayPeer === relayPeer) return null
  return `the protocol is ${local}/${relayPeer} but the snapshot hashes in the protocol tests are pinned for ${pins.local}/${pins.relayPeer}; run pnpm protocol:rehash`
}
