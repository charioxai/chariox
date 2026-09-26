import assert from "node:assert/strict"
import test from "node:test"

import { ROOM_PLACEMENT_ROWS } from "./live-room-placement-matrix.mjs"
import {
  assertPlacementRelayReady,
  buildPlacementRows,
  cleanupOwnedPlacement,
  parseArgs,
  planOwnedCleanup,
} from "./live-room-placement-setup.mjs"

function placementFixture() {
  const homeKernel = { url: "/tmp/home.sock", kernelId: "home-kernel", machineId: "home-machine" }
  const remoteWorker = {
    kernelId: "remote-kernel",
    machineId: "remote-machine",
    acceptingRemoteLeases: true,
  }
  const rooms = ROOM_PLACEMENT_ROWS.map((spec, index) => {
    const remoteEnvironment = spec.environment === "remote"
    const environmentWorker = remoteEnvironment ? remoteWorker : homeKernel
    const roomId = "room-" + index
    const environmentSlice = {
      id: "environment-slice-" + index,
      owner_kernel_id: homeKernel.kernelId,
      owner_machine_id: homeKernel.machineId,
      worker_kernel_ref: environmentWorker.kernelId,
      worker_kernel_id: environmentWorker.kernelId,
      worker_machine_id: environmentWorker.machineId,
      display_mode: "headed",
      status: "running",
    }
    const agentSlice = ["other_local_slice", "different_slice_or_worker"].includes(spec.agent)
      ? {
          id: "agent-slice-" + index,
          owner_kernel_id: homeKernel.kernelId,
          owner_machine_id: homeKernel.machineId,
          worker_kernel_ref: homeKernel.kernelId,
          worker_kernel_id: homeKernel.kernelId,
          worker_machine_id: homeKernel.machineId,
          status: "running",
        }
      : null
    return {
      rowId: spec.id,
      room: { id: roomId, host_daemon_id: homeKernel.kernelId },
      environmentSlice,
      binding: {
        session_id: roomId,
        slice_id: environmentSlice.id,
        owner_kernel_id: homeKernel.kernelId,
        worker_kernel_ref: environmentSlice.worker_kernel_ref,
      },
      environment: {
        session_id: roomId,
        lifecycle: "ready",
        environment_id: "environment-" + index,
        focused_tab_id: "tab-" + index,
        tabs: [{ tab_id: "tab-" + index }],
      },
      agentSlice,
    }
  })
  return { homeKernel, remoteWorker, rooms }
}

function ownedManifest() {
  const invocationId = "setup-123"
  return {
    schema: "chariox.room_placement_matrix.config.v1",
    homeKernel: { kernelId: "home-kernel" },
    setup: {
      schema: "chariox.room_placement_matrix.setup.v1",
      ownership: {
        invocationId,
        baselineSessionIds: ["preexisting-room"],
        baselineSliceIds: ["preexisting-slice"],
        rooms: [{
          id: "owned-room",
          rowId: "home_environment_home_agent",
          alias: "room-placement-setup-123-home_environment_home_agent",
          workspaceId: "/workspace",
          ownerKernelId: "home-kernel",
          createdByInvocation: invocationId,
          environmentStartAttempted: true,
        }],
        slices: [{
          id: "owned-slice",
          role: "home_environment_home_agent-environment",
          name: "room-placement-setup-123-home_environment_home_agent-environment",
          ownerKernelId: "home-kernel",
          createdByInvocation: invocationId,
          startAttempted: true,
        }],
        pending: [],
      },
    },
  }
}

function inventories() {
  return {
    sessions: [
      {
        id: "owned-room",
        alias: "room-placement-setup-123-home_environment_home_agent",
        host_daemon_id: "home-kernel",
      },
      { id: "foreign-room", alias: "operator-session", host_daemon_id: "home-kernel" },
    ],
    slices: [
      {
        id: "owned-slice",
        name: "room-placement-setup-123-home_environment_home_agent-environment",
        owner_kernel_id: "home-kernel",
        session_id: "owned-room",
        status: "running",
      },
      {
        id: "foreign-slice",
        name: "operator-slice",
        owner_kernel_id: "home-kernel",
        session_id: "foreign-room",
        status: "running",
      },
    ],
  }
}

