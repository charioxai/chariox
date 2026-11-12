import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { test } from "node:test"
import { startOwnerCloud } from "./local-owner-cloud.test-support.mjs"

test("account-copy directory retains the enrolled identity through presence updates", async t => {
  let server, observed
  const env = await startOwnerCloud({ token: () => "test-token", note: () => {}, realm: "test-realm", relayPort: 1, onServer: value => { server = value }, onPresence: peer => { observed = peer } })
  t.after(() => server?.close())
  const base = JSON.parse(env.CHARIOX_CLOUD_RELAY_CONFIG_JSON).api_url
  const post = (path, body) => fetch(base + path, { method: "POST", body: JSON.stringify(body) })
  const key = Buffer.alloc(32, 3).toString("base64")
  const thumbprint = createHash("sha256").update(key).digest("hex")
  await post("/auth/kernel/migrate", { kernelId: "kernel-a", machineId: "machine-a", publicKeyThumbprint: thumbprint })
  await post("/kernels/presence", { kernelId: "kernel-a", machineId: "machine-a", metadata: { relay_public_key: key }, status: "ONLINE" })
  assert.deepEqual(observed, { kernelId: "kernel-a", keyThumbprint: thumbprint })
  const response = await fetch(base + "/v1/owner-managed-context-tickets/peers")
  assert.equal(response.status, 200)
  assert.deepEqual(await response.json(), { peers: [{ targetId: "kernel-a", peer: { relayRealmId: "test-realm", machineId: "machine-a", kernelId: "kernel-a", relayPublicKey: key, keyThumbprint: thumbprint } }] })
})
