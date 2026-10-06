import assert from "node:assert/strict"
import test from "node:test"
import { CloudClient } from "./cloud-client.js"
import { CloudClientCredentialStore } from "./cloud-client-credential-store.js"
import { CloudClientAuthError } from "./cloud-client-http.js"
import type { RelayClientIdentity } from "./ipc.js"

// MP-08/MP-11: #898 denial negotiation must survive #888's client-only login.
function client(t: test.TestContext) {
  const store = new CloudClientCredentialStore("/unused-synthetic-client-profile")
  t.mock.method(store, "load", async () => null)
  t.mock.method(store, "saveLogin", async () => assert.fail("denied login saved authority"))
  return new CloudClient(store, () => ({ publicKeyThumbprint: "a".repeat(64) }) as RelayClientIdentity)
}

const started = {
  deviceCode: "synthetic-device", userCode: "PUBLIC-CODE",
  verificationUrl: "https://cloud.example.test/activate", intervalSeconds: 1,
  expiresAt: "2099-01-01T00:00:00Z",
}

test("client-only login advertises denial and stops without saving authority", async t => {
  const api = client(t)
  const polls: unknown[] = []
  t.mock.method(globalThis, "fetch", async (url: URL, options: RequestInit) => {
    const body = JSON.parse(String(options.body))
    if (url.pathname === "/auth/device/start") return Response.json(started)
    assert.equal(url.pathname, "/auth/device/poll")
    polls.push(body)
    assert.deepEqual(body, { deviceCode: "synthetic-device", supportsAccessDenied: true })
    return Response.json({ status: "access_denied" })
  })
  try {
    await assert.rejects(api.login("https://cloud.example.test", () => {}),
      (error: unknown) => error instanceof CloudClientAuthError && error.code === "login_denied")
    assert.equal(polls.length, 1)
  } finally { api.stop() }
})

test("client-only poll retries the exact legacy schema rejection once", async t => {
  const api = client(t)
  const polls: unknown[] = []
  t.mock.method(globalThis, "fetch", async (url: URL, options: RequestInit) => {
    if (url.pathname === "/auth/device/start") return Response.json(started)
    const body = JSON.parse(String(options.body))
    polls.push(body)
    if (polls.length === 1) return Response.json({ error: { code: "invalid_request", message: "Request validation failed" } }, { status: 400 })
    return Response.json({ status: "access_denied" })
  })
  try {
    await assert.rejects(api.login("https://cloud.example.test", () => {}),
      (error: unknown) => error instanceof CloudClientAuthError && error.code === "login_denied")
    assert.deepEqual(polls, [
      { deviceCode: "synthetic-device", supportsAccessDenied: true },
      { deviceCode: "synthetic-device" },
    ])
  } finally { api.stop() }
})

test("client-only poll keeps other failures redacted and never downgrades", async t => {
  for (const [status, code, message] of [
    [401, "invalid_request", "Request validation failed"],
    [403, "authorization_denied", "Request validation failed"],
    [500, "invalid_request", "Request validation failed"],
    [400, "authorization_denied", "Request validation failed"],
    [400, "invalid_request", "synthetic-private-response"],
  ] as const) {
    const api = client(t)
    let polls = 0
    const fetch = t.mock.method(globalThis, "fetch", async (url: URL) => {
      if (url.pathname === "/auth/device/start") return Response.json(started)
      polls++
      return Response.json({ error: { code, message } }, { status })
    })
    try {
      await assert.rejects(api.login("https://cloud.example.test", () => {}),
        (error: unknown) => error instanceof CloudClientAuthError && error.code === code
          && !error.message.includes(message))
      assert.equal(polls, 1)
    } finally { api.stop(); fetch.mock.restore() }
  }
})

test("client-only poll transport failure never downgrades", async t => {
  const api = client(t), failure = new Error("synthetic network failure")
  let polls = 0
  t.mock.method(globalThis, "fetch", async (url: URL) => {
    if (url.pathname === "/auth/device/start") return Response.json(started)
    polls++
    throw failure
  })
  try {
    await assert.rejects(api.login("https://cloud.example.test", () => {}), error => error === failure)
    assert.equal(polls, 1)
  } finally { api.stop() }
})
