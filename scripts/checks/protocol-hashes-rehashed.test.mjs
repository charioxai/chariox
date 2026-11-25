import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './protocol-hashes-rehashed.mjs'

const TYPES = 'apps/kernel/src/local/api/types.rs'
const PEER = 'apps/kernel/src/transport/relay_peer.rs'
const PINS = 'fixtures/protocol-hash-pins.json'
const files = (local, peer, pins) => ({
  [TYPES]: `pub const LOCAL_DAEMON_PROTOCOL_VERSION: u32 = ${local};\n`,
  [PEER]: `pub const RELAY_PEER_PROTOCOL_VERSION: u32 = ${peer};\n`,
  [PINS]: JSON.stringify(pins) + '\n',
})

test('pins that match the protocol pair pass', () => {
  const r = makeRepo(files(10, 5, { local: 10, relayPeer: 5 }))
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a bump without the rehash is red and names the command', () => {
  const r = makeRepo(files(10, 5, { local: 10, relayPeer: 5 }))
  r.write(TYPES, files(11, 6, { local: 10, relayPeer: 5 })[TYPES])
  r.write(PEER, files(11, 6, { local: 10, relayPeer: 5 })[PEER])
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /protocol-hash-pins\.json:1: the protocol is 11\/6 but the snapshot hashes .* pinned for 10\/5; run pnpm protocol:rehash\. Root fix: run pnpm protocol:rehash/)
  r.dispose()
})

test('a bump of the relay peer alone is red too', () => {
  const r = makeRepo(files(10, 5, { local: 10, relayPeer: 5 }))
  r.write(PEER, files(10, 6, { local: 10, relayPeer: 5 })[PEER])
  assert.equal(r.run(check.name).status, 1)
  r.dispose()
})

test('the bump with the re-pinned file passes', () => {
  const r = makeRepo(files(10, 5, { local: 10, relayPeer: 5 }))
  for (const [path, text] of Object.entries(files(11, 6, { local: 11, relayPeer: 6 }))) r.write(path, text)
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a missing pins file is red', () => {
  const r = makeRepo({ [TYPES]: files(10, 5, {})[TYPES], [PEER]: files(10, 5, {})[PEER] })
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /run pnpm protocol:rehash/)
  r.dispose()
})
