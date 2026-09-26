#!/usr/bin/env node

import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { lstat, mkdir, open, readFile, rename, realpath, stat } from "node:fs/promises"
import path from "node:path"
import { pathToFileURL } from "node:url"

import {
  LOCAL_ONLY_PLACEMENT_ROW_ID,
  LOCAL_ONLY_SELECTION_MODE,
  ROOM_PLACEMENT_ROWS,
  validateRoomPlacementMatrixConfig,
} from "./live-room-placement-matrix.mjs"

const repoRoot = path.resolve(import.meta.dirname, "../../..")
const configSchema = "chariox.room_placement_matrix.config.v1"
const setupSchema = "chariox.room_placement_matrix.setup.v1"
const rowCount = 6
const sliceCount = 8
const maxTimeoutMs = 45 * 60_000
const defaultTimeoutMs = 30 * 60_000
const viewport = Object.freeze({
  css_width: 1280,
  css_height: 800,
  device_scale_factor: 1,
  desktop_pixel_width: 1280,
  desktop_pixel_height: 800,
})
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

export function buildPlacementRows({
  homeKernel,
  remoteWorker,
  rooms,
  workspaceId,
  selectionMode = "full_matrix",
  foreignRoomId,
}) {
  requireText(homeKernel?.kernelId, "homeKernel.kernelId")
  requireText(homeKernel?.machineId, "homeKernel.machineId")
  assert.ok(selectionMode === "full_matrix" || selectionMode === LOCAL_ONLY_SELECTION_MODE,
    "unsupported Room placement selection mode")
  const selectedRows = selectionMode === LOCAL_ONLY_SELECTION_MODE
    ? ROOM_PLACEMENT_ROWS.filter((row) => row.id === LOCAL_ONLY_PLACEMENT_ROW_ID)
    : ROOM_PLACEMENT_ROWS
  if (selectionMode === "full_matrix") {
    requireText(remoteWorker?.kernelId, "remoteWorker.kernelId")
    requireText(remoteWorker?.machineId, "remoteWorker.machineId")
    assert.equal(remoteWorker.acceptingRemoteLeases, true,
      "selected remote worker is not accepting remote leases")
    assert.notEqual(remoteWorker.kernelId, homeKernel.kernelId,
      "remote worker kernel must differ from the home kernel")
    assert.notEqual(remoteWorker.machineId, homeKernel.machineId,
      "remote worker machine must differ from the home machine")
  } else {
    requireText(foreignRoomId, "foreignRoomId")
  }
  requireText(workspaceId, "workspaceId")
  assert.ok(Array.isArray(rooms) && rooms.length === selectedRows.length,
    selectionMode === LOCAL_ONLY_SELECTION_MODE
      ? "local-only setup must supply exactly one created placement Room"
      : "setup must supply exactly six created Room records")
  if (selectionMode === "full_matrix") {
    assert.equal(ROOM_PLACEMENT_ROWS.length, rowCount,
      "placement runner must still define exactly six rows")
  }
  const byRowId = new Map(rooms.map((room) => [room.rowId, room]))
  assert.equal(byRowId.size, selectedRows.length, "each placement row requires a different Room")
  const preparedLocalRoom = byRowId.get(LOCAL_ONLY_PLACEMENT_ROW_ID)
  if (selectionMode === LOCAL_ONLY_SELECTION_MODE) {
    assert.notEqual(foreignRoomId, preparedLocalRoom?.room?.id,
      "foreignRoomId must identify a different Room")
  }

  return selectedRows.map((spec) => {
    const prepared = byRowId.get(spec.id)
    assert.ok(prepared, "created Room setup missing " + spec.id)
    const roomId = requireText(prepared.room?.id, spec.id + ".room.id")
    const environmentSlice = prepared.environmentSlice
    const binding = prepared.binding
    const environment = prepared.environment
    const agentSlice = prepared.agentSlice
    assert.equal(prepared.room.host_daemon_id, homeKernel.kernelId,
      spec.id + " Room must be owned by the selected home kernel")
    assert.equal(environmentSlice?.owner_kernel_id, homeKernel.kernelId,
      spec.id + " Environment slice must be home-owned")
    assert.equal(environmentSlice?.owner_machine_id, homeKernel.machineId,
      spec.id + " Environment slice owner machine mismatch")
    assert.equal(environmentSlice?.display_mode, "headed",
      spec.id + " Environment slice must be headed")
    assert.equal(environmentSlice?.status, "running",
      spec.id + " Environment slice must be running")
    assert.equal(binding?.session_id, roomId, spec.id + " Environment binding Room mismatch")
    assert.equal(binding?.slice_id, environmentSlice.id, spec.id + " Environment binding slice mismatch")
    assert.equal(binding?.owner_kernel_id, homeKernel.kernelId,
      spec.id + " Environment binding must be home-owned")
    assert.equal(binding?.worker_kernel_ref, environmentSlice.worker_kernel_ref,
      spec.id + " Environment binding worker differs from its public slice")
    assert.equal(environment?.session_id, roomId, spec.id + " Environment belongs to another Room")
    assert.equal(environment?.lifecycle, "ready",
      spec.id + " Room Environment is not ready")
    const environmentId = requireText(environment?.environment_id, spec.id + ".environmentId")
    const tabId = requireText(environment?.focused_tab_id, spec.id + ".focusedTabId")
    assert.ok(Array.isArray(environment.tabs)
      && environment.tabs.some((tab) => tab?.tab_id === tabId),
    spec.id + " focused Browser Tab is absent from the public Environment snapshot")

    const environmentWorker = spec.environment === "home" ? homeKernel : remoteWorker
    const observedEnvironmentWorker = sliceWorker(environmentSlice)
    assert.equal(observedEnvironmentWorker.kernelId, environmentWorker.kernelId,
      spec.id + " Environment worker kernel was substituted")
    assert.equal(observedEnvironmentWorker.machineId, environmentWorker.machineId,
      spec.id + " Environment worker machine was substituted")

    let agentPlacement
    let agentWorker
    if (spec.agent === "home") {
      agentPlacement = { kind: "home_kernel" }
      agentWorker = homeKernel
    } else if (spec.agent === "remote_worker" || spec.agent === "environment_worker") {
      agentPlacement = { kind: "kernel_ref", kernelRef: remoteWorker.kernelId }
      agentWorker = remoteWorker
    } else {
      assert.ok(agentSlice, spec.id + " requires a separate local agent slice")
      assert.notEqual(agentSlice.id, environmentSlice.id,
        spec.id + " agent slice must differ from its Environment slice")
      assert.equal(agentSlice.owner_kernel_id, homeKernel.kernelId,
        spec.id + " agent slice must be home-owned")
      assert.equal(agentSlice.owner_machine_id, homeKernel.machineId,
        spec.id + " agent slice owner machine mismatch")
      assert.equal(agentSlice.status, "running", spec.id + " agent slice must be running")
      agentPlacement = { kind: "slice_ref", sliceRef: requireText(agentSlice.id, spec.id + ".agentSlice.id") }
      agentWorker = sliceWorker(agentSlice)
      assert.equal(agentWorker.kernelId, homeKernel.kernelId,
        spec.id + " local agent worker kernel was substituted")
      assert.equal(agentWorker.machineId, homeKernel.machineId,
        spec.id + " local agent worker machine was substituted")
    }

    return {
      id: spec.id,
      roomId,
      environmentSliceRef: requireText(environmentSlice.id, spec.id + ".environmentSliceRef"),
      environmentId,
      tabId,
      environmentWorkerKernelId: environmentWorker.kernelId,
      environmentWorkerMachineId: environmentWorker.machineId,
      agentWorkerKernelId: agentWorker.kernelId,
      agentWorkerMachineId: agentWorker.machineId,
      agentPlacement,
      importFirst: spec.agent === "other_local_slice"
        || spec.agent === "different_slice_or_worker",
      workspaceId,
    }
  })
}

