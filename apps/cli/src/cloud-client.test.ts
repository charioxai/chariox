import assert from "node:assert/strict"
import { createServer } from "node:http"
import { mkdtemp, readFile, rm, stat } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import WebSocket, { WebSocketServer } from "ws"
import { createECDH, createHash } from "node:crypto"
import { RelayClientIdentity } from "./ipc.js"
import { createCliRelayIdentityStore } from "./cli-relay-identity-store.js"
import { CloudClient, cloudDirectoryProjection, issueCloudPairingBootstrapToken } from "./cloud-client.js"
import { CloudClientCredentialStore } from "./cloud-client-credential-store.js"
import { CloudClientAuthError } from "./cloud-client-http.js"

type Family = { clientId: string; key: string; access: string; refresh: string; revoked: boolean }
// Renewal includes the real OS profile lock and synchronized credential writes.
// The disk path can exceed the 500 ms renewal window of a two-second grant
// on shared builders. Keep accelerated expiries and the same seven-rotation
// assertions, while allowing two seconds for serialized profile writes.
const ACCESS_LIFETIME_MS = 8_400
const GRANT_LIFETIME_MS = 6_000
async function fixture() {
  const key = createECDH("prime256v1"); key.generateKeys()
  const daemon = new RelayClientIdentity(key.getPrivateKey())
  const relay = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => relay.once("listening", resolve))
  const address = relay.address()
  assert.ok(address && typeof address === "object")
  const relayUrl = `ws://127.0.0.1:${address.port}`
  const families = new Map<string, Family>(), devices = new Map<string, Family>()
  const grants = new Map<string, { family: Family; target: string; expiry: number }>()
  const socketFamilies = new Map<WebSocket, Family>(), expiries = new Map<WebSocket, ReturnType<typeof setTimeout>>()
  const streams = new Set<ReturnType<typeof setInterval>>()
  const metrics = { starts: 0, refreshes: 0, grants: 0, connections: 0, subscriptions: 0, logouts: 0, requests: 0 }
  const paths: string[] = [], errors: unknown[] = []
  let sequence = 0, wrongKey = false, failLogout = false, invalidEnrollment = false
  const targets = [
    { daemonId: "remote-a", daemonAlias: "Work", machineId: "host-a", machineAlias: "Laptop", status: "ONLINE" as const },
    { daemonId: "remote-b", daemonAlias: "Build", machineId: "host-b", machineAlias: "Builder", status: "ONLINE" as const },
    { daemonId: "remote-stale", machineId: "host-b", status: "STALE" as const },
  ]
  const rotate = (family: Family) => { family.access = `synthetic-access-${++sequence}`; family.refresh = `synthetic-refresh-${sequence}` }
  const active = (access: unknown) => [...families.values()].find(f => f.access === access && !f.revoked)
  const revoke = (family: Family) => {
    family.revoked = true
    for (const [socket, owner] of socketFamilies) if (owner === family && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ kind: "close", reason: "relay token revoked" }))
      socket.close()
    }
  }
  const api = createServer((request, response) => {
    void (async () => {
      const path = new URL(request.url!, "http://localhost").pathname
      paths.push(path)
      const chunks: Buffer[] = []
      for await (const chunk of request) chunks.push(Buffer.from(chunk))
      const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : {}
      const send = (value: unknown, status = 200) => { response.writeHead(status, { "content-type": "application/json" }); response.end(JSON.stringify(value)) }
      const deny = (code: string) => send({ error: { code } }, 401)
      if (path === "/auth/device/start") {
        assert.equal(body.enrollmentKind, "CLIENT")
        assert.equal(body.machineId, undefined); assert.equal(body.kernelId, undefined)
        assert.ok(/^[0-9a-f]{64}$/.test(body.publicKeyThumbprint))
        const family: Family = { clientId: body.clientId, key: body.publicKeyThumbprint, access: "", refresh: "", revoked: false }
        const device = `synthetic-device-${++metrics.starts}`
        devices.set(device, family); families.set(family.clientId, family)
        send({ deviceCode: device, userCode: "PUBLIC-CODE", verificationUrl: "https://cloud.example.test/activate?user_code=PUBLIC-CODE", intervalSeconds: 1, expiresAt: new Date(Date.now()+60_000).toISOString() })
      } else if (path === "/auth/device/poll") {
        const family = devices.get(body.deviceCode)!
        rotate(family)
        send({ status: "approved", profile: { enrollmentKind: invalidEnrollment ? "KERNEL" : "CLIENT", publicKeyThumbprint: family.key, accountId: "account-a", userId: "owner-a", email: "fixture@example.test", accountSlug: "fixture", realmId: "realm-a", relayUrl, issuerId: "fixture", clientId: family.clientId }, refreshCredential: family.refresh, cloudSessionToken: family.access, cloudSessionExpiresAt: new Date(Date.now()+ACCESS_LIFETIME_MS).toISOString() })
      } else if (path === "/auth/client/refresh") {
        const family = families.get(body.clientId)
        if (!family || family.revoked) return deny("client_revoked")
        assert.ok(family.refresh === body.refreshCredential, "one profile serializes current-family rotation")
        assert.equal(body.publicKeyThumbprint, family.key)
        assert.ok(/^[0-9a-f]{64}$/.test(body.rotationId))
        metrics.refreshes++; rotate(family)
        send({ refreshCredential: family.refresh, cloudSessionToken: family.access, cloudSessionExpiresAt: new Date(Date.now()+ACCESS_LIFETIME_MS).toISOString() })
      } else if (path === "/relay/targets") {
        if (!active(request.headers.authorization?.slice("Bearer ".length))) return deny("session_invalid")
        send({ targets })
      } else if (path === "/relay/token") {
        const family = active(body.sessionToken)
        if (!family) return deny("session_invalid")
        assert.equal(body.subjectKind, "client"); assert.equal(body.subject, family.clientId)
        assert.equal(body.machineId, undefined); assert.equal(body.publicKeyThumbprint, family.key)
        assert.ok(body.allowedTargets.length === 1 && targets.some(t => t.daemonId === body.allowedTargets[0] && t.status === "ONLINE"))
        const expiry = Date.now()+GRANT_LIFETIME_MS
        const payload = { public_key_thumbprint: wrongKey ? "b".repeat(64) : family.key, allowed_targets: body.allowedTargets, jti: `grant-${++metrics.grants}` }
        const token = `fixture.${Buffer.from(JSON.stringify(payload)).toString("base64url")}.fixture`
        grants.set(token, { family, target: body.allowedTargets[0], expiry })
        send({ token, expiresAt: new Date(expiry).toISOString() })
      } else if (path === "/auth/logout") {
        if (failLogout) return send({ error: { code: "fixture_outage" } }, 503)
        const family = active(body.sessionToken)
        if (!family) return deny("session_invalid")
        assert.equal(body.clientId, family.clientId); assert.equal(body.revokeClient, true); assert.equal(body.revokeMachine, undefined)
        metrics.logouts++; revoke(family); send(null)
      } else throw new Error("unexpected Cloud runtime/proxy path")
    })().catch(error => { errors.push(error); response.writeHead(500); response.end() })
  })
  await new Promise<void>(resolve => api.listen(0, "127.0.0.1", resolve))
  const apiAddress = api.address(); assert.ok(apiAddress && typeof apiAddress === "object")
  relay.on("connection", socket => {
    metrics.connections++
    socket.on("message", raw => {
      try {
        const frame = JSON.parse(String(raw))
        if (frame.kind === "client_connect") {
          const grant = grants.get(frame.auth_token)
          assert.ok(grant && !grant.family.revoked && grant.expiry > Date.now())
          assert.equal(frame.target.daemon_id, grant.target)
          clearTimeout(expiries.get(socket))
          expiries.set(socket, setTimeout(() => socket.close(), Math.max(0, grant.expiry-Date.now())))
          socketFamilies.set(socket, grant.family)
          socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
        } else if (frame.kind === "client_request") {
          assert.equal(createHash("sha256").update(frame.encrypted_request.sender_public_key).digest("hex"), socketFamilies.get(socket)!.key)
          daemon.decrypt(frame.encrypted_request)
          metrics.requests++
          socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, encrypted_response: daemon.encrypt(frame.encrypted_request.sender_public_key, JSON.stringify({ accepted: true })), error: null }))
        } else if (frame.kind === "client_subscribe") {
          assert.equal(createHash("sha256").update(frame.client_public_key).digest("hex"), socketFamilies.get(socket)!.key)
          metrics.subscriptions++
          socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, encrypted_response: daemon.encrypt(frame.client_public_key, "null"), error: null }))
          let eventId = 0
          const stream = setInterval(() => { if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ kind: "client_event", subscription_id: frame.subscription_id, event_id: ++eventId, encrypted_event: daemon.encrypt(frame.client_public_key, JSON.stringify({ event: "runtime_notices", notices: [{ message: "continuing" }] })) })) }, 30)
          streams.add(stream); socket.once("close", () => clearInterval(stream))
        }
      } catch (error) { errors.push(error); socket.terminate() }
    })
  })
  return { apiUrl: `http://127.0.0.1:${apiAddress.port}`, metrics, paths, families,
    setInvalidEnrollment: () => { invalidEnrollment = true }, setWrongKey: () => { wrongKey = true }, setFailLogout: (value: boolean) => { failLogout = value }, revoke,
    async close() {
      for (const timer of expiries.values()) clearTimeout(timer)
      for (const timer of streams) clearInterval(timer)
      for (const socket of relay.clients) socket.terminate()
      await new Promise<void>(resolve => relay.close(() => resolve()))
      await new Promise<void>(resolve => api.close(() => resolve()))
      assert.equal(errors.length, 0, "fixture must preserve the Cloud/relay contract")
    },
  }
}

