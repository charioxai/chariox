// MP-08/MP-11: real Node sockets, supplementary to the built TUI live drill.
import assert from "node:assert/strict"
import test, { type TestContext } from "node:test"
import { setTimeout as sleep } from "node:timers/promises"
import { once } from "node:events"
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { WebSocket, WebSocketServer } from "ws"
import { RelayClientIdentity } from "@chariox/kernel-client/ipc"
import { createECDH } from "node:crypto"
import { LocalIpcClient } from "./ipc.js"

// The production relay crypto class is exported by ipc; generate runtime-only
// identities for this fixture, never read a linked provider profile.
const identity = () => { const ec = createECDH("prime256v1"); ec.generateKeys(); return new RelayClientIdentity(ec.getPrivateKey()) }

type Fixture = { client: LocalIpcClient; origins: string[]; grants: number; admitted: number; directCloses: number
  directForeignFrames: number; renewals: number; leaseRenewals: number; expiredRelayConnects: number }

// Mirrors the kernel and relay seams: the direct session closes on any frame
// that is not a client request, and the relay refuses an expired bearer.
async function fixture(t: TestContext, token: (thumbprint: string) => string): Promise<Fixture> {
  const root = mkdtempSync(join(tmpdir(), "chariox-terminal-direct-"))
  const previous = process.env.CHARIOX_HOME
  process.env.CHARIOX_HOME = root
  const daemon = identity(); const terminal = identity()
  const local = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  const relay = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await Promise.all([once(local, "listening"), once(relay, "listening")])
  const address = local.address(); const relayAddress = relay.address()
  assert(address && typeof address !== "string" && relayAddress && typeof relayAddress !== "string")
  const relayUrl = `ws://127.0.0.1:${relayAddress.port}`
  const registry = join(root, "kernels", "active"); mkdirSync(registry, { recursive: true })
  writeFileSync(join(registry, "kernel.json"), JSON.stringify({ schema_version: 1, kernel_id: "home-1",
    machine_id: "machine-1", host: "127.0.0.1", port: address.port, heartbeat_at_ms: Date.now(), local_daemon_protocol_version: 473 }))
  const state = { origins: [] as string[], grants: 0, admitted: 0, directCloses: 0, directForeignFrames: 0,
    renewals: 0, leaseRenewals: 0, expiredRelayConnects: 0 }
  const answer = (socket: WebSocket, frame: { request_id: string; encrypted_request: never }) => {
    const parsed = JSON.parse(daemon.decrypt(frame.encrypted_request, terminal.publicKeyBase64))
    const body = parsed.request ?? parsed
    let response: unknown = { Sessions: [] }
    if (body.RelayStatus !== undefined) response = { RelayStatus: { status: { capabilities: ["terminal_relay_authorization_renewal_v1"] } } }
    else if (body.IssueCloudRelayClientToken) { state.renewals++; response = { CloudRelayClientTokenIssued: { profile: {}, token: { relay_url: relayUrl, relay_token: token(terminal.publicKeyThumbprint) } } } }
    else if (body.local_terminal_connect) { state.grants++; response = { LocalTerminalConnectIssued: {
      endpoint: `ws://127.0.0.1:${address.port}/v1/browser`, grant: "grant", kernel_id: "home-1",
      endpoint_epoch: "epoch", paired_origin: "https://cloud.example.test", expires_at_ms: Date.now() + 30_000 } } }
    else if (body.local_terminal_renew) { state.leaseRenewals++; response = { LocalTerminalLeaseRenewed: {
      expires_at_ms: Date.now() + 30_000, next_sequence: body.local_terminal_renew.sequence + 1 } } }
    socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, error: null,
      encrypted_response: daemon.encrypt(terminal.publicKeyBase64, JSON.stringify(response)) }))
  }
  local.on("connection", (socket, request) => {
    state.origins.push(String(request.headers.origin))
    socket.on("close", () => { state.directCloses++ })
    socket.send(JSON.stringify({ kind: "local_challenge", kernel_id: "home-1", endpoint_epoch: "epoch", challenge: "challenge" }))
    socket.on("message", (data) => {
      const frame = JSON.parse(String(data))
      if (frame.kind === "local_connect") {
        const proof = JSON.parse(daemon.decrypt(frame.proof, terminal.publicKeyBase64))
        assert.equal(proof.origin, "https://cloud.example.test"); assert.equal(proof.grant, "grant")
        state.admitted++
        socket.send(JSON.stringify({ kind: "local_connected", daemon_public_key: daemon.publicKeyBase64,
          proof: daemon.encrypt(terminal.publicKeyBase64, JSON.stringify({ grant: "grant", challenge: "challenge" })) }))
      } else if (frame.kind === "client_request") answer(socket, frame)
      else { state.directForeignFrames++; socket.close() }
    })
  })
  relay.on("connection", socket => socket.on("message", data => {
    const frame = JSON.parse(String(data))
    if (frame.kind === "client_connect") {
      const exp = JSON.parse(Buffer.from(String(frame.token).split(".")[1] ?? "", "base64url").toString("utf8") || "{}").exp
      if (Number.isFinite(exp) && exp * 1000 <= Date.now()) {
        state.expiredRelayConnects++
        socket.send(JSON.stringify({ kind: "close", reason: "relay token has expired" })); socket.close(); return
      }
      socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
    } else if (frame.kind === "client_request") answer(socket, frame)
  }))
  const client = new LocalIpcClient(relayUrl, { relayAuthToken: token(terminal.publicKeyThumbprint), targetDaemonId: "home-1", relayIdentity: terminal,
    kernelPingIntervalMs: 60_000, controlRequestRetryDeadlineMs: 0 })
  t.after(async () => {
    await client.close()
    for (const server of [local, relay]) { for (const socket of server.clients) socket.terminate(); await new Promise<void>(resolve => server.close(() => resolve())) }
    if (previous === undefined) delete process.env.CHARIOX_HOME; else process.env.CHARIOX_HOME = previous
    rmSync(root, { recursive: true, force: true })
  })
  return Object.assign(state, { client })
}