export function planOwnedCleanup({ manifest, sessions, slices }) {
  const ownership = manifest?.setup?.ownership
  assert.equal(manifest?.schema, configSchema, "unsupported placement setup config")
  assert.equal(manifest?.setup?.schema, setupSchema, "missing placement setup ownership ledger")
  const invocationId = requireText(ownership?.invocationId, "setup.ownership.invocationId")
  const homeKernelId = requireText(manifest?.homeKernel?.kernelId, "homeKernel.kernelId")
  assert.ok(Array.isArray(ownership.rooms) && Array.isArray(ownership.slices)
    && Array.isArray(ownership.pending),
    "setup ownership ledger must list created Rooms and slices")
  assert.ok(Array.isArray(ownership.baselineSessionIds) && Array.isArray(ownership.baselineSliceIds),
    "setup ownership ledger must include pre-mutation public inventory IDs")
  const sessionById = uniqueInventory(sessions, "Room")
  const sliceById = uniqueInventory(slices, "slice")
  const ownedRoomIds = new Set()
  const rooms = []
  const ownedSliceIds = new Set()
  const cleanupSlices = []

  for (const owned of ownership.rooms) {
    assert.equal(owned?.createdByInvocation, invocationId,
      "refusing cleanup of a Room absent from this invocation's ownership ledger")
    const id = requireText(owned.id, "owned Room id")
    assert.ok(!ownedRoomIds.has(id), "duplicate Room in ownership ledger")
    assert.ok(!ownership.baselineSessionIds.includes(id),
      "refusing cleanup of a Room that existed before this invocation")
    ownedRoomIds.add(id)
    assert.equal(owned.ownerKernelId, homeKernelId, "owned Room kernel differs from setup home")
    assert.equal(owned.alias, "room-placement-" + invocationId + "-" + owned.rowId,
      "owned Room alias does not match its invocation marker")
    const current = sessionById.get(id)
    if (!current) continue
    assert.equal(current.host_daemon_id, homeKernelId, "refusing cleanup after Room owner changed")
    assert.equal(current.alias, owned.alias, "refusing cleanup after Room alias changed")
    rooms.push({ ...owned, current })
  }

  for (const owned of ownership.slices) {
    assert.equal(owned?.createdByInvocation, invocationId,
      "refusing cleanup of a slice absent from this invocation's ownership ledger")
    const id = requireText(owned.id, "owned slice id")
    assert.ok(!ownedSliceIds.has(id), "duplicate slice in ownership ledger")
    assert.ok(!ownership.baselineSliceIds.includes(id),
      "refusing cleanup of a slice that existed before this invocation")
    ownedSliceIds.add(id)
    assert.equal(owned.ownerKernelId, homeKernelId, "owned slice kernel differs from setup home")
    assert.equal(owned.name, "room-placement-" + invocationId + "-" + owned.role,
      "owned slice name does not match its invocation marker")
    const current = sliceById.get(id)
    if (!current) continue
    assert.equal(current.owner_kernel_id, homeKernelId, "refusing cleanup after slice owner changed")
    assert.equal(current.name, owned.name, "refusing cleanup after slice name changed")
    const relatedRooms = new Set([
      current.session_id,
      current.environment_session_id,
      ...(current.session_ids ?? []),
    ].filter((value) => typeof value === "string" && value.length > 0))
    assert.ok([...relatedRooms].every((roomId) => ownedRoomIds.has(roomId)),
      "refusing to clean a slice associated with a Room outside this invocation")
    cleanupSlices.push({ ...owned, current })
  }
  return { rooms, slices: cleanupSlices, pendingCreates: ownership.pending.length }
}

