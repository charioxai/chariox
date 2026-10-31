import assert from "node:assert/strict"
import test from "node:test"
import { setTimeout as sleep } from "node:timers/promises"
import { WebSocketServer } from "ws"
import { LocalIpcClient } from "./ipc.js"
import { RelayClientIdentity, createRelayKeypair } from "./relay-crypto.js"

const observationMs = 7_000

type Relay = { port: number; connections: () => number; handshakes: () => number; close: () => Promise<void> }

// `refuse` answers every client_connect with the relay's real expired-token close;
// otherwise the connect is admitted and the socket is dropped right after the subscribe.
let currentIdentity: RelayClientIdentity | undefined

async function fakeRelay(behaviour: "refuse-expired" | "drop-after-subscribe"): Promise<Relay> {
  const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address(); assert.ok(address && typeof address === "object")
  let connections = 0, handshakes = 0
  server.on("connection", socket => {
    connections++
    socket.on("message", data => {
      const frame = JSON.parse(String(data))
      if (frame.kind === "client_connect") {
        handshakes++
        if (behaviour === "refuse-expired") {
          socket.send(JSON.stringify({ kind: "close", reason: "relay token is expired" })); socket.close(); return
        }
        socket.send(JSON.stringify({ kind: "client_connected", target: frame.target, daemon_public_key: daemon.publicKeyBase64 }))
      } else if (frame.kind === "client_subscribe") {
        socket.send(JSON.stringify({ kind: "client_response", request_id: frame.request_id, error: null, encrypted_response: daemon.encrypt(currentIdentity!.publicKeyBase64, "null") }))
        setTimeout(() => socket.terminate(), 20)
      }
    })
  })
  return {
    port: address.port,
    connections: () => connections,
    handshakes: () => handshakes,
    close: () => new Promise<void>(resolve => { for (const socket of server.clients) socket.terminate(); server.close(() => resolve()) }),
  }
}


const expiredToken = (extra: Record<string, unknown> = {}) =>
  `synthetic.${Buffer.from(JSON.stringify({ sub: "terminal", subject_kind: "client", realm_id: "realm", exp: (Date.now() - 60_000) / 1000, ...extra })).toString("base64url")}.signature`

test("an expired token the relay refuses is not presented again by a client that cannot renew it", async () => {
  const relay = await fakeRelay("refuse-expired")
  currentIdentity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const client = new LocalIpcClient(`ws://127.0.0.1:${relay.port}`, { relayAuthToken: expiredToken(), targetDaemonId: "kernel", relayIdentity: currentIdentity, kernelPingIntervalMs: 60_000, controlRequestRetryDeadlineMs: 0 })
  client.onKernelEvent(() => {})
  try {
    await client.subscribeToKernelEvents("session", "attachment").catch(() => {})
    await sleep(observationMs)
    assert.ok(relay.handshakes() <= 1, `the expired token was presented ${relay.handshakes()} times in ${observationMs} ms`)
    await assert.rejects(client.send({ GetDaemonHealth: null }), /authorization|sign in|pair/i)
  } finally {
    client.destroy(); await relay.close()
  }
})

test("a paired client forced to renew after an expired-token refusal waits for the issuer's retry hint", async () => {
  const relay = await fakeRelay("refuse-expired")
  currentIdentity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const client = new LocalIpcClient(`ws://127.0.0.1:${relay.port}`, { relayAuthToken: expiredToken(), targetDaemonId: "kernel", relayIdentity: currentIdentity, kernelPingIntervalMs: 60_000, controlRequestRetryDeadlineMs: 0 })
  let issues = 0
  // The client's clock still calls the grant valid (skew); only the relay says it expired.
  client.startRelayAuthRenewal(Date.now() + 3_600_000, async () => {
    issues++
    throw Object.assign(new Error("cloud relay request failed with 429: cloud_api_code=rate_limited: Too many requests, retry in 20 seconds"), { code: "rate_limited" })
  })
  client.onKernelEvent(() => {})
  try {
    await client.subscribeToKernelEvents("session", "attachment").catch(() => {})
    await sleep(observationMs)
    assert.ok(relay.handshakes() <= 1, `the refused token was presented ${relay.handshakes()} times`)
    assert.ok(issues <= 2, `the issuer was asked ${issues} times although it said retry in 20 seconds`)
  } finally {
    client.destroy(); await relay.close()
  }
})

test("a relay that drops every connection right after the subscribe sees growing reconnect delays", async () => {
  const relay = await fakeRelay("drop-after-subscribe")
  currentIdentity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const client = new LocalIpcClient(`ws://127.0.0.1:${relay.port}`, { relayAuthToken: "operator-token", targetDaemonId: "kernel", relayIdentity: currentIdentity, kernelPingIntervalMs: 60_000, controlRequestRetryDeadlineMs: 0, reconnectJitterMs: 0 })
  client.onKernelEvent(() => {})
  try {
    await client.subscribeToKernelEvents("session", "attachment").catch(() => {})
    await sleep(observationMs)
    assert.ok(relay.handshakes() <= 6, `${relay.handshakes()} reconnects in ${observationMs} ms`)
  } finally {
    client.destroy(); await relay.close()
  }
})