test("six-row setup records public placement identities and rejects worker substitution", () => {
  const fixture = placementFixture()
  const rows = buildPlacementRows({
    ...fixture,
    workspaceId: "/workspace/repository",
  })
  assert.deepEqual(rows.map((row) => row.id), ROOM_PLACEMENT_ROWS.map((row) => row.id))
  assert.deepEqual(rows[0].agentPlacement, { kind: "home_kernel" })
  assert.equal(rows[0].importFirst, false)
  assert.equal(rows[1].agentPlacement.kind, "slice_ref")
  assert.equal(rows[1].importFirst, true)
  assert.equal(rows[2].agentPlacement.kernelRef, fixture.remoteWorker.kernelId)
  assert.equal(rows[2].importFirst, false)
  assert.equal(rows[3].environmentWorkerKernelId, fixture.remoteWorker.kernelId)
  assert.equal(rows[4].agentWorkerKernelId, fixture.remoteWorker.kernelId)
  assert.equal(rows[5].agentPlacement.kind, "slice_ref")
  assert.equal(rows[5].importFirst, true)

  const substituted = placementFixture()
  const remoteEnvironment = substituted.rooms.find((room) => room.rowId === "remote_environment_home_agent")
  remoteEnvironment.environmentSlice.worker_machine_id = substituted.homeKernel.machineId
  assert.throws(() => buildPlacementRows({
    ...substituted,
    workspaceId: "/workspace/repository",
  }), /Environment worker machine was substituted/)

  const notReady = placementFixture()
  notReady.rooms[0].environment.lifecycle = "starting"
  assert.throws(() => buildPlacementRows({
    ...notReady,
    workspaceId: "/workspace/repository",
  }), /Room Environment is not ready/)
})

test("local-only setup selects one local-slice row without a remote worker", () => {
  const fixture = placementFixture()
  const selectedRoom = fixture.rooms.find((room) =>
    room.rowId === "home_environment_other_local_slice_agent")
  const rows = buildPlacementRows({
    homeKernel: fixture.homeKernel,
    rooms: [selectedRoom],
    workspaceId: "/workspace/repository",
    selectionMode: "local_only",
    foreignRoomId: "foreign-room-probe",
  })
  assert.equal(rows.length, 1)
  assert.equal(rows[0].id, "home_environment_other_local_slice_agent")
  assert.deepEqual(rows[0].agentPlacement, { kind: "slice_ref", sliceRef: selectedRoom.agentSlice.id })
  assert.equal(rows[0].importFirst, true)
  assert.throws(() => buildPlacementRows({
    homeKernel: fixture.homeKernel,
    rooms: [selectedRoom],
    workspaceId: "/workspace/repository",
    selectionMode: "local_only",
    foreignRoomId: selectedRoom.room.id,
  }), /foreignRoomId must identify a different Room/)
})

test("local-only command needs no remote worker identifiers", () => {
  const args = [
    "--create", "--local-only", "--config", "/tmp/placement-config.json",
    "--home-url", "/tmp/home.sock", "--workspace", "/workspace",
    "--worktree", "/workspace/worktree", "--provider", "codex",
    "--model", "configured-model", "--account-profile", "codex-1",
    "--effort", "high",
  ]
  const options = parseArgs(args)
  assert.equal(options.selectionMode, "local_only")
  assert.equal(options.remoteKernelId, null)
  assert.equal(options.remoteMachineId, null)
  assert.throws(() => parseArgs([...args, "--remote-machine-id", "remote"]),
    /only used by the full matrix/)
  assert.throws(() => parseArgs(["--cleanup", "--local-only", "--config", "/tmp/config.json"]),
    /only valid with --create/)
})

