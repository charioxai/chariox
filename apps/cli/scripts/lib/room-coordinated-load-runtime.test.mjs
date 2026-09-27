import assert from "node:assert/strict"
import { EventEmitter } from "node:events"
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { COORDINATED_LOAD_SCHEMA, validateCoordinatedLoadConfig } from "./room-coordinated-load-plan.mjs"
import { createRoomCoordinatedLoadRuntime } from "./room-coordinated-load-runtime.mjs"

const portMap = {
  codex: 44000, opencode: 44300, kernel: 44600, mcp: 44900, relay: 45200, novnc: 45500,
  codex_range_start: 46000, opencode_range_start: 51200,
}

function approvedPlan() {
  return validateCoordinatedLoadConfig({
    schema: COORDINATED_LOAD_SCHEMA,
    runId: "runtime-test-123",
    approval: { reference: "change-123", approvedMaxHeadedSlices: 1 },
    homeKernel: { url: "ws://127.0.0.1:4100", kernelId: "kernel-local", machineId: "machine-local" },
    relay: { url: "ws://localhost:4200", targetDaemonId: "kernel-local", tokenEnv: "CHARIOX_DRILL_H_RELAY_TOKEN" },
    headedSlices: [{
      sliceId: "slice-1", sliceName: "drillh-runtime-test-123-one", roomId: "room-1",
      environmentId: "environment-1", runtimeGeneration: 2,
      workerKernelId: "kernel-local", workerMachineId: "machine-local", ports: portMap,
    }],
    workflow: { roomId: "room-1", workflowId: "workflow-1", endpointId: "endpoint-1", prompt: "Run prepared workflow" },
    workspace: { path: "/tmp/drill-h-workspace", worktree: "/tmp/drill-h-worktree" },
    limits: { durationMs: 5_000, sampleIntervalMs: 1_000, maxKernelLatencyMs: 2_000, maxMemoryBytes: 1_000_000_000, maxCpuPercent: 500 },
    slowViewer: { viewerId: "web-local", afterSample: 1, delayMs: 100 },
    evidenceRoot: "/tmp/drill-h-evidence",
  }, { repoRoot: "/source/chariox" })
}

function publicRequests() {
  return {
    listSlicesRequest: () => ({ ListSlices: null }),
    getSliceRequest: (sliceRef) => ({ GetSlice: { slice_ref: sliceRef } }),
    getRoomEnvironmentSliceRequest: (sessionId) => ({ GetRoomEnvironmentSlice: { session_id: sessionId } }),
    getRoomEnvironmentStateRequest: (sessionId) => ({ GetRoomEnvironmentState: { session_id: sessionId } }),
    attachToSessionRequest: (sessionId, clientId) => ({
      AttachToSession: { session_id: sessionId, client_id: clientId, capability_level: "FullTerminal" },
    }),
    detachFromSessionRequest: (attachmentId) => ({ DetachFromSession: { attachment_id: attachmentId } }),
  }
}

function clientFixture({ rejectInventory = false } = {}) {
  const requests = []
  const clients = []
  class LocalIpcClient {
    constructor(url, options) { this.url = url; this.options = options; this.closed = 0; clients.push(this) }
    async send(request) {
      requests.push(request)
      if (Object.hasOwn(request, "ListSlices")) {
        return rejectInventory ? { Error: { message: "fixture refusal" } } : { SlicesListed: { slices: [{ id: "slice-1" }] } }
      }
      if (request.GetSlice) return { Slice: { slice: {
        id: "slice-1", name: "drillh-runtime-test-123-one", status: "running", display_mode: "headed",
        backend: "local_docker", owner_kernel_id: "kernel-local", owner_machine_id: "machine-local",
        worker_kernel_id: "kernel-local", worker_machine_id: "machine-local", local_docker_ports: portMap,
      } } }
      if (request.GetRoomEnvironmentSlice) return { RoomEnvironmentSlice: { binding: {
        session_id: "room-1", slice_id: "slice-1", owner_kernel_id: "kernel-local",
      } } }
      if (request.GetRoomEnvironmentState) return { RoomEnvironmentState: { environment: {
        session_id: "room-1", environment_id: "environment-1", runtime_generation: 2, lifecycle: "ready",
      } } }
      if (request.AttachToSession) return { SessionAttached: { attachment: { id: `attachment-${requests.length}` } } }
      if (request.DetachFromSession) return { SessionDetached: {} }
      throw new Error("unexpected public request shape")
    }
    async close() { this.closed += 1 }
  }
  return { requests, clients, protocol: { LocalIpcClient, requests: publicRequests(), openSelkiesDisplayStream: null } }
}

