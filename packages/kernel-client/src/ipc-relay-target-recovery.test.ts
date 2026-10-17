import assert from "node:assert/strict"
import test from "node:test"
import { WebSocketServer } from "ws"

import { LocalIpcClient, LocalIpcError } from "./ipc.js"
import { createRelayKeypair, RelayClientIdentity } from "./relay-crypto.js"

// MP-08 / MP-10 / MP-11: supplementary shared-client routing regression.
// The real relay emits target_not_connected before forwarding a request.
for (const outcome of ["read", "write", "revoked", "lost-write"] as const) {
  test(`MP-08/MP-11 relay target recovery preserves ${outcome} semantics`, async t => {
    const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
    const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
    const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
    await new Promise<void>(resolve => server.once("listening", resolve))
    const address = server.address()
    assert.ok(address && typeof address === "object")
    const requestIds: string[] = []
    let applied = 0
    server.on("connection", socket => socket.on("message", data => {
      const frame = JSON.parse(String(data))
      if (frame.kind === "client_connect") {
        socket.send(JSON.stringify({ kind: "client_connected", target: frame.target,
          daemon_public_key: daemon.publicKeyBase64 }))
        return
      }
      assert.equal(frame.kind, "client_request")
      requestIds.push(frame.request_id)
      if (requestIds.length === 1 && outcome !== "lost-write") {
        socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id,
          encrypted_response: null, error: { code: outcome === "revoked" ? "identity_revoked" : "target_not_connected",
            message: outcome === "revoked" ? "relay token has been revoked" : "target daemon is not connected to relay",
            retryable: outcome !== "revoked" } }))
        return
      }
      const command = JSON.parse(daemon.decrypt(frame.encrypted_request, identity.publicKeyBase64))
      assert.equal(Object.keys(command.request)[0], outcome === "read" ? "GetProviderRun" : "ControlAppWorker")
      applied++
      if (outcome === "lost-write") { socket.close(); return }
      socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, error: null,
        encrypted_response: daemon.encrypt(identity.publicKeyBase64, JSON.stringify({ ok: true })) }))
    }))
    const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
      relayAuthToken: "fixture-token", targetDaemonId: "kernel", relayIdentity: identity,
      controlRequestRetryDeadlineMs: 2_000, reconnectJitterMs: 0, kernelPingIntervalMs: 60_000,
    })
    t.after(async () => {
      client.destroy()
      for (const socket of server.clients) socket.terminate()
      await new Promise<void>(resolve => server.close(() => resolve()))
    })
    const request = outcome === "read" ? { GetProviderRun: { provider_run_id: "run" } }
      : { ControlAppWorker: { installation_id: "app", action: "restart" } }
    if (outcome === "revoked" || outcome === "lost-write") {
      await assert.rejects(client.send(request), (error: unknown) => error instanceof LocalIpcError
        && error.code === (outcome === "revoked" ? "identity_revoked" : "outcome_unknown"))
      assert.equal(requestIds.length, 1)
      assert.equal(applied, outcome === "revoked" ? 0 : 1)
    } else {
      assert.deepEqual(await client.send(request), { ok: true })
      assert.equal(requestIds.length, 2)
      assert.equal(requestIds[0], requestIds[1])
      assert.equal(applied, 1, "an admission rejection never duplicates a kernel mutation")
    }
  })
}
