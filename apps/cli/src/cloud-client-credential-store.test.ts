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

// MP-08 / MP-11: restart, concurrent profile rotation, and collaboration
// persistence share one private file/lock; public profile authority is separate.
test("MP-08 / MP-11: concurrent scope writes merge with refresh and survive profile reload/login resume", async t => {
  const root = await mkdtemp(join(tmpdir(), "chariox-client-scope-"))
  t.after(() => rm(root, {recursive: true, force: true}))
  const store = new CloudClientCredentialStore(join(root, "client.json")), other = new CloudClientCredentialStore(store.filePath)
  await store.saveLogin(credential())
  const original = (await store.load())!
  t.mock.method(globalThis, "fetch", async () => {
    await new Promise(resolve => setTimeout(resolve, 25))
    return Response.json({refreshCredential: "synthetic-next", cloudSessionToken: "synthetic-next-access", cloudSessionExpiresAt: new Date(Date.now()+900_000).toISOString()})
  })
  await Promise.all([
    store.session(key),
    store.rememberSessionScope(original, {sessionId: "shared", accountId: "owner-A"}),
    other.rememberSessionScope(original, {sessionId: "second", accountId: "owner-C"}),
  ])
  await other.rememberSessionScope(original, {sessionId: "shared", accountId: "personal"})
  await other.rememberSessionScope(original, {sessionId: "shared", accountId: "owner-A"})
  let current = (await new CloudClientCredentialStore(store.filePath).load())!
  assert.equal(current.accessToken, "synthetic-next-access", "stale acceptance must not overwrite refresh")
  assert.equal(current.loginId, original.loginId)
  assert.equal(current.collaborationScopes!.length, 3, "repeat hints replace only the exact caller/session/account tuple")
  assert.deepEqual(current.collaborationScopes!.filter(scope => scope.sessionId === "shared").map(scope => scope.accountId), ["personal", "owner-A"])
  await store.saveLogin({...current, collaborationScopes: []})
  current = (await store.load())!
  assert.equal(current.collaborationScopes!.length, 3)
  assert.equal((await stat(store.filePath)).mode & 0o777, 0o600)
  t.mock.method(globalThis, "fetch", async () => Response.json({error: {code: "client_revoked"}}, {status: 401}))
  await assert.rejects(store.session(key, true), /client_revoked/)
  assert.equal(await store.load(), null, "revocation removes private hints with authority")
})

test("MP-08 / MP-11: scope persistence rejects stale callers after logout, relogin or profile replacement", async t => {
  const root = await mkdtemp(join(tmpdir(), "chariox-client-scope-isolation-"))
  t.after(() => rm(root, {recursive: true, force: true}))
  const store = new CloudClientCredentialStore(join(root, "client.json"))
  await store.saveLogin(credential())
  const original = (await store.load())!
  await store.rememberSessionScope(original, {sessionId: "shared", accountId: "owner"})
  const saved = (await store.load())!
  for (const changed of [
    {...saved, profile: {...saved.profile, apiUrl: "https://foreign.example.test"}},
    {...saved, profile: {...saved.profile, userId: "foreign"}},
    {...saved, profile: {...saved.profile, accountId: "foreign"}},
    {...saved, clientId: "foreign"},
    {...saved, publicKeyThumbprint: "b".repeat(64)},
  ]) {
    await assert.rejects(store.rememberSessionScope(changed, {sessionId: "shared", accountId: "foreign"}), /profile_conflict/)
    await assert.rejects(store.saveLogin(changed), /profile_conflict/)
  }
  await store.clear()
  await assert.rejects(store.rememberSessionScope(original, {sessionId: "shared", accountId: "owner"}), /login_required/)
  await store.saveLogin(credential())
  assert.equal((await store.load())!.collaborationScopes, undefined)
  await assert.rejects(store.rememberSessionScope(original, {sessionId: "shared", accountId: "owner"}), /profile_conflict/, "a new login family cannot inherit a late acceptance from the revoked family")
})

test("MP-08 / MP-11: oversized scope writes fail without corrupting the readable private profile", async t => {
  const root = await mkdtemp(join(tmpdir(), "chariox-client-scope-bound-"))
  t.after(() => rm(root, {recursive: true, force: true}))
  const store = new CloudClientCredentialStore(join(root, "client.json"))
  await store.saveLogin(credential())
  const original = (await store.load())!
  await assert.rejects(store.rememberSessionScope(original, {sessionId: "shared", accountId: "x".repeat(64*1024)}), /credential_file_too_large/)
  assert.equal((await store.load())!.collaborationScopes, undefined)
})
