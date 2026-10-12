import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { createServer } from "node:http"
import { mkdtemp, rm, stat } from "node:fs/promises"
import { tmpdir } from "node:os"
import path from "node:path"
import test from "node:test"
import WebSocket, { WebSocketServer } from "ws"
import {
  assert as drillAssert,
  connectSessionScopedCloudClient,
  createCloudDrillClient,
  createMinimalCommandDeps,
  loadCloudRelayDrillModules,
  loginCloudDrillUser,
  terminateChild,
  waitForCloudRelayTarget,
} from "./live-cloud-relay-drill-helpers.mjs"

const modules = await loadCloudRelayDrillModules()

async function fixture(root) {
  const daemon = modules.createCliRelayIdentityStore(path.join(root, "fixture-daemon", "identity.json")).getOrCreate()
  const relay = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise(resolve => relay.once("listening", resolve))
  const relayUrl = `ws://127.0.0.1:${relay.address().port}`
  const devices = new Map(), grants = new Map(), sockets = new Map(), timers = new Set()
  const members = [{ accountId: "account-owner", sessionId: "session-a", userId: "user-owner", email: "owner@example.com" }]
  const calls = [], errors = [], contacts = []
  let grantNumber = 0, inviteUses = 0, inviteMaxUses = 0
  const profile = (accountSlug, family) => ({
    enrollmentKind: "CLIENT", publicKeyThumbprint: family.key, clientId: family.id,
    accountId: `account-${accountSlug}`, userId: `user-${accountSlug}`, accountSlug,
    email: `${accountSlug}@example.com`, realmId: `realm-${accountSlug}`, relayUrl, issuerId: "fixture",
  })
  const server = createServer((req, res) => {
    void (async () => {
      const chunks = []
      for await (const chunk of req) chunks.push(chunk)
      const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : {}
      const pathname = new URL(req.url, "http://localhost").pathname
      calls.push(pathname)
      const send = (value, status = 200) => { res.writeHead(status, { "content-type": "application/json" }); res.end(JSON.stringify(value)) }
      if (pathname === "/auth/device/start") {
        assert.equal(body.enrollmentKind, "CLIENT")
        assert.equal(body.machineId, undefined); assert.equal(body.kernelId, undefined)
        assert.match(body.publicKeyThumbprint, /^[0-9a-f]{64}$/)
        const id = `device-${devices.size}`, family = { id: body.clientId, key: body.publicKeyThumbprint }
        devices.set(id, family)
        send({ deviceCode: id, userCode: id, expiresAt: new Date(Date.now()+60_000).toISOString(), intervalSeconds: 1, verificationUrl: `${apiUrl}/activate?user_code=${id}` })
      } else if (pathname === "/auth/dev/device/approve") {
        assert.ok(req.headers["x-chariox-dev-auth-secret"], "fixture device approval is authenticated")
        const family = devices.get(body.userCode)
        if (family) { family.accountSlug = body.accountSlug; family.access = `synthetic-access-${body.userCode}` }
        else assert.equal(body.userCode, "kernel-code")
        send({ approved: true })
      } else if (pathname === "/auth/device/poll") {
        const family = devices.get(body.deviceCode)
        assert.ok(family?.accountSlug)
        send({ status: "approved", profile: profile(family.accountSlug, family), cloudSessionToken: family.access, refreshCredential: "synthetic-refresh", cloudSessionExpiresAt: new Date(Date.now()+900_000).toISOString() })
      } else if (pathname === "/relay/targets") {
        assert.ok([...devices.values()].some(f => `Bearer ${f.access}` === req.headers.authorization))
        send({ targets: [{ daemonId: "kernel-a", machineId: "machine-a", status: "ONLINE" }] })
      } else if (pathname === "/relay/token") {
        const family = [...devices.values()].find(f => f.access === body.sessionToken)
        assert.ok(family, "grant requires a terminal session")
        assert.equal(body.subjectKind, "client"); assert.equal(body.subject, family.id)
        assert.equal(body.clientId, family.id); assert.equal(body.publicKeyThumbprint, family.key)
        assert.equal(body.kernelCredential, undefined); assert.equal(body.machineCredential, undefined)
        assert.deepEqual(body.allowedTargets, ["kernel-a"])
        assert.equal(body.accountId, "account-owner"); assert.equal(body.realmId, "realm-owner")
        assert.equal(body.ttlMs, 300_000)
        if (body.sessionId) assert.equal(body.sessionId, "session-a")
        const expiry = Date.now()+300
        const token = `fixture.${Buffer.from(JSON.stringify({ public_key_thumbprint: family.key, session_id: body.sessionId, allowed_targets: body.allowedTargets, jti: `grant-${++grantNumber}` })).toString("base64url")}.fixture`
        grants.set(token, { key: family.key, expiry, sessionId: body.sessionId })
        send({ token, expiresAt: new Date(expiry).toISOString() })
      } else if (pathname === "/sessions/invites") {
        assert.ok([...devices.values()].some(f => f.access === body.sessionToken))
        assert.equal(body.sessionId, "session-a")
        inviteMaxUses = body.maxUses
        send({ inviteId: "invite-a", inviteToken: "synthetic-invite", sessionId: "session-a", accountId: "account-owner", createdByUserId: "user-owner" })
      } else if (pathname === "/sessions/invites/synthetic-invite/accept") {
        const family = [...devices.values()].find(f => f.access === body.sessionToken)
        assert.ok(family)
        if (inviteUses >= inviteMaxUses) {send({error: {code: "session_invite_invalid"}}, 403); return}
        inviteUses++
        for (const [userId, collaboratorUserId] of [["user-owner", `user-${family.accountSlug}`], [`user-${family.accountSlug}`, "user-owner"]]) {
          contacts.push({accountId: "account-owner", userId, collaboratorUserId, hiddenAt: null, lastCollaboratedAt: new Date().toISOString(), sharedSessionCount: 1})
        }
        // MP-08 / MP-11: Cloud 3620f1245 records invite.accountId, not caller.accountId.
        members.push({ accountId: "account-owner", sessionId: "session-a", userId: `user-${family.accountSlug}`, email: `${family.accountSlug}@example.com` })
        send({ sessionId: "session-a", accountId: "account-owner", userId: `user-${family.accountSlug}`, invitedByUserId: "user-owner", joinedAt: new Date().toISOString() })
      } else if (pathname === "/sessions/members") {
        const family = [...devices.values()].find(f => `Bearer ${f.access}` === req.headers.authorization)
        assert.ok(family)
        const query = new URL(req.url, "http://localhost").searchParams
        const accountId = query.get("accountId"), sessionId = query.get("sessionId")
        // Match requireSharedSessionMember's exact (accountId, sessionId, caller.userId).
        if (!members.some(m => m.accountId === accountId && m.sessionId === sessionId && m.userId === `user-${family.accountSlug}`)) {
          send({ error: { code: "session_invite_invalid" } }, 403)
          return
        }
        send({ sessionId, members: members.filter(m => m.accountId === accountId && m.sessionId === sessionId) })
      } else if (pathname === "/collaborators/recent") {
        const family = [...devices.values()].find(f => `Bearer ${f.access}` === req.headers.authorization)
        assert.ok(family)
        const accountId = new URL(req.url, "http://localhost").searchParams.get("accountId")
        send({collaborators: contacts.filter(c => c.accountId === accountId && c.userId === `user-${family.accountSlug}` && c.hiddenAt === null)
          .sort((a, b) => Date.parse(b.lastCollaboratedAt)-Date.parse(a.lastCollaboratedAt)).slice(0, 25)
          .map(c => ({userId: c.collaboratorUserId, email: `${c.collaboratorUserId.slice(5)}@example.com`, lastCollaboratedAt: c.lastCollaboratedAt, sharedSessionCount: c.sharedSessionCount}))})
      } else throw new Error("unexpected fixture route")
    })().catch(error => { errors.push(error); res.writeHead(500); res.end() })
  })
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve))
  const apiUrl = `http://127.0.0.1:${server.address().port}`
  relay.on("connection", socket => {
    socket.on("message", raw => {
      try {
        const frame = JSON.parse(String(raw))
        if (frame.kind === "client_connect") {
          const grant = grants.get(frame.auth_token)
          assert.ok(grant && grant.expiry > Date.now())
          assert.equal(frame.target.daemon_id, "kernel-a")
          clearTimeout(sockets.get(socket)?.expiry)
          const expiry = setTimeout(() => socket.close(), grant.expiry-Date.now())
          timers.add(expiry); sockets.set(socket, { grant, expiry })
          socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
        } else if (frame.kind === "client_request") {
          const sender = frame.encrypted_request.sender_public_key
          assert.equal(createHash("sha256").update(sender).digest("hex"), sockets.get(socket).grant.key)
          daemon.decrypt(frame.encrypted_request)
          socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, encrypted_response: daemon.encrypt(sender, JSON.stringify({ accepted: true })), error: null }))
        } else if (frame.kind === "client_subscribe") {
          assert.equal(frame.session_id, "session-a")
          socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, encrypted_response: daemon.encrypt(frame.client_public_key, "null"), error: null }))
          const timer = setInterval(() => {
            if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ kind: "client_event", subscription_id: frame.subscription_id, event_id: Date.now(), encrypted_event: daemon.encrypt(frame.client_public_key, JSON.stringify({ event: "runtime_notices", notices: [{ message: "fixture-event" }] })) }))
          }, 30)
          timers.add(timer); socket.once("close", () => clearInterval(timer))
        }
      } catch (error) { errors.push(error); socket.terminate() }
    })
  })
  const kernelProfile = { api_url: apiUrl, email: "owner@example.com", account_id: "account-owner", user_id: "user-owner", account_slug: "owner", realm_id: "realm-owner", relay_url: relayUrl, issuer_id: "fixture", machine_id: "machine-a", kernel_id: "kernel-a", kernel_enrolled: true }
  const status = { daemon_id: "kernel-a", machine_id: "machine-a", connected: true, configured: true, relay_url: relayUrl }
  const kernelRequests = []
  const localClient = { async send(request) {
    const variant = Object.keys(request)[0]; kernelRequests.push(variant)
    switch (variant) {
      case "RelayStatus": return { RelayStatus: { status } }
      case "CloudRelayStatus": return { CloudRelayStatus: { profile: kernelProfile } }
      case "StartCloudRelayLogin": return { CloudRelayLoginStarted: { login: { api_url: apiUrl, device_code: "kernel-device", user_code: "kernel-code", verification_url: `${apiUrl}/activate?user_code=kernel-code`, expires_at: new Date(Date.now()+60_000).toISOString(), interval_seconds: 1 } } }
      case "PollCloudRelayLogin": return { CloudRelayLoginPolled: { result: { status: "approved", profile: kernelProfile } } }
      case "ConnectCloudRelay": return { CloudRelayConnected: { profile: kernelProfile, status } }
      default: throw new Error("unexpected kernel authority path")
    }
  } }
  return { apiUrl, calls, kernelRequests, localClient,
    grantCount: () => grantNumber,
    async close() {
      for (const timer of timers) clearTimeout(timer)
      for (const socket of relay.clients) socket.terminate()
      await new Promise(resolve => relay.close(resolve))
      await new Promise(resolve => server.close(resolve))
      assert.equal(errors.length, 0, "Cloud and relay fixture errors must be absent")
    },
  }
}

