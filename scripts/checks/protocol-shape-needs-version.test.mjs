import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './protocol-shape-needs-version.mjs'

const TYPES = 'apps/kernel/src/local/api/types/agent.rs'
const VERSION = 'apps/kernel/src/local/api/types.rs'
const SNAPSHOT = 'apps/kernel/src/local/api/tests/protocol_shapes/agent-1.json'
const struct = fields => `#[derive(Serialize)]\npub struct Agent {\n${fields.map(f => `    pub ${f}: u32,\n`).join('')}}\n`
const version = n => `pub const LOCAL_DAEMON_PROTOCOL_VERSION: u32 = ${n};\n`
const base = () => ({ [TYPES]: struct(['id']), [VERSION]: version(10), [SNAPSHOT]: '{"hash":"a"}\n' })

test('a change that leaves the wire alone passes', () => {
  const r = makeRepo(base())
  r.write(TYPES, `// the agent\n${struct(['id'])}`)
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a new serialized field without a version bump is red and names the constant', () => {
  const r = makeRepo(base())
  r.write(TYPES, struct(['id', 'done_text']))
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /types\/agent\.rs:4: changes a serialized shape .*without raising LOCAL_DAEMON_PROTOCOL_VERSION\. Root fix: raise the protocol constant named here/)
  r.dispose()
})

test('a bump without a snapshot update is red', () => {
  const r = makeRepo(base())
  r.write(TYPES, struct(['id', 'done_text']))
  r.write(VERSION, version(11))
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /raises LOCAL_DAEMON_PROTOCOL_VERSION, but no protocol snapshot changed/)
  r.dispose()
})

test('a field, the bump and the snapshot together pass', () => {
  const r = makeRepo(base())
  r.write(TYPES, struct(['id', 'done_text']))
  r.write(VERSION, version(11))
  r.write(SNAPSHOT, '{"hash":"b"}\n')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('the relay peer protocol has its own constant', () => {
  const peer = 'apps/kernel/src/transport/relay_peer.rs'
  const r = makeRepo({ [peer]: `${struct(['id'])}pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 5;\n` })
  r.write(peer, `${struct(['id', 'extra'])}pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 5;\n`)
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /without raising RELAY_PEER_PROTOCOL_VERSION/)
  r.dispose()
})

test('a change committed on a branch is judged against the merge base too', () => {
  const r = makeRepo(base())
  r.write(TYPES, struct(['id', 'done_text']))
  r.commit('add field')
  assert.equal(r.run(check.name).status, 1)
  r.dispose()
})