export async function cleanupOwnedPlacement({ client, requests, manifest, timeoutMs = 60_000 }) {
  const deadline = Date.now() + timeoutMs
  const send = (request) => sendBounded(client, request, remaining(deadline))
  const [sessionResponse, sliceResponse] = await Promise.all([
    send(requests.listSessionsRequest()),
    send(requests.listSlicesRequest()),
  ])
  const sessions = responseVariant(sessionResponse, "SessionsListed").sessions
  const slices = responseVariant(sliceResponse, "SlicesListed").slices
  const targets = planOwnedCleanup({ manifest, sessions, slices })
  const cleaned = { roomIds: [], sliceIds: [] }
  for (const room of [...targets.rooms].reverse()) {
    if (room.environmentStartAttempted) {
      const response = responseVariant(await send(requests.stopRoomEnvironmentRequest(room.id)),
        "RoomEnvironmentUpdated")
      assert.equal(response.environment?.session_id, room.id, "Room Environment stop returned another Room")
    }
    responseVariant(await send(requests.endSessionRequest(room.id)), "SessionEnded")
    const deleted = responseVariant(await send(
      requests.deleteSessionRequest(room.id, room.workspaceId)), "SessionDeleted").session
    assert.equal(deleted?.id, room.id, "Room deletion returned another session")
    cleaned.roomIds.push(room.id)
  }
  for (const slice of [...targets.slices].reverse()) {
    if (slice.current.status === "running" || slice.startAttempted) {
      const stopped = responseVariant(await send(requests.stopSliceRequest(slice.id)), "SliceStopped").slice
      assert.equal(stopped?.id, slice.id, "slice stop returned another slice")
    }
    const deleted = responseVariant(await send(
      requests.deleteSliceRequest(slice.id)), "SliceDeleted").slice
    assert.equal(deleted?.id, slice.id, "slice deletion returned another slice")
    cleaned.sliceIds.push(slice.id)
  }
  cleaned.pendingCreateCount = targets.pendingCreates
  return cleaned
}

export function parseArgs(argv) {
  const mode = argv[0]
  assert.ok(mode === "--create" || mode === "--cleanup",
    "select --create [--local-only] or --cleanup")
  const values = new Map()
  let localOnly = false
  for (let index = 1; index < argv.length; index += 1) {
    const name = argv[index]
    if (name === "--local-only") {
      assert.equal(localOnly, false, "duplicate option --local-only")
      localOnly = true
      continue
    }
    const value = argv[index + 1]
    assert.ok(name?.startsWith("--") && value && !value.startsWith("--"), "missing value for " + name)
    assert.ok(!values.has(name), "duplicate option " + name)
    values.set(name, value)
    index += 1
  }
  assert.ok(mode !== "--cleanup" || !localOnly, "--local-only is only valid with --create")
  const configPath = requiredOption(values, "--config")
  assert.ok(path.isAbsolute(configPath), "--config must be absolute")
  const options = { mode, configPath: path.resolve(configPath) }
  if (mode === "--cleanup") return options
  options.selectionMode = localOnly ? LOCAL_ONLY_SELECTION_MODE : "full_matrix"
  Object.assign(options, {
    homeUrl: requiredOption(values, "--home-url"),
    workspaceId: requiredOption(values, "--workspace"),
    worktreeId: requiredOption(values, "--worktree"),
    remoteMachineId: localOnly ? null : requiredOption(values, "--remote-machine-id"),
    remoteKernelId: localOnly ? null : requiredOption(values, "--remote-kernel-id"),
    provider: requiredOption(values, "--provider"),
    model: requiredOption(values, "--model"),
    accountProfile: requiredOption(values, "--account-profile"),
    effort: requiredOption(values, "--effort"),
    timeoutMs: values.has("--timeout-ms") ? Number(values.get("--timeout-ms")) : defaultTimeoutMs,
  })
  if (localOnly) {
    assert.ok(!values.has("--remote-machine-id") && !values.has("--remote-kernel-id"),
      "--remote-machine-id and --remote-kernel-id are only used by the full matrix")
  }
  assert.ok(isLocalKernelEndpoint(options.homeUrl), "--home-url must identify a local kernel endpoint")
  assert.ok(path.isAbsolute(options.workspaceId) && path.isAbsolute(options.worktreeId),
    "--workspace and --worktree must be absolute paths")
  assert.ok(["codex", "claude", "opencode"].includes(options.provider),
    "--provider must select an official provider")
  assert.ok(Number.isSafeInteger(options.timeoutMs) && options.timeoutMs > 0
    && options.timeoutMs <= maxTimeoutMs, "--timeout-ms exceeds the bounded setup limit")
  return options
}

