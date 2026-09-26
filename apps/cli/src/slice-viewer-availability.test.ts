import assert from "node:assert/strict"
import test from "node:test"

import type { SliceRecord } from "@chariox/kernel-client/kernel-types"

import {
  evaluateSliceViewerAvailability,
  formatSliceViewerAvailability,
  validateRoomBoundSliceIdentity,
} from "./slice-viewer-availability.js"

test("headless SliceRecord remains explicitly unavailable even when running", () => {
  const slice = publicSlice({
    id: "slice-headless",
    display_mode: "headless",
    status: "running",
  })

  const availability = evaluateSliceViewerAvailability(slice)

  assert.equal(availability.state, "unavailable")
  assert.match(formatSliceViewerAvailability(availability), /viewer=unavailable .*headless/i)
  assert.match(availability.message, /headed/i)
})

test("headless SliceRecord cannot be made available by a serialized endpoint response", () => {
  const slice = publicSlice({ id: "slice-headless", display_mode: "headless", status: "running" })
  const response = JSON.parse(`{"SliceDisplayEndpoint":{"endpoint":{
    "slice_id":"slice-headless","kind":"selkies","url":"wss://relay.example/display/ephemeral",
    "access":"tunnel","expires_at_ms":60000,"capabilities":["view","websocket"],
    "stream_protocol":"chariox-display-v1","stream_id":"stream-1","peer_public_key":"worker-key"
  }}}`) as { SliceDisplayEndpoint: { endpoint: { slice_id: string; kind: "selkies" } } }

  const availability = evaluateSliceViewerAvailability(slice, { endpoint: response.SliceDisplayEndpoint.endpoint })

  assert.equal(availability.state, "unavailable")
  assert.match(availability.message, /slice-headless.*headless/i)
})

test("a public headed endpoint response is available only for the matching running SliceRecord", () => {
  const slice = publicSlice({
    id: "slice-headed",
    display_mode: "headed",
    status: "running",
    display_endpoint: {
      slice_id: "slice-headed",
      kind: "selkies",
      url: "http://127.0.0.1:45500/",
      access: "local",
      expires_at_ms: null,
      capabilities: ["view", "websocket", "h264", "software_encoding"],
      stream_protocol: null,
      stream_id: null,
      peer_public_key: null,
    },
  })
  const response = JSON.parse(`{"SliceDisplayEndpoint":{"endpoint":{
    "slice_id":"slice-headed","kind":"selkies","url":"wss://relay.example/display/display-2/stream",
    "access":"tunnel","expires_at_ms":60000,"capabilities":["view","websocket","h264","encrypted"],
    "stream_protocol":"chariox-display-v1","stream_id":"display-2","peer_public_key":"worker-key"
  }}}`) as { SliceDisplayEndpoint: { endpoint: { slice_id: string; kind: "selkies" } } }

  const availability = evaluateSliceViewerAvailability(slice, { endpoint: response.SliceDisplayEndpoint.endpoint })

  assert.deepEqual(availability, { state: "available", endpointKind: "selkies" })
  assert.equal(formatSliceViewerAvailability(availability), "viewer=available display=selkies")
})

test("stopped and unhealthy SliceRecord statuses produce actionable unavailable errors", () => {
  const stopped = evaluateSliceViewerAvailability(publicSlice({
    display_mode: "headed",
    status: "stopped",
    last_operation: "stop",
  }))
  const unhealthy = evaluateSliceViewerAvailability(publicSlice({
    display_mode: "headed",
    status: "unhealthy",
    last_error: "worker heartbeat is stale",
  }))

  assert.equal(stopped.state, "unavailable")
  assert.match(stopped.message, /\/slice start slice-1/)
  assert.equal(unhealthy.state, "error")
  assert.match(unhealthy.message, /stale|offline/i)
  assert.match(unhealthy.message, /slice logs|worker/i)
})

test("kernel endpoint failures distinguish stale or offline workers without exposing endpoint data", () => {
  const slice = publicSlice({
    id: "slice-headed",
    name: "desktop",
    display_mode: "headed",
    status: "running",
    display_endpoint: {
      slice_id: "slice-headed",
      kind: "selkies",
      url: "http://127.0.0.1:45500/?token=private",
      access: "local",
    },
  })

  const availability = evaluateSliceViewerAvailability(slice, {
    error: new Error("worker is offline or stale; relay peer is not connected at https://relay.example/display?token=private Bearer credential"),
  })

  assert.equal(availability.state, "error")
  assert.match(availability.message, /stale or offline/i)
  assert.match(formatSliceViewerAvailability(availability), /viewer=error/)
  assert.match(availability.message, /check the worker and relay/)
  assert.doesNotMatch(JSON.stringify(availability), /127\.0\.0\.1|relay\.example|private|credential/)
})

test("authenticated viewer-key denial remains a kernel error instead of an offline diagnosis", () => {
  const slice = publicSlice({
    display_mode: "headed",
    status: "running",
  })

  const availability = evaluateSliceViewerAvailability(slice, {
    error: new Error("viewer key does not match the authenticated relay client"),
  })

  assert.equal(availability.state, "error")
  assert.match(availability.message, /kernel denied display admission.*authenticated relay client/)
  assert.doesNotMatch(availability.message, /check the worker and relay|stale or offline/i)
})

test("Room slice validation preserves binding session, owner, worker, and slice identity", () => {
  const binding = JSON.parse(`{"session_id":"session-1","slice_id":"slice-1",
    "owner_kernel_id":"kernel-home","worker_kernel_ref":"worker-1"}`)
  const slice = publicSlice({
    id: "slice-1",
    owner_kernel_id: "kernel-home",
    worker_kernel_ref: "worker-1",
  })

  assert.equal(validateRoomBoundSliceIdentity("session-1", binding, slice), null)
  assert.match(validateRoomBoundSliceIdentity("session-2", binding, slice) ?? "", /different session/)
  assert.match(validateRoomBoundSliceIdentity("session-1", { ...binding, slice_id: "slice-other" }, slice) ?? "", /slice identity/)
  assert.match(validateRoomBoundSliceIdentity("session-1", { ...binding, owner_kernel_id: "kernel-other" }, slice) ?? "", /owner kernel/)
  assert.match(validateRoomBoundSliceIdentity("session-1", { ...binding, worker_kernel_ref: "worker-other" }, slice) ?? "", /worker identity/)
})

function publicSlice(overrides: Record<string, unknown>): SliceRecord {
  const serialized = JSON.stringify({
    Slice: {
      slice: {
        id: "slice-1",
        name: "desktop",
        owner_kernel_id: "kernel-home",
        owner_machine_id: "machine-home",
        backend: "ssh_docker",
        os: "linux",
        display_mode: "headed",
        status: "running",
        workspace_mount: null,
        worker_kernel_ref: "worker-1",
        worker_kernel_id: "kernel-worker",
        worker_machine_id: "machine-worker",
        relay_endpoint: null,
        local_docker_ports: null,
        providers: [],
        provider_auth: [],
        display_endpoint: {
          slice_id: "slice-1",
          kind: "selkies",
          url: "http://127.0.0.1:45500/",
          access: "local",
          expires_at_ms: null,
          capabilities: ["view", "websocket", "h264", "software_encoding"],
          stream_protocol: null,
          stream_id: null,
          peer_public_key: null,
        },
        created_at_ms: 1,
        updated_at_ms: 2,
        ...overrides,
      },
    },
  })
  return (JSON.parse(serialized) as { Slice: { slice: SliceRecord } }).Slice.slice
}