function profile(root: string) {
  const identity = createCliRelayIdentityStore(join(root, "relay", "identity.json"))
  const store = new CloudClientCredentialStore(join(root, "relay", "cloud-client.json"))
  return { store, client: new CloudClient(store, () => identity.getOrCreate()) }
}
const authCode = (code: string) => (error: unknown) => error instanceof CloudClientAuthError && error.code === code

test("detached client-only login survives process resume and several access/grant expiries without reconnect or re-login", async () => {
  const root = await mkdtemp(join(tmpdir(), "kauth-detached-")), cloud = await fixture()
  const first = profile(root), notices: string[] = []
  const resumed = profile(root)
  try {
    const publicProfile = await first.client.login(cloud.apiUrl, verification => { notices.push(verification.userCode); assert.ok(!("deviceCode" in verification)) })
    assert.equal(publicProfile.machineId, undefined); assert.equal(publicProfile.kernelId, undefined)
    assert.equal((await stat(first.store.filePath)).mode & 0o777, 0o600)
    assert.ok(!(await readFile(first.store.filePath, "utf8")).includes('"machineCredential"'))
    first.client.stop()
    let revoked = 0
    await resumed.client.resume(() => { revoked++ })
    const again = await resumed.client.login(cloud.apiUrl, () => { throw new Error("profile resume requested another sign-in") })
    assert.equal(again.clientId, publicProfile.clientId)
    assert.equal(cloud.metrics.starts, 1)
    const inventory = cloudDirectoryProjection(await resumed.client.directory())
    assert.equal(inventory.machines.length, 2); assert.equal(inventory.kernels.length, 2)
    assert.ok(!inventory.kernels.some(k => k.kernel_id === "remote-stale"))
    await assert.rejects(resumed.client.connect("remote-stale"), authCode("kernel_offline"))
    const target = await resumed.client.connect("remote-a")
    let events = 0
    target.onKernelEvent(event => { if (event.event === "runtime_notices") events++ })
    await target.subscribeToKernelEvents("session-fixture", "attachment-fixture")
    const deadline = Date.now()+48_000
    while (Date.now() < deadline) {
      assert.deepEqual(await target.send({ GetDaemonHealth: null }), { accepted: true })
      await new Promise(resolve => setTimeout(resolve, 35))
    }
    assert.ok(cloud.metrics.grants >= 7); assert.ok(cloud.metrics.refreshes >= 7); assert.ok(events > 200)
    assert.equal(cloud.metrics.connections, 2); assert.equal(cloud.metrics.subscriptions, 1); assert.equal(notices.length, 1)
    assert.ok(cloud.paths.every(path => ["/auth/device/start", "/auth/device/poll", "/auth/client/refresh", "/relay/targets", "/relay/token"].includes(path)), "Cloud carries bootstrap only")
    cloud.revoke(cloud.families.get(publicProfile.clientId!)!)
    await assert.rejects(resumed.client.directory(), authCode("client_revoked"))
    assert.equal(await resumed.store.load(), null); assert.equal(revoked, 1)
    await assert.rejects(target.send({ GetDaemonHealth: null }))
    await new Promise(resolve => setTimeout(resolve, 100))
    const stopped = events
    await new Promise(resolve => setTimeout(resolve, 100))
    assert.equal(events, stopped); assert.equal(cloud.metrics.connections, 2)
  } finally { first.client.stop(); resumed.client.stop(); await cloud.close(); await rm(root, { recursive: true, force: true }) }
})