test("local-only setup accepts a disconnected relay while the remote matrix does not", () => {
  const disconnected = {
    connected: false,
    daemon_id: "home-kernel",
    machine_id: "home-machine",
  }
  assert.doesNotThrow(() => assertPlacementRelayReady(disconnected, "local_only"))
  assert.throws(() => assertPlacementRelayReady(disconnected, "full_matrix"),
    /not connected to its relay/)
  assert.doesNotThrow(() => assertPlacementRelayReady({ ...disconnected, connected: true }, "full_matrix"))
  assert.throws(() => assertPlacementRelayReady({ ...disconnected, connected: true }, "unknown"),
    /unsupported Room placement selection mode/)
})

test("cleanup plan is restricted to invocation-owned IDs and refuses an injected foreign ID", () => {
  const manifest = ownedManifest()
  const current = inventories()
  const plan = planOwnedCleanup({ manifest, ...current })
  assert.deepEqual(plan.rooms.map((room) => room.id), ["owned-room"])
  assert.deepEqual(plan.slices.map((slice) => slice.id), ["owned-slice"])

  const forged = structuredClone(manifest)
  forged.setup.ownership.slices.push({
    id: "foreign-slice",
    role: "operator",
    name: "operator-slice",
    ownerKernelId: "home-kernel",
  })
  assert.throws(() => planOwnedCleanup({ manifest: forged, ...current }),
    /absent from this invocation's ownership ledger/)

  const preexisting = structuredClone(manifest)
  preexisting.setup.ownership.baselineSessionIds.push("owned-room")
  assert.throws(() => planOwnedCleanup({ manifest: preexisting, ...current }),
    /Room that existed before this invocation/)

  const reassigned = inventories()
  reassigned.slices[0].session_id = "foreign-room"
  assert.throws(() => planOwnedCleanup({ manifest, ...reassigned }),
    /slice associated with a Room outside this invocation/)
})

test("cleanup sends stop and delete requests only for ledger-owned resources", async () => {
  const manifest = ownedManifest()
  const current = inventories()
  const sent = []
  const request = (kind, id = null, workspaceId = null) => ({ kind, id, workspaceId })
  const requests = {
    listSessionsRequest: () => request("list-sessions"),
    listSlicesRequest: () => request("list-slices"),
    stopRoomEnvironmentRequest: (id) => request("stop-room-environment", id),
    endSessionRequest: (id) => request("end-session", id),
    deleteSessionRequest: (id, workspaceId) => request("delete-session", id, workspaceId),
    stopSliceRequest: (id) => request("stop-slice", id),
    deleteSliceRequest: (id) => request("delete-slice", id),
  }
  const client = {
    async send(message) {
      sent.push(message)
      if (message.kind === "list-sessions") return { SessionsListed: { sessions: current.sessions } }
      if (message.kind === "list-slices") return { SlicesListed: { slices: current.slices } }
      if (message.kind === "stop-room-environment") {
        return { RoomEnvironmentUpdated: { environment: { session_id: message.id } } }
      }
      if (message.kind === "end-session") return { SessionEnded: {} }
      if (message.kind === "delete-session") return { SessionDeleted: { session: { id: message.id } } }
      if (message.kind === "stop-slice") return { SliceStopped: { slice: { id: message.id } } }
      if (message.kind === "delete-slice") return { SliceDeleted: { slice: { id: message.id } } }
      throw new Error("unexpected request " + message.kind)
    },
  }
  const result = await cleanupOwnedPlacement({ client, requests, manifest })
  assert.deepEqual(result, {
    roomIds: ["owned-room"],
    sliceIds: ["owned-slice"],
    pendingCreateCount: 0,
  })
  assert.deepEqual(sent.filter((item) => item.id).map((item) => item.id), [
    "owned-room",
    "owned-room",
    "owned-room",
    "owned-slice",
    "owned-slice",
  ])
  assert.ok(!sent.some((item) => item.id === "foreign-room" || item.id === "foreign-slice"))
})
