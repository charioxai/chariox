import test from "node:test"
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
