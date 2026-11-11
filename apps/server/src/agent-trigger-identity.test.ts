import test, {mock} from "node:test"
import {createECDH, ECDH} from "node:crypto"
import {relayKeypairFromPrivateKey} from "@chariox/kernel-client/agent-trigger-ingress-client"
import assert from "node:assert/strict"
import {chmodSync, mkdtempSync, rmSync, writeFileSync} from "node:fs"
import {tmpdir} from "node:os"
import {join} from "node:path"
import {createTriggerIdentity, loadTriggerIdentity} from "./agent-trigger-identity.js"
import {pairTriggerIngress} from "./agent-trigger-pairing.js"

test("the ingress relay key is created once, private, and loaded back unchanged", () => {
  const dir = mkdtempSync(join(tmpdir(), "trigger-identity-"))
  try {
    const path = join(dir, "identity.json")
    assert.equal(loadTriggerIdentity(path), null)
    const key = createTriggerIdentity(path)
    assert.equal(key.length, 32)
    assert.ok(loadTriggerIdentity(path)!.equals(key))
    assert.throws(() => createTriggerIdentity(path), /EEXIST/, "an existing key is never overwritten")
    chmodSync(path, 0o644)
    assert.throws(() => loadTriggerIdentity(path), /private regular file/)
    chmodSync(path, 0o600); writeFileSync(path, JSON.stringify({version: 1, private_key: "short"}))
    assert.throws(() => loadTriggerIdentity(path), /pair again/)
  } finally {rmSync(dir, {recursive: true, force: true})}
})

test("a relay key whose scalar has leading zero bytes is stored at full width and still imports", async () => {
  const dir = mkdtempSync(join(tmpdir(), "trigger-identity-"))
  // OpenSSL drops leading zero bytes from getPrivateKey(); pin the generated key to one that has two.
  const scalar = Buffer.concat([Buffer.alloc(2), Buffer.from("0123456789abcdef0123456789abcdef0123456789abcdef0123456789ab", "hex")])
  assert.equal(scalar.length, 32)
  const generate = mock.method(ECDH.prototype, "generateKeys", function (this: ECDH) { this.setPrivateKey(scalar); return this.getPublicKey() })
  try {
    const path = join(dir, "identity.json")
    const key = createTriggerIdentity(path)
    assert.equal(generate.mock.callCount(), 1)
    assert.ok(key.equals(scalar), "the key is the 32-byte big-endian scalar")
    const loaded = loadTriggerIdentity(path)
    assert.ok(loaded?.equals(scalar), "a saved key can be loaded again")
    const keypair = await relayKeypairFromPrivateKey(loaded!)
    assert.equal(keypair.privateKey.type, "private")
    const expected = createECDH("prime256v1"); expected.setPrivateKey(scalar)
    assert.equal(keypair.publicKeyBase64, expected.getPublicKey().toString("base64"))
  } finally {generate.mock.restore(); rmSync(dir, {recursive: true, force: true})}
})

test("pairing refuses a link that is not a terminal pairing link or names a kernel with no deployment", async () => {
  const dir = mkdtempSync(join(tmpdir(), "trigger-pairing-"))
  try {
    const file = join(dir, "deployments.json")
    writeFileSync(file, JSON.stringify({relay: {url: "ws://127.0.0.1:9", token: "t"}, deployments: [{id: "a", kernelId: "kernel-1", sessionId: "s", triggerId: "t", accessToken: "x".repeat(40)}]}), {mode: 0o600})
    await assert.rejects(pairTriggerIngress(file, "not-a-link"), /terminal pairing link|JSON/)
    const link = `chariox-terminal-pair-v1.${Buffer.from(JSON.stringify({target_daemon_id: "kernel-2"})).toString("base64url")}`
    await assert.rejects(pairTriggerIngress(file, link), /no deployment .* belongs to kernel kernel-2/)
    assert.equal(loadTriggerIdentity(join(dir, "agent-trigger-identity.json")), null, "a refused link leaves no key behind")
  } finally {rmSync(dir, {recursive: true, force: true})}
})
