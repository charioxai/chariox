import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient, RelayClientIdentity } from "./ipc.js"
import { startPairedRelayRenewal } from "./paired-relay-renewal.js"

const thumbprint = "a".repeat(64)

function token(alg: string, expiresInSeconds: number) {
  const encode = (value: unknown) => Buffer.from(JSON.stringify(value)).toString("base64url")
  return [
    encode({ alg, typ: "JWT" }),
    encode({ sub: "cli-1", exp: Math.floor(Date.now() / 1000) + expiresInSeconds, allowed_targets: ["home"], public_key_thumbprint: thumbprint }),
    "signature",
  ].join(".")
}

test("MP-11 a kernel-issued (EdDSA) terminal grant renews through IssueRelayClientToken, not Cloud", async () => {
  const sent: unknown[] = []
  let renew: (() => Promise<{ token: string; expiresAtMs: number }>) | undefined
  const visible = { startRelayAuthRenewal: (_expiry: number, callback: typeof renew) => { renew = callback } } as unknown as LocalIpcClient
  const fresh = token("EdDSA", 900)
  const issuer = {
    send: async (request: unknown) => {
      sent.push(request)
      return { RelayClientTokenIssued: { token: { relay_url: "ws://relay.test", relay_token: fresh, target_daemon_id: "home", token_id: "t2", expires_at_ms: Date.now() + 900_000 } } }
    },
    close: async () => {},
  } as unknown as LocalIpcClient
  startPairedRelayRenewal(visible, {
    token: token("EdDSA", 600), subject: "cli-1", endpoint: "ws://relay.test", target: "home", terminalId: "",
    identity: { publicKeyThumbprint: thumbprint } as RelayClientIdentity,
    createClient: () => issuer,
  })
  const renewed = await renew!()
  assert.equal(renewed.token, fresh)
  assert.deepEqual(sent, [{ IssueRelayClientToken: { client_id: "cli-1", public_key_thumbprint: thumbprint, scope: "attach" } }])
})

test("MP-11 a Cloud (HS256) terminal grant keeps renewing through IssueCloudRelayClientToken", async () => {
  const sent: Array<Record<string, unknown>> = []
  let renew: (() => Promise<unknown>) | undefined
  const visible = { startRelayAuthRenewal: (_expiry: number, callback: typeof renew) => { renew = callback } } as unknown as LocalIpcClient
  const issuer = {
    send: async (request: Record<string, unknown>) => {
      sent.push(request)
      throw new Error("stop after recording the request")
    },
    close: async () => {},
  } as unknown as LocalIpcClient
  startPairedRelayRenewal(visible, {
    token: token("HS256", 600), subject: "cli-1", endpoint: "ws://relay.test", target: "home", terminalId: "cli-1",
    identity: { publicKeyThumbprint: thumbprint } as RelayClientIdentity,
    createClient: () => issuer,
  })
  await assert.rejects(renew!())
  assert.deepEqual(Object.keys(sent[0]!), ["IssueCloudRelayClientToken"])
})