export function assertPlacementRelayReady(status, selectionMode) {
  if (selectionMode === LOCAL_ONLY_SELECTION_MODE) return
  assert.equal(selectionMode, "full_matrix", "unsupported Room placement selection mode")
  assert.equal(status.connected, true, "selected home kernel is not connected to its relay")
}

async function createPlacementSetup(options, LocalIpcClient, requests) {
  const configPath = await preparePrivateConfigPath(options.configPath)
  const client = new LocalIpcClient(options.homeUrl)
  let manifest
  let configCreated = false
  const invocationId = randomUUID()
  const deadline = Date.now() + options.timeoutMs
  try {
    const status = responseVariant(await sendBounded(client,
      requests.relayStatusRequest(), remaining(deadline)), "RelayStatus").status
    assertPlacementRelayReady(status, options.selectionMode)
    const homeKernel = {
      url: options.homeUrl,
      kernelId: requireText(status.daemon_id, "RelayStatus.daemon_id"),
      machineId: requireText(status.machine_id, "RelayStatus.machine_id"),
    }
    const remoteWorker = options.selectionMode === LOCAL_ONLY_SELECTION_MODE
      ? null
      : await resolveRemoteWorker({
          client, requests, machineId: options.remoteMachineId, kernelId: options.remoteKernelId,
          homeKernel, deadline,
        })
    const priorSessions = responseVariant(await sendBounded(client,
      requests.listSessionsRequest(), remaining(deadline)), "SessionsListed").sessions
    const priorSlices = responseVariant(await sendBounded(client,
      requests.listSlicesRequest(), remaining(deadline)), "SlicesListed").slices
    assert.ok(Array.isArray(priorSessions) && Array.isArray(priorSlices),
      "home kernel did not return public Room and slice inventories")
    manifest = {
      schema: configSchema,
      homeKernel,
      selectionMode: options.selectionMode,
      provider: {
        provider: options.provider,
        model: options.model,
        accountProfile: options.accountProfile,
        effort: options.effort,
      },
      rows: [],
      setup: {
        schema: setupSchema,
        status: "creating",
        invocationId,
        createdAt: new Date().toISOString(),
        ...(remoteWorker ? { remoteWorker } : {}),
        ownership: {
          invocationId,
          homeKernelId: homeKernel.kernelId,
          baselineSessionIds: priorSessions.map((session) => session.id),
          baselineSliceIds: priorSlices.map((slice) => slice.id),
          rooms: [],
          slices: [],
          pending: [],
        },
      },
    }
    await writePrivateConfig(configPath, manifest, { create: true })
    configCreated = true
    const roomSetups = []
    const selectedRows = options.selectionMode === LOCAL_ONLY_SELECTION_MODE
      ? ROOM_PLACEMENT_ROWS.filter((row) => row.id === LOCAL_ONLY_PLACEMENT_ROW_ID)
      : ROOM_PLACEMENT_ROWS
    for (const spec of selectedRows) {
      assert.ok(roomSetups.length < selectedRows.length, "setup exceeded the selected Room resource limit")
      const alias = "room-placement-" + invocationId + "-" + spec.id
      const room = await createOwnedRoom({
        client, requests, manifest, configPath, alias,
        workspaceId: options.workspaceId, worktreeId: options.worktreeId, deadline,
      })
      const environmentWorker = spec.environment === "home" ? homeKernel : remoteWorker
      const environmentSlice = await createOwnedSlice({
        client, requests, manifest, configPath,
        role: spec.id + "-environment",
        roomId: room.id,
        workspaceId: options.workspaceId,
        worktreeId: options.worktreeId,
        workerKernelRef: spec.environment === "home" ? null : remoteWorker.kernelId,
        displayMode: "headed", deadline,
      })
      const binding = responseVariant(await sendBounded(client,
        requests.bindRoomEnvironmentSliceRequest(room.id, environmentSlice.id), remaining(deadline)),
      "RoomEnvironmentSlice").binding
      assertRoomBinding(binding, room.id, environmentSlice, homeKernel.kernelId)
      const runningEnvironmentSlice = await startOwnedSlice({
        client, requests, manifest, configPath, sliceRecord: environmentSlice,
        expectedWorker: environmentWorker, deadline,
      })
      const ownedRoom = manifest.setup.ownership.rooms.find((item) => item.id === room.id)
      ownedRoom.environmentStartAttempted = true
      await writePrivateConfig(configPath, manifest)
      responseVariant(await sendBounded(client,
        requests.startRoomEnvironmentRequest(room.id, viewport), remaining(deadline)), "RoomEnvironmentUpdated")
      const environment = await waitForEnvironment({
        client, requests, roomId: room.id, deadline,
      })
      let agentSlice = null
      if (spec.agent === "other_local_slice" || spec.agent === "different_slice_or_worker") {
        agentSlice = await createOwnedSlice({
          client, requests, manifest, configPath,
          role: spec.id + "-agent",
          roomId: room.id,
          workspaceId: options.workspaceId,
          worktreeId: options.worktreeId,
          workerKernelRef: null,
          displayMode: "headless", deadline,
        })
        agentSlice = await startOwnedSlice({
          client, requests, manifest, configPath, sliceRecord: agentSlice,
          expectedWorker: homeKernel, deadline,
        })
      }
      roomSetups.push({
        rowId: spec.id,
        room,
        environmentSlice: runningEnvironmentSlice,
        binding,
        environment,
        agentSlice,
      })
    }
    if (options.selectionMode === LOCAL_ONLY_SELECTION_MODE) {
      const foreignRoom = await createOwnedRoom({
        client, requests, manifest, configPath,
        alias: "room-placement-" + invocationId + "-foreign-room-probe",
        workspaceId: options.workspaceId,
        worktreeId: options.worktreeId,
        deadline,
      })
      manifest.foreignRoomId = requireText(foreignRoom.id, "foreign Room probe id")
    }
    const expectedRoomCount = options.selectionMode === LOCAL_ONLY_SELECTION_MODE ? 2 : rowCount
    const expectedSliceCount = options.selectionMode === LOCAL_ONLY_SELECTION_MODE ? 2 : sliceCount
    assert.equal(manifest.setup.ownership.rooms.length, expectedRoomCount,
      "setup created an unexpected number of owned Rooms")
    assert.equal(manifest.setup.ownership.slices.length, expectedSliceCount,
      "setup created an unexpected number of owned slices")
    manifest.rows = buildPlacementRows({
      homeKernel,
      remoteWorker,
      rooms: roomSetups,
      workspaceId: options.workspaceId,
      selectionMode: options.selectionMode,
      foreignRoomId: manifest.foreignRoomId,
    })
    manifest.setup.status = "ready_for_official_runner"
    manifest.setup.readyAt = new Date().toISOString()
    validateRoomPlacementMatrixConfig(manifest)
    await writePrivateConfig(configPath, manifest)
    return manifest
  } catch (error) {
    if (manifest && configCreated) {
      manifest.setup.status = "setup_failed_cleanup_attempted"
      manifest.setup.failureCode = "placement_setup_failed"
      await writePrivateConfig(configPath, manifest).catch(() => {})
      let recoveryIncomplete = false
      let cleanupIncomplete = false
      try {
        await recoverPendingCreates({ client, requests, manifest, configPath, timeoutMs: 30_000 })
      } catch {
        recoveryIncomplete = true
      }
      try {
        manifest.setup.cleanupResult = await cleanupOwnedPlacement({
          client, requests, manifest, timeoutMs: 120_000,
        })
      } catch {
        cleanupIncomplete = true
      }
      manifest.setup.status = recoveryIncomplete || cleanupIncomplete
        || manifest.setup.ownership.pending.length > 0
        ? "setup_failed_cleanup_incomplete"
        : "setup_failed_cleaned"
      await writePrivateConfig(configPath, manifest).catch(() => {})
    }
    throw new Error("placement setup failed; inspect the private ownership ledger and public inventory", {
      cause: error,
    })
  } finally {
    await client.close?.().catch(() => {})
  }
}