test("MP-08 / MP-10 / MP-11: drill composes kernel enrollment and private human login, collaboration and key-bound relay renewal", async t => {
  const root = await mkdtemp(path.join(tmpdir(), "kauth-r8-fixture-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const cloud = await fixture(root)
  const terminal = createCloudDrillClient(modules, root, "owner")
  const terminals = [terminal], clients = []
  const profileRef = { current: null }, notices = []
  try {
    const deps = createMinimalCommandDeps({ apiUrl: cloud.apiUrl, runId: "owner", workspace: root, localClient: cloud.localClient, modules, terminal, profileRef, notices })
    const handlers = modules.createCommandActionHandlers(deps)
    await handlers.handleCloudCommand({ kind: "cloud", raw: "/cloud link", args: ["link"] })
    assert.equal(profileRef.current.kernelEnrolled, true)
    assert.equal(profileRef.current.kernelId, "kernel-a")
    await handlers.handleCloudCommand({ kind: "cloud", raw: "/cloud login", args: ["login"] })
    assert.equal(profileRef.current.kernelId, "kernel-a", "terminal login cannot replace the kernel profile")
    assert.equal((await terminal.client.profile()).machineId, undefined)
    assert.equal((await stat(terminal.client.store.filePath)).mode & 0o777, 0o600)
    const online = await waitForCloudRelayTarget(terminal.client, { daemonId: "kernel-a", status: "ONLINE" })
    assert.equal(online.machineId, "machine-a")
    const detached = await terminal.client.connect("kernel-a"); clients.push(detached)
    assert.deepEqual(await detached.send({ GetDaemonHealth: null }), { accepted: true })
    const invitation = await deps.createCloudSessionInvite("session-a", { maxUses: 1 })
    let peer = await loginCloudDrillUser(cloud.apiUrl, { modules, stateRoot: root, name: "peer", email: "peer@example.com", accountSlug: "peer" })
    terminals.push(peer)
    assert.equal((await peer.client.collaboration.acceptSessionInvite(invitation.invite.invite_token)).acceptance.user_id, "user-peer")
    assert.equal((await deps.listCloudSessionMembers("session-a")).members.length, 2)
    assert.equal((await peer.client.collaboration.sessionMembers("session-a")).members.length, 2)
    assert.equal((await peer.client.profile()).accountId, "account-peer", "shared session scope cannot replace the caller's login account")
    assert.equal((await peer.client.collaboration.collaborators())[0]?.user_id, "user-owner")
    await assert.rejects(peer.client.collaboration.acceptSessionInvite(invitation.invite.invite_token), /session_invite_invalid/)
    // MP-08 / MP-10 / MP-11: same persisted identity/profile, fresh CloudClient.
    peer.client.stop()
    peer = createCloudDrillClient(modules, root, "peer")
    terminals.push(peer)
    await peer.client.resume()
    const scoped = await connectSessionScopedCloudClient(modules, peer, { accountId: "account-owner", realmId: "realm-owner", sessionId: "session-a", targetDaemonId: "kernel-a" })
    clients.push(scoped)
    assert.deepEqual(await scoped.send({ AttachToSession: {session_id: "session-a", client_id: "peer-restarted"} }), {accepted: true})
    assert.equal((await peer.client.collaboration.sessionMembers("session-a")).members.length, 2)
    assert.equal((await peer.client.collaboration.collaborators())[0]?.user_id, "user-owner")
    let events = 0
    scoped.onKernelEvent(() => { events++ })
    await scoped.subscribeToKernelEvents("session-a", "attachment-a")
    const until = Date.now()+1_100
    while (Date.now() < until) {
      assert.deepEqual(await scoped.send({ GetDaemonHealth: null }), { accepted: true })
      await new Promise(resolve => setTimeout(resolve, 40))
    }
    assert.ok(cloud.grantCount() >= 8, "both clients renew across several grant expiries")
    assert.ok(events > 15, "encrypted events remain connected across grant expiries")
    assert.ok(!cloud.kernelRequests.some(v => /CreateCloudSessionInvite|PairCloudRelayClient|IssueCloudRelayClientToken/.test(v)), "human authority stays in the terminal")
    assert.ok(notices.every(message => !message.includes("synthetic-access") && !message.includes("synthetic-refresh") && !message.includes("fixture.")))
  } finally {
    for (const client of clients) client.destroy()
    for (const entry of terminals) entry.client.stop()
    await cloud.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("MP-11: unsafe signal targets and private assertion details never escape the drill", async () => {
  for (const pid of [undefined, NaN, 0, 1, -1, -50]) {
    await assert.rejects(terminateChild({ pid, exitCode: null, signalCode: null, kill() { assert.fail("unsafe signal sent") } }), /invalid child PID/)
  }
  assert.throws(() => drillAssert(false, "safe failure", { credential: "synthetic-private" }), error => error.message === "safe failure")
})
