import assert from "node:assert/strict"
import test from "node:test"

import { pairCloudRelayClient, pairCloudRelayMachine } from "./cloud-relay.js"
import type { RelayCloudProfile } from "./preferences.js"

function profile(): RelayCloudProfile {
  return {
    apiUrl: "https://cloud.example.invalid/", email: "fixture@example.invalid",
    accountId: "fixture-account", userId: "fixture-user", accountSlug: "fixture",
    realmId: "fixture-realm", relayUrl: "wss://relay.example.invalid", issuerId: "fixture-issuer",
    clientId: "prior-client", machineId: "prior-machine",
    cloudSessionToken: "synthetic-cloud-session", machineCredential: "synthetic-machine-credential",
  }
}

test("explicit client and machine pairing forward the Cloud session bearer to token admission", async (t) => {
  for (const kind of ["client", "machine"] as const) {
    const linked = Object.freeze(profile())
    const requests: { url: string; headers: Headers; body: Record<string, unknown> }[] = []
    const fetch = t.mock.method(globalThis, "fetch", async (...[input, init]: Parameters<typeof globalThis.fetch>) => {
      const url = String(input)
      requests.push({ url, headers: new Headers(init?.headers), body: JSON.parse(String(init?.body)) })
      return Response.json(url.endsWith("/pairing-tokens") ? { token: "synthetic-pairing-token" } : {})
    })
    const paired = kind === "client"
      ? await pairCloudRelayClient(linked, "new-client", "client alias")
      : await pairCloudRelayMachine(linked, "new-machine", "machine alias", { profileVersion: 1 })
    assert.equal(requests.length, 2)
    assert.equal(requests[0]?.url, "https://cloud.example.invalid/pairing-tokens")
    assert.equal(requests[0]?.headers.get("authorization"), "Bearer synthetic-cloud-session")
    assert.equal(requests[0]?.body.subjectKind, kind)
    assert.equal(requests[0]?.body.accountId, linked.accountId)
    assert.equal(requests[0]?.body.sessionToken, undefined)
    assert.equal(requests[0]?.body.machineCredential, undefined)
    assert.equal(requests[1]?.url, `https://cloud.example.invalid/${kind === "client" ? "clients" : "machines"}/pair`)
    assert.equal(requests[1]?.body.token, "synthetic-pairing-token")
    assert.equal(paired[kind === "client" ? "clientId" : "machineId"], `new-${kind}`)
    assert.equal(linked.clientId, "prior-client")
    assert.equal(linked.machineId, "prior-machine")
    fetch.mock.restore()
  }
})

test("explicit pairing with a missing or blank session makes no request and preserves its profile", async (t) => {
  let requests = 0
  t.mock.method(globalThis, "fetch", async () => {
    requests++
    return Response.json({ token: "synthetic-pairing-token" })
  })
  for (const cloudSessionToken of [undefined, "", " ", "\n\t"]) {
    const linked = profile()
    if (cloudSessionToken === undefined) delete linked.cloudSessionToken
    else linked.cloudSessionToken = cloudSessionToken
    const before = structuredClone(linked)
    await assert.rejects(pairCloudRelayClient(linked, "new-client"), /cloud session.*login/i)
    await assert.rejects(pairCloudRelayMachine(linked, "new-machine"), /cloud session.*login/i)
    assert.deepEqual(linked, before)
  }
  assert.equal(requests, 0, "machine credentials cannot substitute for account-wide pairing authority")
})

test("denied pairing-token admission does not redeem a token or replace a linked identity", async (t) => {
  const linked = profile()
  const before = structuredClone(linked)
  let requests = 0
  t.mock.method(globalThis, "fetch", async () => {
    requests++
    return Response.json({ error: { message: "account operate access denied" } }, { status: 403 })
  })
  await assert.rejects(pairCloudRelayClient(linked, "new-client"), /account operate access denied/)
  await assert.rejects(pairCloudRelayMachine(linked, "new-machine"), /account operate access denied/)
  assert.equal(requests, 2, "each denied admission stops before token redemption")
  assert.deepEqual(linked, before)
})