async function createOwnedRoom({ client, requests, manifest, configPath, alias, workspaceId, worktreeId, deadline }) {
  const ownership = manifest.setup.ownership
  const rowId = alias.slice(("room-placement-" + ownership.invocationId + "-").length)
  ownership.pending.push({ kind: "room", alias, rowId, workspaceId, baselineIds: ownership.baselineSessionIds })
  await writePrivateConfig(configPath, manifest)
  let response
  try {
    response = await sendBounded(client, requests.createSessionRequest(workspaceId, worktreeId, alias),
      remaining(deadline))
  } catch (error) {
    await recoverPendingCreates({ client, requests, manifest, configPath, timeoutMs: 30_000 })
    throw error
  }
  const room = responseVariant(response, "SessionCreated").session
  const id = requireText(room?.id, "SessionCreated.session.id")
  assert.equal(room.alias, alias, "created Room alias differs from its invocation marker")
  assert.equal(room.host_daemon_id, manifest.homeKernel.kernelId, "created Room belongs to another kernel")
  const pending = ownership.pending.find((item) => item.kind === "room" && item.alias === alias)
  assert.ok(pending, "created Room lost its persisted ownership intent")
  ownership.rooms.push({
    id, alias, rowId, workspaceId,
    ownerKernelId: manifest.homeKernel.kernelId,
    createdByInvocation: ownership.invocationId,
    environmentStartAttempted: false,
  })
  ownership.pending = ownership.pending.filter((item) => item !== pending)
  await writePrivateConfig(configPath, manifest)
  return room
}

