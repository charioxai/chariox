import assert from "node:assert/strict"
import test from "node:test"
import WebSocket, { WebSocketServer } from "ws"

import { LocalIpcClient } from "./ipc.js"
import type { KernelEvent } from "./kernel-events.js"
import type { EncryptedRelayPayload, RelayTarget } from "./kernel-transport-frames.js"
import { createRelayKeypair, RelayClientIdentity } from "./relay-crypto.js"

type RelayFixtureFrame = {
  kind: string
  request_id?: string
  subscription_id?: string
  target?: RelayTarget
  client_public_key?: string
  encrypted_request?: EncryptedRelayPayload
}

function createIdentity() {
  const keypair = createRelayKeypair()
  const identity = new RelayClientIdentity(keypair.privateKey)
  keypair.privateKey.fill(0)
  return identity
}

async function createRelayFixture(
  handleFrame: (socket: WebSocket, frame: RelayFixtureFrame, connection: number) => void,
) {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>((resolve, reject) => {
    server.once("listening", resolve)
    server.once("error", reject)
  })
  const address = server.address()
  assert.ok(address && typeof address === "object")
  let connectionCount = 0
  server.on("connection", socket => {
    const connection = ++connectionCount
    socket.on("message", data => {
      handleFrame(socket, JSON.parse(String(data)) as RelayFixtureFrame, connection)
    })
  })
  return {
    server,
    url: `ws://127.0.0.1:${address.port}`,
    connectionCount: () => connectionCount,
  }
}

function clientConnectedFrame(target: RelayTarget | undefined, daemon: RelayClientIdentity) {
  return JSON.stringify({
    kind: "client_connected",
    target,
    daemon_public_key: daemon.publicKeyBase64,
  })
}

function clientResponseFrame(requestId: string | undefined, encryptedResponse: EncryptedRelayPayload) {
  return JSON.stringify({
    kind: "client_response",
    request_id: requestId,
    encrypted_response: encryptedResponse,
    error: null,
  })
}

function clientEventFrame(
  subscriptionId: string | undefined,
  clientPublicKey: string,
  daemon: RelayClientIdentity,
  eventId: number,
  event: unknown,
) {
  return JSON.stringify({
    kind: "client_event",
    subscription_id: subscriptionId,
    event_id: eventId,
    encrypted_event: daemon.encrypt(clientPublicKey, JSON.stringify(event)),
  })
}

function createClient(url: string, identity?: RelayClientIdentity) {
  return new LocalIpcClient(url, {
    relayAuthToken: "fixture-token",
    targetDaemonId: "daemon-1",
    relayIdentity: identity,
    kernelEventStaleMs: 0,
    kernelPingIntervalMs: 60_000,
    reconnectJitterMs: 0,
    controlRequestRetryDeadlineMs: 0,
  })
}

function createEventQueue(client: LocalIpcClient) {
  const events: Extract<KernelEvent, { event: "runtime_notices" }>[] = []
  const waiters: ((event: (typeof events)[number]) => void)[] = []
  client.onKernelEvent(event => {
    if (event.event !== "runtime_notices") return
    const waiter = waiters.shift()
    if (waiter) waiter(event)
    else events.push(event)
  })
  return () => {
    const next = events.shift()
    if (next) return Promise.resolve(next)
    return new Promise<(typeof events)[number]>(resolve => waiters.push(resolve))
  }
}

async function withTimeout<T>(promise: Promise<T>, label: string): Promise<T> {
  let timeout: ReturnType<typeof setTimeout> | undefined
  try {
    return await Promise.race([
      promise,
      new Promise<T>((_, reject) => {
        timeout = setTimeout(() => reject(new Error(`timed out waiting for ${label}`)), 2_000)
      }),
    ])
  } finally {
    if (timeout) clearTimeout(timeout)
  }
}

async function closeFixture(server: WebSocketServer, client: LocalIpcClient) {
  client.destroy()
  for (const socket of server.clients) socket.terminate()
  await new Promise<void>(resolve => server.close(() => resolve()))
}

for (const persistent of [false, true]) {
  test(`relay IPC responses reject wrong senders and accept the paired daemon (${persistent ? "persistent" : "ephemeral"})`, async t => {
    const daemonA = createIdentity()
    const daemonB = createIdentity()
    const foreignDaemon = createIdentity()
    let requestNumber = 0
    const fixture = await createRelayFixture((socket, frame, connection) => {
      const daemon = connection === 1 ? daemonA : daemonB
      if (frame.kind === "client_connect") {
        socket.send(clientConnectedFrame(frame.target, daemon))
        return
      }
      if (frame.kind !== "client_request" || !frame.encrypted_request) return
      requestNumber += 1
      const signer = connection === 1
        ? requestNumber % 2 === 1 ? foreignDaemon : daemonA
        : requestNumber % 2 === 1 ? daemonA : daemonB
      const encryptedResponse = signer.encrypt(
        frame.encrypted_request.sender_public_key,
        JSON.stringify({ accepted: true, connection, requestNumber }),
      )
      socket.send(clientResponseFrame(frame.request_id, encryptedResponse))
    })
    const clientKeypair = createRelayKeypair()
    const client = createClient(
      fixture.url,
      persistent ? new RelayClientIdentity(clientKeypair.privateKey) : undefined,
    )
    t.after(async () => {
      clientKeypair.privateKey.fill(0)
      await closeFixture(fixture.server, client)
    })

    await assert.rejects(
      client.send({ GetDaemonHealth: null }),
      /relay sender identity mismatch/,
      "a foreign sender encrypted for the request recipient must be rejected",
    )
    assert.deepEqual(await client.send({ GetDaemonHealth: null }), {
      accepted: true,
      connection: 1,
      requestNumber: 2,
    })

    client.destroy()
    await assert.rejects(
      client.send({ GetDaemonHealth: null }),
      /relay sender identity mismatch/,
      "an old connection key must not authenticate a response after reconnect",
    )
    assert.deepEqual(await client.send({ GetDaemonHealth: null }), {
      accepted: true,
      connection: 2,
      requestNumber: 4,
    })
    assert.equal(fixture.connectionCount(), 2)
  })
}