test("MP-08 / MP-11: background client revocation remains explicit on later control requests", async () => {
  const root = await mkdtemp(join(tmpdir(), "kauth-background-revocation-")), cloud = await fixture()
  const terminal = profile(root)
  let deadline: ReturnType<typeof setTimeout> | undefined
  try {
    const linked = await terminal.client.login(cloud.apiUrl, () => {})
    let revoked!: () => void
    const observed = new Promise<void>(resolve => { revoked = resolve })
    await terminal.client.resume(revoked)
    cloud.revoke(cloud.families.get(linked.clientId!)!)
    await Promise.race([
      observed,
      new Promise<never>((_, reject) => { deadline = setTimeout(() => reject(new Error("background revocation was not observed")), 5_000) }),
    ])
    assert.equal(await terminal.store.load(), null)
    await assert.rejects(terminal.client.directory(), authCode("client_revoked"))
    await assert.rejects(terminal.client.issue("remote-a"), authCode("client_revoked"))
  } finally {
    clearTimeout(deadline)
    terminal.client.stop()
    await cloud.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("MP-08 / MP-11: missing active client authority settles revocation before the background callback", async () => {
  const root = await mkdtemp(join(tmpdir(), "kauth-revocation-race-")), cloud = await fixture()
  const terminal = profile(root)
  let revoked = 0
  try {
    await terminal.client.login(cloud.apiUrl, () => {})
    await terminal.client.resume(() => { revoked++ })
    // Another profile process can remove the credential while this process
    // still owns admitted work and its background refresh is settling.
    await terminal.store.clear()
    await assert.rejects(terminal.client.directory(), authCode("client_revoked"))
    await assert.rejects(terminal.client.issue("remote-a"), authCode("client_revoked"))
    assert.equal(revoked, 1)
  } finally {
    terminal.client.stop()
    await cloud.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("detached profiles are independent; logout waits for acknowledgement and cannot revoke kernels", async () => {
  const root = await mkdtemp(join(tmpdir(), "kauth-detached-profiles-")), cloud = await fixture()
  const a = profile(join(root, "a")), b = profile(join(root, "b"))
  try {
    const first = await a.client.login(cloud.apiUrl, () => {})
    const second = await b.client.login(cloud.apiUrl, () => {})
    assert.notEqual(first.clientId, second.clientId)
    assert.equal(cloud.families.size, 2)
    cloud.setFailLogout(true)
    await assert.rejects(a.client.logout(), authCode("fixture_outage"))
    assert.ok(await a.store.load())
    cloud.setFailLogout(false)
    await a.client.logout()
    assert.equal(await a.store.load(), null); assert.ok(await b.store.load())
    assert.equal(cloud.metrics.logouts, 1)
    assert.ok(cloud.families.get(first.clientId!)!.revoked && !cloud.families.get(second.clientId!)!.revoked)
    await b.client.directory()
  } finally { a.client.stop(); b.client.stop(); await cloud.close(); await rm(root, { recursive: true, force: true }) }
})

test("detached bootstrap denies profile account conflicts and grants bound to another key", async () => {
  const root = await mkdtemp(join(tmpdir(), "kauth-detached-negative-")), cloud = await fixture()
  const a = profile(join(root, "a")), conflicting = profile(join(root, "conflict")), invalid = profile(join(root, "invalid"))
  try {
    await assert.rejects(conflicting.client.login(cloud.apiUrl, () => {}, "other-account"), authCode("profile_conflict"))
    assert.equal(await conflicting.store.load(), null); assert.equal(cloud.metrics.logouts, 1)
    await a.client.login(cloud.apiUrl, () => {})
    await assert.rejects(a.client.login("https://other-cloud.example.test", () => {}), authCode("profile_conflict"))
    cloud.setWrongKey()
    await assert.rejects(a.client.connect("remote-a"), /did not bind the token/)
    assert.equal(cloud.metrics.connections, 0)
    cloud.setInvalidEnrollment()
    await assert.rejects(invalid.client.login(cloud.apiUrl, () => {}), authCode("invalid_client_enrollment"))
    assert.equal(await invalid.store.load(), null)
  } finally { a.client.stop(); conflicting.client.stop(); invalid.client.stop(); await cloud.close(); await rm(root, { recursive: true, force: true }) }
})


test("Cloud pairing bootstrap uses the receiving client profile and refuses a foreign relay before issuance", async () => {
  const root = await mkdtemp(join(tmpdir(), "kauth-pairing-bootstrap-")), cloud = await fixture()
  const previous = process.env.CHARIOX_HOME
  process.env.CHARIOX_HOME = root
  const identity = createCliRelayIdentityStore().getOrCreate()
  const client = new CloudClient()
  try {
    await assert.rejects(issueCloudPairingBootstrapToken("ws://fixture", "remote-a", identity), /signed-in terminal/)
    const publicProfile = await client.login(cloud.apiUrl, () => {})
    await assert.rejects(issueCloudPairingBootstrapToken("wss://foreign.invalid", "remote-a", identity), authCode("profile_conflict"))
    assert.equal(cloud.metrics.grants, 0)
    const token = await issueCloudPairingBootstrapToken(publicProfile.relayUrl!, "remote-a", identity)
    const encoded = token.split(".")[1]
    assert.ok(encoded)
    const payload = JSON.parse(Buffer.from(encoded, "base64url").toString())
    assert.equal(payload.public_key_thumbprint, identity.publicKeyThumbprint)
    assert.equal(cloud.metrics.grants, 1)
    assert.equal(cloud.metrics.starts, 1)
  } finally {
    client.stop()
    if (previous === undefined) delete process.env.CHARIOX_HOME; else process.env.CHARIOX_HOME = previous
    await cloud.close(); await rm(root, { recursive: true, force: true })
  }
})