async function inScratch(t) {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-load-runtime-test-"))
  const runDirectory = path.join(root, "run")
  await mkdir(runDirectory, { mode: 0o700 })
  const repoRoot = path.join(root, "repo")
  await mkdir(path.join(repoRoot, "apps/cli/dist"), { recursive: true })
  await writeFile(path.join(repoRoot, "apps/cli/dist/index.js"), "fixture cli")
  t.after(() => rm(root, { recursive: true, force: true }))
  return { root, runDirectory, repoRoot }
}

function fakeExecFile(command, args) {
  if (command === "docker" && args[0] === "ps") return { stdout: `${JSON.stringify({ ID: "container-1", Names: "chariox-slice-drillh-runtime-test-123-one", Status: "Up 1 second", Ports: "" })}\n` }
  if (command === "docker" && args[0] === "volume") return { stdout: "chariox-slice-drillh-runtime-test-123-one-home\n" }
  if (command === "lsof") return { stdout: "" }
  throw new Error(`unexpected resource command: ${command}`)
}

test("actual captureInventory uses the listener observer and exact prepared resource scope", async (t) => {
  const scratch = await inScratch(t)
  const plan = approvedPlan()
  const calls = []
  const { protocol } = clientFixture()
  const runtime = await createRoomCoordinatedLoadRuntime({ plan, ...scratch }, {
    protocol, relayToken: "fixture-relay-token", localKernelAuthEnvironment: {},
    execFileAsync: async (...args) => { calls.push(args); return fakeExecFile(...args) },
    tuiProcess: { currentOwnedPids: async (tasks) => { assert.deepEqual(tasks, []); return [] } },
  })
  const inventory = await runtime.captureInventory(null, { phase: "before", ownedTasks: [] })
  assert.deepEqual(inventory.containers, ["container-1|chariox-slice-drillh-runtime-test-123-one|Up|"])
  assert.deepEqual(inventory.volumes, ["chariox-slice-drillh-runtime-test-123-one-home"])
  assert.deepEqual(inventory.ports, [])
  assert.deepEqual(calls.map(([command, args]) => [command, args[0]]), [["docker", "ps"], ["docker", "volume"], ["lsof", "-nP"]])
  const expectedPorts = [...plan.headedSlices.flatMap((slice) => slice.publishedPorts)].sort((left, right) => left - right)
  assert.deepEqual(calls[2][1], ["-nP", ...expectedPorts.flatMap((port) => [`-iTCP:${port}`]), "-sTCP:LISTEN", "-Fpn"])
})

test("actual verifyPrepared sends public identity requests and closes its client on pre-task failure", async (t) => {
  const scratch = await inScratch(t)
  const rejected = clientFixture({ rejectInventory: true })
  const runtime = await createRoomCoordinatedLoadRuntime({ plan: approvedPlan(), ...scratch }, {
    protocol: rejected.protocol, relayToken: "fixture-relay-token", localKernelAuthEnvironment: {},
  })
  await assert.rejects(runtime.verifyPrepared(), /public kernel response omitted its expected variant/)
  assert.deepEqual(rejected.requests, [{ ListSlices: null }])
  assert.equal(rejected.clients.length, 1)
  assert.equal(rejected.clients[0].closed, 1)
})