test("MP-08 detached and native clients send the paired Origin in Node and compiled Bun transport", async (t) => {
  const state = await fixture(t, () => "fixture")
  assert.deepEqual(await state.client.send({ ListSessions: {} }), { Sessions: [] })
  assert.equal(state.grants, 1, "must request the explicit short terminal lease")
  assert.equal(state.admitted, 1, "must use authenticated direct transport automatically")
  assert.deepEqual(state.origins, ["https://cloud.example.test"])
})

test("MP-08/MP-11 relay authorization renewal keeps the direct carrier and its relay-renewed lease", async (t) => {
  const token = (thumbprint: string) => "synthetic." + Buffer.from(JSON.stringify({ sub: "terminal", client_id: "terminal", subject_kind: "client",
    realm_id: "realm", account_id: "account", user_id: "owner", public_key_thumbprint: thumbprint,
    allowed_actions: ["client.connect", "packet.route"], allowed_targets: ["home-1"], exp: (Date.now() + 1_500) / 1000 })).toString("base64url") + ".signature"
  const state = await fixture(t, token)
  assert.deepEqual(await state.client.send({ ListSessions: {} }), { Sessions: [] })
  assert.equal(state.client.isLocalDirectTransport(), true)
  // Several short grant expiries, then the first 10-second direct lease renewal.
  await sleep(11_500)
  assert.ok(state.renewals >= 2, "the bound terminal identity must renew before expiry")
  assert.equal(state.directForeignFrames, 0, "relay re-authentication must never be written to a direct socket")
  assert.equal(state.directCloses, 0, "renewal must not retire the healthy direct carrier")
  assert.ok(state.leaseRenewals >= 1, "the direct lease must renew over the relay")
  assert.equal(state.expiredRelayConnects, 0, "direct lease renewal must use the renewed relay identity")
  assert.equal(state.client.isLocalDirectTransport(), true)
  assert.deepEqual(await state.client.send({ ListSessions: {} }), { Sessions: [] })
})