async function createOwnedSlice({
  client, requests, manifest, configPath, role, roomId, workspaceId, worktreeId,
  workerKernelRef, displayMode, deadline,
}) {
  const ownership = manifest.setup.ownership
  const name = "room-placement-" + ownership.invocationId + "-" + role
  ownership.pending.push({
    kind: "slice", name, role, roomId, workerKernelRef,
    baselineIds: ownership.baselineSliceIds,
  })
  await writePrivateConfig(configPath, manifest)
  let response
  try {
    response = await sendBounded(client, requests.createSliceRequest({
      name,
      backend: "local_docker",
      displayMode,
      ...(displayMode === "headed" ? { displayBackend: "selkies" } : {}),
      workspaceId,
      worktreeId,
      workspaceMount: worktreeId,
      workerKernelRef,
      base: "clean",
    }), remaining(deadline))
  } catch (error) {
    await recoverPendingCreates({ client, requests, manifest, configPath, timeoutMs: 30_000 })
    throw error
  }
  const slice = responseVariant(response, "SliceCreated").slice
  const id = requireText(slice?.id, "SliceCreated.slice.id")
  assert.equal(slice.name, name, "created slice name differs from its invocation marker")
  assert.equal(slice.owner_kernel_id, manifest.homeKernel.kernelId, "created slice belongs to another kernel")
  const pending = ownership.pending.find((item) => item.kind === "slice" && item.name === name)
  assert.ok(pending, "created slice lost its persisted ownership intent")
  ownership.slices.push({
    id, name, role, roomId, workerKernelRef,
    ownerKernelId: manifest.homeKernel.kernelId,
    createdByInvocation: ownership.invocationId,
    startAttempted: false,
  })
  ownership.pending = ownership.pending.filter((item) => item !== pending)
  await writePrivateConfig(configPath, manifest)
  return slice
}

async function startOwnedSlice({ client, requests, manifest, configPath, sliceRecord, expectedWorker, deadline }) {
  const owned = manifest.setup.ownership.slices.find((item) => item.id === sliceRecord.id)
  assert.ok(owned, "refusing to start a slice not owned by this invocation")
  owned.startAttempted = true
  await writePrivateConfig(configPath, manifest)
  responseVariant(await sendBounded(client, requests.startSliceRequest(sliceRecord.id),
    remaining(deadline)), "SliceStarted")
  while (Date.now() < deadline) {
    const current = responseVariant(await sendBounded(client,
      requests.getSliceRequest(sliceRecord.id), remaining(deadline)), "Slice").slice
    assert.equal(current?.id, sliceRecord.id, "public slice lookup returned a different slice")
    assert.equal(current.owner_kernel_id, manifest.homeKernel.kernelId, "started slice owner changed")
    if (current.status === "running") {
      assert.equal(current.display_mode, sliceRecord.display_mode, "started slice display mode changed")
      const worker = sliceWorker(current)
      assert.equal(worker.kernelId, expectedWorker.kernelId, "started slice worker kernel was substituted")
      assert.equal(worker.machineId, expectedWorker.machineId, "started slice worker machine was substituted")
      return current
    }
    assert.ok(!["failed", "error"].includes(current.status),
      "slice " + sliceRecord.id + " failed before reaching running state")
    await sleep(Math.min(500, remaining(deadline)))
  }
  throw new Error("slice " + sliceRecord.id + " did not reach running state before timeout")
}

async function waitForEnvironment({ client, requests, roomId, deadline }) {
  while (Date.now() < deadline) {
    const environment = responseVariant(await sendBounded(client,
      requests.getRoomEnvironmentStateRequest(roomId), remaining(deadline)), "RoomEnvironmentState").environment
    assert.equal(environment?.session_id, roomId, "Room Environment state belongs to a different Room")
    const tabId = environment?.focused_tab_id
    if (environment?.lifecycle === "ready" && typeof tabId === "string" && tabId.trim()
      && Array.isArray(environment.tabs)
      && environment.tabs.some((tab) => tab?.tab_id === tabId)) return environment
    assert.notEqual(environment?.lifecycle, "failed",
      "Room Environment entered a failed lifecycle state")
    await sleep(Math.min(500, remaining(deadline)))
  }
  throw new Error("Room Environment did not expose a focused public Browser Tab before timeout")
}

