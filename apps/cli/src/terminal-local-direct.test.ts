// MP-08/MP-11: real Node sockets, supplementary to the built TUI live drill.
import assert from "node:assert/strict"
import test from "node:test"
import { once } from "node:events"
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { WebSocketServer } from "ws"
import { RelayClientIdentity } from "@chariox/kernel-client/ipc"
import { createECDH } from "node:crypto"
import { LocalIpcClient } from "./ipc.js"

// The production relay crypto class is exported by ipc; generate runtime-only
// identities for this fixture, never read a linked provider profile.
const identity = () => { const ec = createECDH("prime256v1"); ec.generateKeys(); return new RelayClientIdentity(ec.getPrivateKey()) }

test("MP-08 detached and native clients send the paired Origin in Node and compiled Bun transport", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "chariox-terminal-direct-"))
  const previous = process.env.CHARIOX_HOME
  process.env.CHARIOX_HOME = root
  const daemon = identity(); const terminal = identity()
  const local = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  const relay = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await Promise.all([once(local, "listening"), once(relay, "listening")])
  const address = local.address(); const relayAddress = relay.address()
  assert(address && typeof address !== "string" && relayAddress && typeof relayAddress !== "string")
  const registry = join(root, "kernels", "active"); mkdirSync(registry, { recursive: true })
  writeFileSync(join(registry, "kernel.json"), JSON.stringify({ schema_version: 1, kernel_id: "home-1",
    machine_id: "machine-1", host: "127.0.0.1", port: address.port, heartbeat_at_ms: Date.now(), local_daemon_protocol_version: 473 }))
  let admitted = 0; let origins: string[] = []; let grants = 0
  local.on("connection", (socket, request) => {
    origins.push(String(request.headers.origin))
    socket.send(JSON.stringify({ kind: "local_challenge", kernel_id: "home-1", endpoint_epoch: "epoch", challenge: "challenge" }))
    socket.on("message", (data) => {
      const frame = JSON.parse(String(data))
      if (frame.kind === "local_connect") {
        const proof = JSON.parse(daemon.decrypt(frame.proof, terminal.publicKeyBase64))
        assert.equal(proof.origin, "https://cloud.example.test"); assert.equal(proof.grant, "grant")
        admitted++
        socket.send(JSON.stringify({ kind: "local_connected", daemon_public_key: daemon.publicKeyBase64,
          proof: daemon.encrypt(terminal.publicKeyBase64, JSON.stringify({ grant: "grant", challenge: "challenge" })) }))
      } else if (frame.kind === "client_request") {
        socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, error: null,
          encrypted_response: daemon.encrypt(terminal.publicKeyBase64, JSON.stringify({ Sessions: [] })) }))
      }
    })
  })
  relay.on("connection", socket => socket.on("message", data => {
    const frame = JSON.parse(String(data))
    if (frame.kind === "client_connect") socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
    else if (frame.kind === "client_request") {
      const body = JSON.parse(daemon.decrypt(frame.encrypted_request, terminal.publicKeyBase64))
      let response: unknown = { Sessions: [] }
      if (body.local_terminal_connect) { grants++; response = { LocalTerminalConnectIssued: {
        endpoint: `ws://127.0.0.1:${address.port}/v1/browser`, grant: "grant", kernel_id: "home-1",
        endpoint_epoch: "epoch", paired_origin: "https://cloud.example.test", expires_at_ms: Date.now() + 30_000 } } }
      socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, error: null,
        encrypted_response: daemon.encrypt(terminal.publicKeyBase64, JSON.stringify(response)) }))
    }
  }))
  const client = new LocalIpcClient(`ws://127.0.0.1:${relayAddress.port}`, {
    relayAuthToken: "fixture", targetDaemonId: "home-1", relayIdentity: terminal,
  })
  t.after(async () => {
    await client.close()
    for (const server of [local, relay]) { for (const socket of server.clients) socket.terminate(); await new Promise<void>(resolve => server.close(() => resolve())) }
    if (previous === undefined) delete process.env.CHARIOX_HOME; else process.env.CHARIOX_HOME = previous
    rmSync(root, { recursive: true, force: true })
  })
  assert.deepEqual(await client.send({ ListSessions: {} }), { Sessions: [] })
  assert.equal(grants, 1, "must request the explicit short terminal lease")
  assert.equal(admitted, 1, "must use authenticated direct transport automatically")
  assert.deepEqual(origins, ["https://cloud.example.test"])
})
