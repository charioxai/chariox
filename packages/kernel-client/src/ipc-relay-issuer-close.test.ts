import assert from "node:assert/strict"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { setTimeout as sleep } from "node:timers/promises"
import test from "node:test"
import { WebSocketServer } from "ws"
import { LocalIpcClient, LocalIpcError } from "./ipc.js"
import { RelayClientIdentity, createRelayKeypair } from "./relay-crypto.js"

// Exercise public close(), with a real issuer connection and an unresolved
// issuer request. Inspect timer ownership so an unref'd leak cannot pass.
test("close retires the explicit issuer sockets, heartbeat and in-flight requests", async () => {
  const home = await mkdtemp(join(tmpdir(), "chariox-issuer-close-"))
  const issuerServer = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  const relayServer = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await Promise.all([issuerServer, relayServer].map(server => new Promise<void>(resolve => server.once("listening", resolve))))
  const endpoint = (server: WebSocketServer) => {
    const address = server.address(); assert.ok(address && typeof address === "object")
    return `ws://127.0.0.1:${address.port}/kernel`
  }
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const claims = { sub: "terminal", client_id: "terminal", subject_kind: "client", realm_id: "realm", account_id: "account", user_id: "owner", public_key_thumbprint: identity.publicKeyThumbprint, allowed_actions: ["client.connect", "packet.route"], allowed_targets: ["managed-kernel"], exp: (Date.now() + 1500) / 1000 }
  const token = `synthetic.${Buffer.from(JSON.stringify(claims)).toString("base64url")}.signature`
  let probeRequests = 0, heldRequests = 0
  issuerServer.on("connection", socket => socket.on("message", data => {
    const frame = JSON.parse(String(data))
    if (frame.request.RelayStatus !== undefined) {
      probeRequests++
      socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, error: null,
        response: { RelayStatus: { status: { daemon_id: "account-kernel", capabilities: ["terminal_relay_authorization_renewal_v1"] } } } }))
    } else heldRequests++ // Hold both the renewal and explicit request below.
  }))
  relayServer.on("connection", socket => socket.on("message", data => {
    const frame = JSON.parse(String(data))
    if (frame.kind === "client_connect") socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
    else if (frame.kind === "client_request") socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, error: null, encrypted_response: daemon.encrypt(identity.publicKeyBase64, '{"ok":true}') }))
    else if (frame.kind === "client_subscribe") socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, error: null, encrypted_response: daemon.encrypt(identity.publicKeyBase64, "null") }))
  }))
  const client = new LocalIpcClient(endpoint(relayServer), { relayAuthToken: token, targetDaemonId: "managed-kernel", relayIdentity: identity,
    relayAuthorizationIssuer: { endpoint: endpoint(issuerServer), daemonId: "account-kernel" }, localAuthEnvironment: { CHARIOX_HOME: home }, kernelPingIntervalMs: 60000 })
  try {
    await client.send({ GetDaemonHealth: null })
    await client.subscribeToKernelEvents("session", "attachment")
    const child = Reflect.get(client, "relayIssuer") as LocalIpcClient
    assert.ok(child)
    let settled = false
    const pending = child.send({ ListSessions: null }).then(() => assert.fail("held issuer request must not succeed"), error => { settled = true; return error })
    for (let n = 0; n < 100 && heldRequests < 2; n++) await sleep(10)
    assert(probeRequests > 0 && heldRequests >= 2, "capability probe and renewal must reach the issuer")
    assert.notEqual(Reflect.get(child, "controlHeartbeat"), null)
    await client.close()
    for (let n = 0; n < 30 && (issuerServer.clients.size || relayServer.clients.size); n++) await sleep(10)
    assert.equal(issuerServer.clients.size, 0, "graceful close must release the issuer socket")
    assert.equal(relayServer.clients.size, 0)
    assert.equal(settled, true, "close must retire unresolved issuer work")
    const error = await pending
    assert(error instanceof LocalIpcError); assert.equal(error.code, "client_closed")
    for (const owner of [client, child]) {
      for (const field of ["controlHeartbeat", "eventHeartbeat", "kernelEventWatchdog", "reconnectTimeout"]) assert.equal(Reflect.get(owner, field), null, `${field} must be released`)
      assert.equal(Reflect.get(owner, "relayRenewal"), null)
      const requests = Reflect.get(Reflect.get(owner, "pendingRequests"), "pending") as Map<string, unknown>
      assert.equal(requests.size, 0, "pending request timers must be retired")
    }
    const probes = probeRequests
    await sleep(100)
    assert.equal(probeRequests, probes, "closed issuer must not reconnect")
  } finally {
    client.destroy() // Failure cleanup only; assertions above use close().
    for (const server of [issuerServer, relayServer]) {
      for (const socket of server.clients) socket.terminate()
      await new Promise<void>(resolve => server.close(() => resolve()))
    }
    await rm(home, { recursive: true, force: true })
  }
})
