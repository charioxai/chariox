import assert from "node:assert/strict"
import test from "node:test"
import { createECDH } from "node:crypto"
import { LocalIpcClient, RelayClientIdentity } from "./ipc.js"
import { createMutableLocalIpcClient } from "./mutable-local-ipc-client.js"
import { createCliCommandActionComposition } from "./cli-command-action-composition.js"

import { readRoomViewerAvailability } from "./room-viewer-availability.js"

for (const { viewer, initialRelay } of (["room", "slice"] as const)
  .flatMap((viewer) => [true, false].map((initialRelay) => ({ viewer, initialRelay })))) {
  test(`${viewer} viewer resolves current identity after pivots from ${initialRelay ? "relay" : "local"}`, async () => {
    const keys = createECDH("prime256v1")
    keys.generateKeys()
    const identity = new RelayClientIdentity(keys.getPrivateKey())
    const responses = publicRoomSliceResponses("headed")
    const requestedKeys: string[] = []
    const footers: string[] = []
    let afterBinding: (() => void) | undefined
    const makeClient = (relay: boolean, paired = false) => {
      const client = new LocalIpcClient(relay ? "wss://relay.example.test" : "ws://127.0.0.1:1", relay ? {
        relayAuthToken: "synthetic-test-token",
        ...(paired ? { relayIdentity: identity } : {}),
      } : {})
      client.send = async <T>(request: unknown): Promise<T> => {
        if ("GetRoomEnvironmentSlice" in (request as object)) {
          afterBinding?.()
          return responses.binding as T
        }
        if ("GetSlice" in (request as object)) return responses.slice as T
        if ("GetSliceDisplayEndpoint" in (request as object)) {
          requestedKeys.push((request as { GetSliceDisplayEndpoint: { viewer_public_key: string } }).GetSliceDisplayEndpoint.viewer_public_key)
          throw new Error("synthetic endpoint stop")
        }
        throw new Error(`unexpected request: ${JSON.stringify(request)}`)
      }
      return client
    }
    const client = createMutableLocalIpcClient(makeClient(initialRelay, initialRelay))
    const handlers = createCliCommandActionComposition({
      client, options: {}, preferencesState: () => ({}),
      isAttached: () => true, sessionState: () => ({ id: "session-1" }),
      attachmentState: () => ({ id: "attachment-1" }), focusedAgentId: () => "agent-1",
      flashFooter: (message: string) => footers.push(message), appendNotice: () => {},
    } as unknown as Parameters<typeof createCliCommandActionComposition>[0])
    const view = () => viewer === "room"
      ? handlers.handleRoomCommand({ kind: "room", raw: "/room view", args: ["view"] })
      : handlers.handleSliceCommand({ kind: "slice", raw: "/slice screen slice-1", args: ["screen", "slice-1"] })
    await view()
    assert.equal(requestedKeys.length, 1)
    assert.ok(requestedKeys[0])
    if (initialRelay) assert.equal(requestedKeys[0], identity.publicKeyBase64)
    else assert.notEqual(requestedKeys[0], identity.publicKeyBase64)
    client.swapClient(makeClient(true))
    await view()
    assert.equal(requestedKeys.length, 1, "unpaired relay must not allocate an endpoint")
    assert.match(footers.at(-1)!, /paired key-bound relay identity/)
    client.swapClient(makeClient(false))
    await view()
    assert.equal(requestedKeys.length, 2)
    assert.ok(requestedKeys[1])
    assert.notEqual(requestedKeys[1], identity.publicKeyBase64)
    client.swapClient(makeClient(true, true))
    await view()
    assert.equal(requestedKeys[2], identity.publicKeyBase64)
    afterBinding = () => client.swapClient(makeClient(true))
    await view()
    assert.equal(requestedKeys.length, 3, "pivot while reading binding must not reuse the preflight key")
    assert.match(footers.at(-1)!, /paired key-bound relay identity/)
  })
}

test("Room status returns an explicitly unavailable headless slice without opening a display endpoint", async () => {
  const requests: unknown[] = []
  const responses = publicRoomSliceResponses("headless")
  const result = await readRoomViewerAvailability({
    attachmentId: () => "attachment-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      if ("GetRoomEnvironmentSlice" in (request as object)) return responses.binding as TResponse
      if ("GetSlice" in (request as object)) return responses.slice as TResponse
      throw new Error(`unexpected request: ${JSON.stringify(request)}`)
    },
  }, "session-1", "/room status", false)

  assert.equal(result.binding?.slice_id, "slice-1")
  assert.equal(result.availability.state, "unavailable")
  assert.match(result.availability.message, /headless/)
  assert.deepEqual(requests, [
    { GetRoomEnvironmentSlice: { session_id: "session-1" } },
    { GetSlice: { slice_ref: "slice-1" } },
  ])
})

test("remote Room view without the paired identity reports an error before endpoint allocation", async () => {
  const requests: unknown[] = []
  const responses = publicRoomSliceResponses("headed")
  const result = await readRoomViewerAvailability({
    attachmentId: () => "attachment-1",
    isRelayConnection: () => true,
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      if ("GetRoomEnvironmentSlice" in (request as object)) return responses.binding as TResponse
      if ("GetSlice" in (request as object)) return responses.slice as TResponse
      throw new Error(`unexpected request: ${JSON.stringify(request)}`)
    },
  }, "session-1", "/room view", true)

  assert.equal(result.availability.state, "error")
  assert.match(result.availability.message, /paired key-bound relay identity/)
  assert.equal(requests.some((request) => "GetSliceDisplayEndpoint" in (request as object)), false)
})

function publicRoomSliceResponses(displayMode: "headless" | "headed") {
  return JSON.parse(JSON.stringify({
    binding: {
      RoomEnvironmentSlice: {
        binding: {
          session_id: "session-1",
          slice_id: "slice-1",
          owner_kernel_id: "kernel-home",
          worker_kernel_ref: "kernel-worker",
        },
      },
    },
    slice: {
      Slice: {
        slice: {
          id: "slice-1",
          name: "desktop",
          owner_kernel_id: "kernel-home",
          owner_machine_id: "machine-home",
          backend: "ssh_docker",
          os: "linux",
          display_mode: displayMode,
          status: "running",
          workspace_mount: null,
          worker_kernel_ref: "kernel-worker",
          worker_kernel_id: "kernel-worker-id",
          worker_machine_id: "machine-worker",
          relay_endpoint: null,
          local_docker_ports: null,
          providers: [],
          provider_auth: [],
          display_endpoint: displayMode === "headed" ? {
            slice_id: "slice-1",
            kind: "selkies",
            url: "http://127.0.0.1:45500/",
            access: "local",
            expires_at_ms: null,
            capabilities: ["view", "websocket", "h264"],
            stream_protocol: null,
            stream_id: null,
            peer_public_key: null,
          } : null,
          created_at_ms: 1,
          updated_at_ms: 2,
        },
      },
    },
  }))
}
