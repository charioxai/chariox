import assert from "node:assert/strict"
import test from "node:test"
import { mkdtemp, rm, stat } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { CloudClientCredentialStore, type CloudClientCredential } from "./cloud-client-credential-store.js"

const key = "a".repeat(64)
function credential(): CloudClientCredential {
  return { profile: { accountId: "account-fixture", apiUrl: "https://cloud.example.test" } as CloudClientCredential["profile"], clientId: "cli-fixture", publicKeyThumbprint: key, refreshCredential: "synthetic-refresh", accessToken: "synthetic-access", expiresAtMs: 0 }
}

test("CLI profile rotates once across concurrent processes/stores and retains its private family", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-client-refresh-")), before = globalThis.fetch
  try {
    const store = new CloudClientCredentialStore(join(root, "a", "client.json"))
    const otherProcess = new CloudClientCredentialStore(store.filePath)
    const otherProfile = new CloudClientCredentialStore(join(root, "b", "client.json"))
    await store.saveLogin(credential())
    let calls = 0
    globalThis.fetch = (async () => {
      calls++
      await new Promise(resolve => setTimeout(resolve, 30))
      return Response.json({ refreshCredential: "synthetic-refresh-next", cloudSessionToken: "synthetic-access-next", cloudSessionExpiresAt: new Date(Date.now()+900_000).toISOString() })
    }) as typeof fetch
    const results = await Promise.all([store.session(key), otherProcess.session(key)])
    assert.equal(calls, 1)
    assert.ok(results.every(result => result.refreshCredential === "synthetic-refresh-next"))
    assert.equal((await stat(store.filePath)).mode & 0o777, 0o600)
    assert.equal(await otherProfile.load(), null)
    await assert.rejects(store.session("b".repeat(64)), /profile_conflict/)
  } finally { globalThis.fetch = before; await rm(root, { recursive: true, force: true }) }
})

test("lost refresh reply survives restart with the same rotation ID; revoked family is removed", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-client-refresh-retry-")), before = globalThis.fetch
  try {
    const store = new CloudClientCredentialStore(join(root, "client.json"))
    await store.saveLogin(credential())
    let rotationId: string | undefined, calls = 0
    globalThis.fetch = (async (_url, options) => {
      const body = JSON.parse(options!.body as string)
      if (++calls === 1) { rotationId = body.rotationId; throw new Error("fixture lost response") }
      assert.equal(body.rotationId, rotationId)
      return Response.json({ refreshCredential: "synthetic-recovered", cloudSessionToken: "synthetic-access-next", cloudSessionExpiresAt: new Date(Date.now()+900_000).toISOString() })
    }) as typeof fetch
    await assert.rejects(store.session(key), /lost response/)
    const restarted = new CloudClientCredentialStore(store.filePath)
    assert.ok((await restarted.load())!.pendingRotationId)
    await restarted.session(key)
    assert.equal((await restarted.load())!.pendingRotationId, undefined)
    globalThis.fetch = (async () => Response.json({ error: { code: "refresh_reuse_detected" } }, { status: 401 })) as typeof fetch
    await assert.rejects(restarted.session(key, true), /refresh_reuse_detected/)
    assert.equal(await restarted.load(), null)
  } finally { globalThis.fetch = before; await rm(root, { recursive: true, force: true }) }
})

test("different CLI profiles rotate independently while another profile waits for Cloud", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-client-refresh-independent-")), before = globalThis.fetch
  let release!: () => void
  const held = new Promise<void>(resolve => { release = resolve })
  try {
    const a = new CloudClientCredentialStore(join(root, "a.json"))
    const b = new CloudClientCredentialStore(join(root, "b.json"))
    await a.saveLogin(credential())
    await b.saveLogin({ ...credential(), clientId: "other-cli" })
    let started!: () => void
    const startedA = new Promise<void>(resolve => { started = resolve })
    globalThis.fetch = (async (_url, options) => {
      if (JSON.parse(options!.body as string).clientId === "cli-fixture") { started(); await held }
      return Response.json({ refreshCredential: "synthetic-next", cloudSessionToken: "synthetic-access-next", cloudSessionExpiresAt: new Date(Date.now()+900_000).toISOString() })
    }) as typeof fetch
    const pendingA = a.session(key)
    await startedA
    let timeout: ReturnType<typeof setTimeout> | undefined
    try {
      await Promise.race([b.session(key), new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error("independent profile was blocked")), 1_000) })])
    } finally { if (timeout) clearTimeout(timeout); release(); await pendingA }
  } finally { release(); globalThis.fetch = before; await rm(root, { recursive: true, force: true }) }
})


test("lost reply recovery persists an expired successor before rotating again, including another restart", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-client-refresh-expired-")), before = globalThis.fetch
  try {
    const store = new CloudClientCredentialStore(join(root, "client.json"))
    await store.saveLogin(credential())
    let firstRotation: string | undefined, secondRotation: string | undefined, calls = 0
    globalThis.fetch = (async (_url, options) => {
      const body = JSON.parse(options!.body as string)
      calls++
      if (calls === 1) { firstRotation = body.rotationId; throw new Error("fixture first lost reply") }
      if (calls === 2) {
        assert.equal(body.rotationId, firstRotation)
        assert.equal(body.refreshCredential, "synthetic-refresh")
        return Response.json({ refreshCredential: "synthetic-successor", cloudSessionToken: "synthetic-expired", cloudSessionExpiresAt: new Date(Date.now()-60_000).toISOString() })
      }
      assert.equal(body.refreshCredential, "synthetic-successor")
      assert.notEqual(body.rotationId, firstRotation)
      const durable = await store.load()
      assert.equal(durable!.refreshCredential, "synthetic-successor")
      assert.equal(durable!.pendingRotationId, body.rotationId)
      if (calls === 3) { secondRotation = body.rotationId; throw new Error("fixture second lost reply") }
      assert.equal(body.rotationId, secondRotation)
      return Response.json({ refreshCredential: "synthetic-fresh", cloudSessionToken: "synthetic-fresh-access", cloudSessionExpiresAt: new Date(Date.now()+900_000).toISOString() })
    }) as typeof fetch
    await assert.rejects(store.session(key), /first lost reply/)
    await assert.rejects(new CloudClientCredentialStore(store.filePath).session(key), /second lost reply/)
    const recovered = await new CloudClientCredentialStore(store.filePath).session(key)
    assert.equal(recovered.accessToken, "synthetic-fresh-access")
    assert.equal(recovered.pendingRotationId, undefined)
    assert.equal(calls, 4)
  } finally { globalThis.fetch = before; await rm(root, { recursive: true, force: true }) }
})
