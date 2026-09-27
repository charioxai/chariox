import assert from "node:assert/strict"
import test from "node:test"

import { readRoomViewerAvailability } from "./room-viewer-availability.js"

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