async function resolveRemoteWorker({ client, requests, machineId, kernelId, homeKernel, deadline }) {
  const machines = responseVariant(await sendBounded(client,
    requests.listRemoteMachinesRequest(), remaining(deadline)), "RemoteMachinesListed").machines
  assert.ok(Array.isArray(machines), "public remote machine inventory is unavailable")
  const machine = machines.find((entry) => entry?.machine_id === machineId)
  assert.ok(machine, "remote machine is not in the public inventory")
  assert.equal(machine.online, true, "selected remote machine is offline")
  assert.equal(machine.pending, false, "selected remote machine has not completed pairing")
  assert.equal(machine.trust_status, "approved", "selected remote machine is not approved")
  const kernels = responseVariant(await sendBounded(client,
    requests.listRemoteMachineKernelsRequest(machineId), remaining(deadline)), "RemoteMachineKernelsListed").kernels
  assert.ok(Array.isArray(kernels), "public remote kernel inventory is unavailable")
  const kernel = kernels.find((entry) => entry?.kernel_id === kernelId)
  assert.ok(kernel, "selected remote kernel is not registered on the selected machine")
  assert.equal(kernel.machine_id, machineId, "remote kernel machine identity was substituted")
  assert.equal(kernel.accepting_remote_leases, true, "selected remote kernel is not accepting leases")
  assert.notEqual(kernel.kernel_id, homeKernel.kernelId, "selected remote worker is the home kernel")
  assert.notEqual(kernel.machine_id, homeKernel.machineId, "selected remote worker is the home machine")
  return {
    kernelId: requireText(kernel.kernel_id, "remote kernel id"),
    machineId: requireText(kernel.machine_id, "remote machine id"),
    acceptingRemoteLeases: true,
  }
}

async function recoverPendingCreates({ client, requests, manifest, configPath, timeoutMs = 60_000 }) {
  const ownership = manifest.setup.ownership
  for (const pending of [...ownership.pending]) {
    const isRoom = pending.kind === "room"
    const response = await sendBounded(client,
      isRoom ? requests.listSessionsRequest() : requests.listSlicesRequest(), timeoutMs)
    const inventory = isRoom
      ? responseVariant(response, "SessionsListed").sessions
      : responseVariant(response, "SlicesListed").slices
    const candidates = inventory.filter((record) => !pending.baselineIds.includes(record.id)
      && (isRoom
        ? record.alias === pending.alias && record.host_daemon_id === manifest.homeKernel.kernelId
        : record.name === pending.name && record.owner_kernel_id === manifest.homeKernel.kernelId))
    assert.ok(candidates.length <= 1, "ambiguous pending creation; refusing resource cleanup")
    const candidate = candidates[0]
    assert.ok(candidate, "pending creation is not yet visible in public inventory; ownership remains uncertain")
    if (candidate) {
      if (isRoom) {
        ownership.rooms.push({
          id: candidate.id,
          alias: pending.alias,
          rowId: pending.rowId,
          workspaceId: pending.workspaceId,
          ownerKernelId: manifest.homeKernel.kernelId,
          createdByInvocation: ownership.invocationId,
          environmentStartAttempted: false,
        })
      } else {
        ownership.slices.push({
          id: candidate.id,
          name: pending.name,
          role: pending.role,
          roomId: pending.roomId,
          workerKernelRef: pending.workerKernelRef,
          ownerKernelId: manifest.homeKernel.kernelId,
          createdByInvocation: ownership.invocationId,
          startAttempted: false,
        })
      }
    }
    ownership.pending = ownership.pending.filter((item) => item !== pending)
    await writePrivateConfig(configPath, manifest)
  }
}

async function preparePrivateConfigPath(configPath) {
  assert.ok(path.isAbsolute(configPath), "config path must be absolute")
  const resolved = path.resolve(configPath)
  assert.ok(!isWithin(resolved, repoRoot), "private setup config must be outside the repository")
  const realRepoRoot = await realpath(repoRoot)
  let ancestor = path.dirname(resolved)
  let realAncestor
  while (!realAncestor) {
    try {
      realAncestor = await realpath(ancestor)
    } catch (error) {
      if (error?.code !== "ENOENT") throw error
      const parent = path.dirname(ancestor)
      assert.notEqual(parent, ancestor, "config path has no existing ancestor")
      ancestor = parent
    }
  }
  assert.ok(!isWithin(realAncestor, realRepoRoot),
    "config directory path resolves inside the repository")
  await mkdir(path.dirname(resolved), { recursive: true, mode: 0o700 })
  const parent = await stat(path.dirname(resolved))
  assert.ok(parent.isDirectory() && (parent.mode & 0o077) === 0,
    "config directory must be private (0700 or stricter)")
  const actualParent = await realpath(path.dirname(resolved))
  assert.ok(!isWithin(actualParent, realRepoRoot), "config directory resolves inside the repository")
  return resolved
}

async function writePrivateConfig(configPath, value, { create = false } = {}) {
  const json = JSON.stringify(value, null, 2) + "\n"
  if (create) {
    const file = await open(configPath, "wx", 0o600)
    try {
      await file.writeFile(json, "utf8")
      await file.sync()
    } finally {
      await file.close()
    }
    return
  }
  const temporary = configPath + "." + process.pid + "." + randomUUID() + ".tmp"
  const file = await open(temporary, "wx", 0o600)
  try {
    await file.writeFile(json, "utf8")
    await file.sync()
  } finally {
    await file.close()
  }
  await rename(temporary, configPath)
}