test("relay IPC rejects a handshake that does not echo the selected daemon target", async t => {
  const daemon = createIdentity()
  let requests = 0
  const fixture = await createRelayFixture((socket, frame) => {
    if (frame.kind === "client_connect") {
      socket.send(clientConnectedFrame({ daemon_id: "other-daemon", daemon_alias: null }, daemon))
    } else if (frame.kind === "client_request") {
      requests += 1
    }
  })
  const client = createClient(fixture.url)
  t.after(() => closeFixture(fixture.server, client))

  await assert.rejects(client.send({ GetDaemonHealth: null }), /different daemon target/)
  assert.equal(requests, 0)
})

test("relay IPC rejects a handshake without a daemon public key", async t => {
  let requests = 0
  const fixture = await createRelayFixture((socket, frame) => {
    if (frame.kind === "client_connect") {
      socket.send(JSON.stringify({ kind: "client_connected", target: frame.target }))
    } else if (frame.kind === "client_request") {
      requests += 1
    }
  })
  const client = createClient(fixture.url)
  t.after(() => closeFixture(fixture.server, client))

  await assert.rejects(client.send({ GetDaemonHealth: null }), /did not provide daemon public key/)
  assert.equal(requests, 0)
})

for (const persistent of [false, true]) {
  test(`relay IPC subscription events pin daemon identity through reconnect (${persistent ? "persistent" : "ephemeral"})`, async t => {
    const daemonA = createIdentity()
    const daemonB = createIdentity()
    const clientKeypair = createRelayKeypair()
    let subscribeCount = 0
    const subscriptionIds: string[] = []
    const fixture = await createRelayFixture((socket, frame, connection) => {
      const daemon = connection === 1 ? daemonA : daemonB
      if (frame.kind === "client_connect") {
        socket.send(clientConnectedFrame(frame.target, daemon))
        return
      }
      if (frame.kind !== "client_subscribe" || !frame.client_public_key) return
      subscribeCount += 1
      subscriptionIds.push(frame.subscription_id!)
      socket.send(clientResponseFrame(
        frame.request_id,
        daemon.encrypt(frame.client_public_key, "null"),
      ))
      const eventId = connection === 1 ? 1 : 2
      const event = (message: string) => ({ event: "runtime_notices", notices: [{ message }] })
      if (connection === 1) {
        socket.send(clientEventFrame(frame.subscription_id, frame.client_public_key, daemonA, eventId, event("daemon-a")))
      } else {
        socket.send(clientEventFrame(frame.subscription_id, frame.client_public_key, daemonA, eventId, event("old-generation")))
        socket.send(clientEventFrame(frame.subscription_id, frame.client_public_key, daemonB, eventId, event("daemon-b")))
      }
    })
    const connectedClient = createClient(
      fixture.url,
      persistent ? new RelayClientIdentity(clientKeypair.privateKey) : undefined,
    )
    t.after(async () => {
      clientKeypair.privateKey.fill(0)
      await closeFixture(fixture.server, connectedClient)
    })
    connectedClient.onRelaySubscriptionDiagnostic(() => { throw new Error("observer failed") })
    const diagnostics: { event: string; subscriptionId: string }[] = []
    connectedClient.onRelaySubscriptionDiagnostic(value => diagnostics.push(value))
    const nextConnectedEvent = createEventQueue(connectedClient)

    const firstEventPromise = withTimeout(nextConnectedEvent(), "first subscription event")
    await Promise.all([
      connectedClient.subscribeToKernelEvents("session-1", "attachment-1"),
      connectedClient.subscribeToKernelEvents("session-1", "attachment-1"),
    ])
    await connectedClient.subscribeToKernelEvents("session-1", "attachment-1")
    assert.equal(subscribeCount, 1, "concurrent/live callers must reuse one binding")
    const firstEvent = await firstEventPromise
    assert.equal(firstEvent.notices[0]?.message, "daemon-a")

    const reconnectedEventPromise = withTimeout(nextConnectedEvent(), "reconnected subscription event")
    await connectedClient.restartKernelEventStream()
    const reconnectedEvent = await reconnectedEventPromise
    assert.equal(reconnectedEvent.notices[0]?.message, "daemon-b")
    assert.equal(subscribeCount, 2)
    assert.equal(new Set(subscriptionIds).size, 2, "MP-08/MP-11 each encrypted binding needs a distinct relay id")
    assert.deepEqual(diagnostics.map(value => value.event), ["binding_sent", "event_decrypted", "binding_sent", "event_decrypt_failed", "event_decrypted"])
    assert.deepEqual(Object.keys(diagnostics[0]!).sort(), ["event", "subscriptionId"])
    assert.equal(diagnostics[2]?.subscriptionId, subscriptionIds[1])
    assert.equal(fixture.connectionCount(), 2)
  })
}
