import assert from "node:assert/strict"
import { createECDH } from "node:crypto"
import { chmodSync, mkdtempSync, readFileSync, rmSync, statSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { RelayClientIdentity } from "@chariox/kernel-client/ipc"

import { createCliRelayIdentityStore } from "./cli-relay-identity-store.js"

test("CLI relay identity is protected and persists the same non-exporting identity", (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")

  const first = createCliRelayIdentityStore(identityPath).getOrCreate()
  const second = createCliRelayIdentityStore(identityPath).load()
  assert.ok(second)
  assert.equal(second.publicKeyBase64, first.publicKeyBase64)
  assert.equal(second.publicKeyThumbprint, first.publicKeyThumbprint)
  assert.deepEqual(Object.keys(first).sort(), ["publicKeyBase64", "publicKeyThumbprint"])
  assert.equal("privateKey" in first, false)
  assert.equal(statSync(identityPath).mode & 0o777, 0o600)

  const stored = JSON.parse(readFileSync(identityPath, "utf8")) as { private_key: string }
  assert.equal(typeof stored.private_key, "string")
  const peer = createTestRelayIdentity()
  const encrypted = first.encrypt(peer.publicKeyBase64, "cli identity round trip")
  assert.equal(peer.decrypt(encrypted), "cli identity round trip")
  assert.equal(
    first.decrypt(peer.encrypt(first.publicKeyBase64, Buffer.from("persisted identity round trip"))),
    "persisted identity round trip",
  )
})

test("CLI relay identity refuses a group-readable key file", (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-mode-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")
  createCliRelayIdentityStore(identityPath).getOrCreate()
  chmodSync(identityPath, 0o644)

  assert.throws(() => createCliRelayIdentityStore(identityPath).load(), /mode-0600/)
})

function createTestRelayIdentity(): RelayClientIdentity {
  const ecdh = createECDH("prime256v1")
  ecdh.generateKeys()
  const privateKey = ecdh.getPrivateKey()
  try {
    return new RelayClientIdentity(privateKey)
  } finally {
    privateKey.fill(0)
  }
}