test("actual runtime attaches a viewer, starts Selkies, starts a normal TUI, then cleans only owned handles", async (t) => {
  const scratch = await inScratch(t)
  const fixture = clientFixture()
  const controls = []
  let received = 0
  const stream = {
    endpoint: { stream_id: "display-stream-1", slice_id: "slice-1" },
    async sendControl(type) { controls.push(type) },
    async receive() {
      received += 1
      return received === 1
        ? { kind: "text", data: new TextEncoder().encode("VIDEO_STARTED") }
        : { kind: "binary", data: new Uint8Array([1, 2, 3]) }
    },
    async close() { controls.push("CLOSED") },
  }
  fixture.protocol.openSelkiesDisplayStream = async (options) => {
    assert.equal(options.sessionId, "room-1")
    assert.equal(options.sliceId, "slice-1")
    assert.match(options.attachmentId, /^attachment-/)
    return stream
  }
  const spawnCalls = []
  const child = Object.assign(new EventEmitter(), { pid: 32123, exitCode: null, signalCode: null })
  const stopped = []
  const runtime = await createRoomCoordinatedLoadRuntime({ plan: approvedPlan(), ...scratch }, {
    protocol: fixture.protocol, relayToken: "fixture-relay-token", localKernelAuthEnvironment: {},
    spawn: (...args) => { spawnCalls.push(args); return child },
    tuiProcess: {
      waitForAutomationRoom: async (state, roomId) => { assert.equal(roomId, "room-1"); state.attachmentId = "attachment-tui" },
      stopProcessGroup: async (pid, ownedChild) => { stopped.push(pid); assert.equal(ownedChild, child) },
      removeAutomationSocket: async () => {},
    },
  })
  const prepared = await runtime.verifyPrepared()
  assert.equal(prepared.slices[0].workerKernelId, "kernel-local")
  assert.deepEqual(fixture.requests.slice(0, 4), [
    { ListSlices: null }, { GetSlice: { slice_ref: "slice-1" } },
    { GetRoomEnvironmentSlice: { session_id: "room-1" } }, { GetRoomEnvironmentState: { session_id: "room-1" } },
  ])
  const viewer = await runtime.startViewer(null, { id: "web-local", route: "local" })
  assert.deepEqual(fixture.requests[4], { AttachToSession: {
    session_id: "room-1", client_id: "runtime-test-123-web-local", capability_level: "FullTerminal",
  } })
  assert.equal(viewer.streamId, "display-stream-1")
  const tui = await runtime.startTui(null, { id: "local-tui", route: "local" })
  assert.equal(spawnCalls.length, 1)
  assert.equal(spawnCalls[0][0], "script")
  assert.ok(spawnCalls[0][1].join(" ").includes("--session"))
  assert.ok(spawnCalls[0][1].join(" ").includes("room-1"))
  assert.ok(spawnCalls[0][1].join(" ").includes("runtime-test-123-local-tui"))
  assert.equal(tui.processGroupId, child.pid)
  assert.deepEqual(await runtime.stopOwnedTask(tui), { ownerRunId: "runtime-test-123", stopped: true, remaining: false })
  assert.deepEqual(await runtime.stopOwnedTask(viewer), { ownerRunId: "runtime-test-123", stopped: true, remaining: false })
  assert.deepEqual(stopped, [child.pid])
  assert.ok(fixture.requests.some((request) => request.DetachFromSession?.attachment_id === "attachment-tui"))
  assert.ok(fixture.requests.some((request) => request.DetachFromSession?.attachment_id.startsWith("attachment-")))
  assert.deepEqual(controls, ["START_VIDEO", "STOP_VIDEO", "CLOSED"])
  assert.equal(fixture.clients[0].closed, 1)

  const failedTui = clientFixture()
  const failedChild = Object.assign(new EventEmitter(), { pid: 32124, exitCode: null, signalCode: null })
  const failedRuntime = await createRoomCoordinatedLoadRuntime({ plan: approvedPlan(), ...scratch }, {
    protocol: failedTui.protocol, relayToken: "fixture-relay-token", localKernelAuthEnvironment: {},
    spawn: () => failedChild,
    tuiProcess: {
      waitForAutomationRoom: async (state) => { state.attachmentId = "attachment-before-startup-failure"; throw new Error("fixture startup failure") },
      stopProcessGroup: async () => {},
      removeAutomationSocket: async () => {},
    },
  })
  await assert.rejects(failedRuntime.startTui(null, { id: "relay-tui", route: "relay" }), /fixture startup failure/)
  assert.deepEqual(failedTui.requests, [{ DetachFromSession: { attachment_id: "attachment-before-startup-failure" } }])
  assert.equal(failedTui.clients[0].closed, 1)
})
