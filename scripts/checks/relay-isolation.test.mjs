import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './relay-isolation.mjs'

const files = () => ({
  'apps/relay/Cargo.toml': '[package]\nname = "chariox-relay"\n\n[dependencies]\nserde = "1"\n\n[dev-dependencies]\ntokio = "1"\n',
  'apps/relay/src/server.rs': 'use serde::Serialize;\n',
})
const planted = (r, label) => {
  const result = r.run(check.name)
  assert.equal(result.status, 1, label)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`), `${label}: names the root fix`)
  return result.text
}

test('a relay that forwards ciphertext passes', () => {
  const r = makeRepo(files())
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a kernel crate dependency is red', () => {
  const r = makeRepo(files())
  r.write('apps/relay/Cargo.toml', files()['apps/relay/Cargo.toml'].replace('serde = "1"\n', 'serde = "1"\nchariox-kernel = { path = "../kernel" }\n'))
  assert.match(planted(r, 'path'), /apps\/relay\/Cargo\.toml:6: the relay depends on the workspace crate chariox-kernel by path/)
  r.dispose()
})

test('a decryption crate is red', () => {
  const r = makeRepo(files())
  r.write('apps/relay/Cargo.toml', files()['apps/relay/Cargo.toml'].replace('serde = "1"\n', 'serde = "1"\nchacha20poly1305 = "0.10"\n'))
  assert.match(planted(r, 'aead'), /the relay depends on the decryption crate chacha20poly1305/)
  r.dispose()
})

test('relay code using a kernel module is red, a test using one is not', () => {
  const r = makeRepo(files())
  r.write('apps/relay/tests/e2e.rs', 'use chariox_kernel::session::Prompt;\n')
  assert.equal(r.run(check.name).status, 0)
  r.write('apps/relay/src/server.rs', 'use serde::Serialize;\nuse chariox_kernel::session::Prompt;\n')
  assert.match(planted(r, 'use'), /apps\/relay\/src\/server\.rs:2: relay code uses chariox_kernel/)
  r.dispose()
})

test('relay code that moves, measures and encodes ciphertext passes', () => {
  const r = makeRepo(files())
  r.write('apps/relay/src/server.rs', 'fn forward(payload: &Payload) -> Vec<u8> {\n    if payload.ciphertext.len() < 16 { return Vec::new() }\n    STANDARD.decode(&payload.ciphertext).unwrap()\n}\n')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('relay code that parses, decodes to text, searches or logs the ciphertext is red', () => {
  for (const [label, statement] of [
    ['parse', 'let value: serde_json::Value = serde_json::from_slice(&ciphertext).unwrap();'],
    ['text', 'let text = String::from_utf8(ciphertext.clone()).unwrap();'],
    ['search', 'let hit = payload.ciphertext.contains("prompt");'],
    ['log', 'tracing::debug!("forwarding {}", payload.ciphertext);'],
    ['brace macro', 'tracing::debug! { ciphertext = ?payload.ciphertext };'],
    ['brace macro in a block', 'if verbose { tracing::debug! { bytes = ?payload.ciphertext } }'],
  ]) {
    const r = makeRepo(files())
    r.write('apps/relay/src/server.rs', `fn forward(payload: &Payload) {\n    ${statement}\n}\n`)
    assert.match(planted(r, label), /apps\/relay\/src\/server\.rs:2: relay code inspects the payload field ciphertext/, label)
    r.dispose()
  }
})

test('a test module that reads the ciphertext does not count', () => {
  const r = makeRepo(files())
  r.write('apps/relay/src/server.rs', '#[cfg(test)]\nmod tests {\n    fn t(p: &Payload) { let v = serde_json::from_slice(&p.ciphertext); }\n}\n')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})