async function readPrivateConfig(configPath) {
  const info = await lstat(configPath)
  assert.ok(info.isFile() && (info.mode & 0o077) === 0,
    "setup config must be a private regular file (0600 or stricter)")
  const manifest = JSON.parse(await readFile(configPath, "utf8"))
  assert.equal(manifest?.schema, configSchema, "unsupported placement setup config")
  assert.equal(manifest?.setup?.schema, setupSchema, "setup config has no ownership ledger")
  return manifest
}

async function sendBounded(client, request, timeoutMs) {
  let timer
  try {
    return await Promise.race([
      client.send(request),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("public kernel request timed out")), timeoutMs)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

function assertRoomBinding(binding, roomId, slice, ownerKernelId) {
  assert.equal(binding?.session_id, roomId, "Room Environment binding Room mismatch")
  assert.equal(binding?.slice_id, slice.id, "Room Environment binding slice mismatch")
  assert.equal(binding?.owner_kernel_id, ownerKernelId, "Room Environment binding owner mismatch")
  assert.equal(binding?.worker_kernel_ref, slice.worker_kernel_ref, "Room Environment binding worker mismatch")
}

function sliceWorker(slice) {
  return {
    kernelId: requireText(slice?.worker_kernel_id ?? slice?.owner_kernel_id, "slice worker kernel id"),
    machineId: requireText(slice?.worker_machine_id ?? slice?.owner_machine_id, "slice worker machine id"),
  }
}

function responseVariant(response, key) {
  assert.ok(response && Object.hasOwn(response, key), "public kernel response is missing " + key)
  return response[key]
}

function uniqueInventory(records, label) {
  assert.ok(Array.isArray(records), "public " + label + " inventory is malformed")
  const byId = new Map()
  for (const record of records) {
    const id = requireText(record?.id, label + " inventory id")
    assert.ok(!byId.has(id), "duplicate id in public " + label + " inventory")
    byId.set(id, record)
  }
  return byId
}

function requireText(value, label) {
  assert.ok(typeof value === "string" && value.trim().length > 0, label + " must be nonempty")
  return value.trim()
}

function requiredOption(values, key) {
  return requireText(values.get(key), key)
}

function remaining(deadline) {
  const value = deadline - Date.now()
  assert.ok(value > 0, "placement setup deadline expired")
  return value
}

function isLocalKernelEndpoint(value) {
  if (path.isAbsolute(value)) return true
  try {
    const url = new URL(value)
    return url.protocol === "ws:" && ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname)
      && !url.username && !url.password && !url.search && !url.hash
  } catch {
    return false
  }
}

function isWithin(candidate, parent) {
  const relative = path.relative(parent, candidate)
  return relative === "" || (!relative.startsWith(".." + path.sep)
    && relative !== ".." && !path.isAbsolute(relative))
}

async function main(argv) {
  const options = parseArgs(argv)
  if (options.mode === "--create") {
    const [{ LocalIpcClient }, requests] = await Promise.all([
      import("../../../packages/kernel-client/dist/ipc.js"),
      import("../../../packages/kernel-client/dist/ipc-requests.js"),
    ])
    const manifest = await createPlacementSetup(options, LocalIpcClient, requests)
    process.stdout.write(JSON.stringify({
      status: manifest.setup.status,
      configPath: options.configPath,
      roomCount: manifest.rows.length,
      selectionMode: manifest.selectionMode,
      ownedRoomCount: manifest.setup.ownership.rooms.length,
      sliceCount: manifest.setup.ownership.slices.length,
      officialProviderActionsRun: false,
      acceptanceClaimed: false,
    }, null, 2) + "\n")
    return
  }
  const configPath = await preparePrivateConfigPath(options.configPath)
  const manifest = await readPrivateConfig(configPath)
  const [{ LocalIpcClient }, requests] = await Promise.all([
    import("../../../packages/kernel-client/dist/ipc.js"),
    import("../../../packages/kernel-client/dist/ipc-requests.js"),
  ])
  const client = new LocalIpcClient(manifest.homeKernel.url)
  try {
    let recoveryIncomplete = false
    try {
      await recoverPendingCreates({ client, requests, manifest, configPath, timeoutMs: 30_000 })
    } catch {
      recoveryIncomplete = true
    }
    const result = await cleanupOwnedPlacement({ client, requests, manifest })
    const cleanupComplete = !recoveryIncomplete && manifest.setup.ownership.pending.length === 0
    manifest.setup.status = cleanupComplete ? "cleaned" : "cleanup_incomplete"
    if (cleanupComplete) manifest.setup.cleanedAt = new Date().toISOString()
    manifest.setup.cleanupResult = result
    await writePrivateConfig(configPath, manifest)
    process.stdout.write(JSON.stringify({ status: manifest.setup.status, ...result }, null, 2) + "\n")
    if (!cleanupComplete) throw new Error("uncertain resource creation remains in the ownership ledger")
  } finally {
    await client.close?.().catch(() => {})
  }
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  main(process.argv.slice(2)).catch(() => {
    process.stderr.write(JSON.stringify({
      status: "failed",
      failureCode: "room_placement_setup_failed",
    }) + "\n")
    process.exitCode = 1
  })
}
