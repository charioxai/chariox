import assert from "node:assert/strict"
import test from "node:test"

import { pairCloudRelayClient, pairCloudRelayMachine, pollCloudDeviceLogin } from "./cloud-relay.js"
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

test("device denial is a terminal direct CLI poll result without an approval profile", async (t) => {
  t.mock.method(globalThis, "fetch", async (_url: unknown, init?: RequestInit) => {
    assert.deepEqual(JSON.parse(String(init?.body)), { deviceCode: "synthetic-device-code", supportsAccessDenied: true })
    return Response.json({ status: "access_denied" })
  })
  assert.deepEqual(await pollCloudDeviceLogin("https://cloud.example.test", "synthetic-device-code"), { status: "access_denied" })
})

test("device login completes against a legacy Cloud poll schema", async (t) => {
  const bodies: Record<string, unknown>[] = []
  let legacyPolls = 0
  t.mock.method(globalThis, "fetch", async (_url: unknown, init?: RequestInit) => {
    const body = JSON.parse(String(init?.body))
    bodies.push(body)
    // Cloud before denial negotiation rejects additionalProperties before polling.
    if ("supportsAccessDenied" in body) {
      return Response.json({ error: { code: "invalid_request", message: "Request validation failed" } }, { status: 400 })
    }
    assert.deepEqual(body, { deviceCode: "synthetic-device-code" })
    legacyPolls++
    return Response.json(legacyPolls === 1
      ? { status: "authorization_pending", intervalSeconds: 1, expiresAt: "2030-01-01T00:00:00Z" }
      : { status: "approved", profile: profile(), cloudSessionToken: "synthetic-cloud-session",
          cloudSessionExpiresAt: "2030-01-01T00:00:00Z" })
  })
  assert.equal((await pollCloudDeviceLogin("https://cloud.example.test", "synthetic-device-code")).status, "authorization_pending")
  const approved = await pollCloudDeviceLogin("https://cloud.example.test", "synthetic-device-code")
  assert.equal(approved.status, "approved")
  if (approved.status === "approved") assert.equal(approved.profile.accountId, "fixture-account")
  assert.deepEqual(bodies, [true, false, true, false].map((advertised) => ({
    deviceCode: "synthetic-device-code", ...(advertised ? { supportsAccessDenied: true } : {}),
  })))
})

test("poll fallback is bounded and limited to legacy schema rejection", async (t) => {
  for (const [status, code, message, expectedRequests] of [
    [401, "invalid_request", "Request validation failed", 1],
    [403, "authorization_denied", "Forbidden", 1],
    [500, "invalid_request", "Request validation failed", 1],
    [400, "authorization_denied", "Request validation failed", 1],
    [400, "invalid_request", "Invalid device code", 1],
    [400, "invalid_request", "Request validation failed", 2],
  ] as const) {
    let requests = 0
    const fetch = t.mock.method(globalThis, "fetch", async () => {
      requests++
      return Response.json({ error: { code, message } }, { status })
    })
    await assert.rejects(pollCloudDeviceLogin("https://cloud.example.test", "synthetic-device-code"), (error: unknown) => {
      assert.equal((error as Error).message, message)
      return true
    })
    assert.equal(requests, expectedRequests, `${status} ${code}: ${message}`)
    fetch.mock.restore()
  }
})

test("poll transport failure does not downgrade the capability request", async (t) => {
  const failure = new Error("synthetic network failure")
  let requests = 0
  t.mock.method(globalThis, "fetch", async () => {
    requests++
    throw failure
  })
  await assert.rejects(pollCloudDeviceLogin("https://cloud.example.test", "synthetic-device-code"), (error) => error === failure)
  assert.equal(requests, 1)
})
