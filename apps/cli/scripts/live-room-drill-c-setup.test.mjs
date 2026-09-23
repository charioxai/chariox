import assert from "node:assert/strict"
import test from "node:test"
import { access, mkdir, mkdtemp, readdir, rename, rm } from "node:fs/promises"
import path from "node:path"

import {
  assertCloudRelayBootstrap,
  assertExistingKernelSnapshot,
  assertLoopbackUrl,
  assertModeOptions,
  assertRootlessWorkspaceRoot,
  assertNewSessionIdentity,
  assertNewSliceIdentity,
  assertRoomSliceBinding,
  buildRoomBaseline,
  buildSetupManifest,
  cleanupOwnedResources,
  createExistingKernelWorkspaceFixture,
  parseArgs,
  requiresDirectDockerAccess,
} from "./live-room-drill-c-setup.mjs"
import { removeRoomDirectDockerWorkspaceFixture } from "./lib/room-rootless-workspace-fixture.mjs"

const workspace = "/home/test/.chariox/dev/browser-computer-use/drill-c/workspace"

const setup = {
  session: {
    id: "room-1",
    host_daemon_id: "kernel-1",
    host_machine_id: "machine-1",
    workspace_id: workspace,
    worktree_id: workspace,
    active_provider_run_id: null,
    agents: [],
  },
  slice: {
    id: "slice-1",
    name: "drill-c-local",
    backend: "local_docker",
    owner_kernel_id: "kernel-1",
    owner_machine_id: "machine-1",
    display_mode: "headed",
    display_endpoint: { kind: "selkies", url: "http://127.0.0.1:6080/" },
    worker_kernel_ref: "worker-1",
    workspace_id: workspace,
    worktree_id: workspace,
    workspace_mount: workspace,
    local_docker_ports: { novnc: 6080 },
  },
  binding: {
    session_id: "room-1",
    slice_id: "slice-1",
    owner_kernel_id: "kernel-1",
    worker_kernel_ref: "worker-1",
  },
  environment: {
    session_id: "room-1",
    environment_id: "environment-1",
    runtime_generation: 4,
    lifecycle: "ready",
    focused_tab_id: "tab-1",
    actions: [],
    viewport: { css_width: 1280, css_height: 800, device_scale_factor: 1, desktop_pixel_width: 1280, desktop_pixel_height: 800, revision: 1 },
    tabs: [{ tab_id: "tab-1", url: "about:blank", title: "", document_revision: 1 }],
  },
  resourceInventory: {
    session_id: "room-1",
    environment_id: "environment-1",
    slice_id: "slice-1",
    browser_ids: ["browser-1"],
    profile_ids: ["profile-1"],
  },
  transport: {
    status: "verified",
    relayUrl: "ws://127.0.0.1:47000",
    targetDaemonId: "kernel-1",
    targetMachineId: "machine-1",
    sessionVisible: true,
    verifiedAt: "2026-09-23T10:00:00.000Z",
    cloudApiUrl: "http://127.0.0.1:4321",
  },
}

const baseline = buildRoomBaseline({
  sessionId: setup.session.id,
  sliceId: setup.slice.id,
  environment: setup.environment,
  resourceInventory: setup.resourceInventory,
  actionHistory: { actions: [], next_before_sequence: null },
  capturedAt: "2026-09-23T10:00:00.000Z",
})

const priorKernelState = { sessionCount: 0, sliceCount: 0 }

function manifestInput(overrides = {}) {
  return {
    createdAt: "2026-09-23T10:00:00.000Z",
    sourceCommit: "e30e3086688ca58571afdae3b9e27252bbe830d7",
    rootDir: "/home/test/.chariox/dev/browser-computer-use/drill-c",
    manifestPath: "/home/test/.chariox/dev/browser-computer-use/drill-c/setup-manifest.json",
    cloudUrl: "http://127.0.0.1:4321",
    kernelUrl: "ws://127.0.0.1:52001/kernel",
    relayUrl: "ws://127.0.0.1:47000",
    daemonId: "kernel-1",
    daemonAlias: "kernel-1",
    machineId: "machine-1",
    machineAlias: "machine-1",
    ...setup,
    workspace,
    worktree: workspace,
    baseline,
    priorKernelState,
    ...overrides,
  }
}

async function rootlessWorkspaceTree(t) {
  const root = await mkdtemp(path.join("/var/tmp", "drill-c-rootless-workspace-test-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const homeDir = path.join(root, "private-home")
  const repositoryRoot = path.join(root, "source-repository")
  const workspaceRoot = path.join(root, "engine-visible")
  await Promise.all([mkdir(homeDir), mkdir(repositoryRoot), mkdir(workspaceRoot)])
  return { root, homeDir, repositoryRoot, workspaceRoot }
}

test("Drill C setup modes require loopback endpoints and explicit existing-kernel identity", () => {
  assert.throws(() => assertLoopbackUrl("https://cloud.example.test", "local Cloud URL", ["http:", "https:"]), /loopback host/)
  assert.throws(() => assertLoopbackUrl("wss://relay.example.test", "local relay URL", ["ws:"]), /must use ws:/)
  assert.throws(() => assertLoopbackUrl("ws://user@127.0.0.1:47000", "relay URL", ["ws:"]), /username/)

  const isolated = parseArgs([], { HOME: "/home/test" })
  assert.equal(isolated.mode, "isolated_local")
  assert.doesNotThrow(() => assertModeOptions(isolated))
  assert.equal(requiresDirectDockerAccess(isolated.mode), true)

  const existingEnv = {
    HOME: "/home/test",
    CHARIOX_ROOM_DRILL_FIXTURE_WORKSPACE_ROOT: "/var/tmp",
    get CHARIOX_LOCAL_RELAY_URL() { throw new Error("existing mode read the local relay URL") },
    get CHARIOX_LOCAL_RELAY_TOKEN() { throw new Error("existing mode read a relay token") },
  }
  const existing = parseArgs([
    "--existing-kernel", "ws://127.0.0.1:52001/kernel",
    "--rootless-workspace-root", "/var/tmp",
    "--expected-daemon-id", "kernel-1",
    "--expected-machine-id", "machine-1",
  ], existingEnv)
  assert.equal(existing.mode, "existing_kernel")
  assert.equal(requiresDirectDockerAccess(existing.mode), false)
  assert.match(existing.rootDir, /^\/home\/test\/\.chariox\/dev\/browser-computer-use\/drill-c-existing-kernel-/)
  assert.equal(existing.manifestPath, `${existing.rootDir}/setup-manifest.json`)
  assert.equal(existing.relayUrl, null)
  assert.equal(existing.relayToken, null)
  assert.doesNotThrow(() => assertModeOptions(existing))
  const existingFromExplicitRootEnvironment = parseArgs([
    "--existing-kernel", "ws://127.0.0.1:52001/kernel",
    "--expected-daemon-id", "kernel-1",
  ], existingEnv)
  assert.equal(existingFromExplicitRootEnvironment.rootlessWorkspaceRoot, "/var/tmp")
  assert.doesNotThrow(() => assertModeOptions(existingFromExplicitRootEnvironment))
  assert.throws(() => assertModeOptions(parseArgs([
    "--existing-kernel", "ws://127.0.0.1:52001/kernel",
  ], { HOME: "/home/test" })), /requires --expected-daemon-id/)
  assert.throws(() => assertModeOptions(parseArgs([
    "--existing-kernel", "ws://kernel.example.test:52001/kernel",
    "--expected-daemon-id", "kernel-1",
    "--rootless-workspace-root", "/var/tmp",
  ], { HOME: "/home/test" })), /loopback host/)
  assert.throws(() => assertModeOptions(parseArgs([
    "--existing-kernel", "ws://127.0.0.1:52001/kernel",
    "--expected-daemon-id", "kernel-1",
    "--rootless-workspace-root", "/var/tmp",
    "--relay-url", "ws://127.0.0.1:47000",
  ], { HOME: "/home/test" })), /cannot be combined/)
  assert.throws(() => assertModeOptions(parseArgs([
    "--existing-kernel", "ws://127.0.0.1:52001/kernel",
    "--expected-daemon-id", "kernel-1",
  ], { HOME: "/home/test" })), /requires --rootless-workspace-root/)
  assert.throws(() => assertModeOptions(parseArgs([
    "--existing-kernel", "ws://127.0.0.1:52001/kernel",
    "--expected-daemon-id", "kernel-1",
    "--rootless-workspace-root", "/var/tmp/private-home/.chariox/dev/workspaces",
  ], { HOME: "/var/tmp/private-home" })), /outside the invoking user's home/)
  assert.throws(() => assertRootlessWorkspaceRoot({
    workspaceRoot: "/home/test/.chariox/dev/workspaces",
    homeDir: "/home/test",
    repositoryRoot: "/home/test/source",
  }), /under explicit engine-visible \/var\/tmp/)
  assert.throws(() => assertRootlessWorkspaceRoot({
    workspaceRoot: "/var/tmp",
    homeDir: "/home/test",
    repositoryRoot: "/var/tmp",
  }), /outside the source repository/)
})

test("existing-kernel workspace selection creates an empty guarded child under the explicit safe root", async (t) => {
  const tree = await rootlessWorkspaceTree(t)
  const fixture = await createExistingKernelWorkspaceFixture(tree)

  assert.equal(fixture.kind, "direct")
  assert.equal(fixture.workspaceRoot, tree.workspaceRoot)
  assert.equal(path.dirname(fixture.workspace), tree.workspaceRoot)
  assert.deepEqual(await readdir(fixture.workspace), [])
  assert.equal(assertRootlessWorkspaceRoot({
    workspaceRoot: tree.workspaceRoot,
    homeDir: tree.homeDir,
    repositoryRoot: tree.repositoryRoot,
  }), tree.workspaceRoot)

  await removeRoomDirectDockerWorkspaceFixture(fixture)
  await assert.rejects(access(fixture.workspace), error => error?.code === "ENOENT")
})

test("existing-kernel cleanup verifies Room and slice shutdown before removing the owned workspace", async (t) => {
  const tree = await rootlessWorkspaceTree(t)
  const fixture = await createExistingKernelWorkspaceFixture(tree)
  const events = []
  const requests = {
    stopRoomEnvironmentRequest: (sessionId) => ({ StopRoomEnvironment: { session_id: sessionId } }),
    endSessionRequest: (sessionId) => ({ EndSession: { session_id: sessionId } }),
    stopSliceRequest: (sliceId) => ({ StopSlice: { slice_ref: sliceId } }),
    deleteSliceRequest: (sliceId) => ({ DeleteSlice: { slice_ref: sliceId } }),
  }
  const responses = {
    StopRoomEnvironment: { RoomEnvironmentUpdated: { environment: { session_id: "room-owned" } } },
    EndSession: { SessionEnded: { session: { id: "room-owned" } } },
    StopSlice: { SliceStopped: { slice: { id: "slice-owned" } } },
    DeleteSlice: { SliceDeleted: { slice: { id: "slice-owned" } } },
  }
  const state = {
    client: {
      async send(request) {
        const command = Object.keys(request)[0]
        events.push(command)
        return responses[command]
      },
      async close() { events.push("close") },
    },
    sessionId: "room-owned",
    sliceId: "slice-owned",
    roomEnvironmentStartAttempted: true,
    sessionCreateAttempted: true,
    sliceCreateAttempted: true,
    rootlessWorkspaceFixture: fixture,
    workspaceOwned: false,
    workspace: fixture.workspace,
  }

  const errors = await cleanupOwnedResources({
    state,
    requests,
    children: [],
    removeWorkspaceFixture: async (lease) => {
      events.push("remove-workspace")
      await removeRoomDirectDockerWorkspaceFixture(lease)
    },
  })

  assert.deepEqual(errors, [])
  assert.deepEqual(events, [
    "StopRoomEnvironment", "EndSession", "StopSlice", "DeleteSlice", "close", "remove-workspace",
  ])
  await assert.rejects(access(fixture.workspace), error => error?.code === "ENOENT")
})

test("existing-kernel cleanup retains the workspace when slice deletion is unverified", async (t) => {
  const tree = await rootlessWorkspaceTree(t)
  const fixture = await createExistingKernelWorkspaceFixture(tree)
  const requests = {
    stopRoomEnvironmentRequest: (sessionId) => ({ StopRoomEnvironment: { session_id: sessionId } }),
    endSessionRequest: (sessionId) => ({ EndSession: { session_id: sessionId } }),
    stopSliceRequest: (sliceId) => ({ StopSlice: { slice_ref: sliceId } }),
    deleteSliceRequest: (sliceId) => ({ DeleteSlice: { slice_ref: sliceId } }),
  }
  const state = {
    client: {
      async send(request) {
        const command = Object.keys(request)[0]
        if (command === "DeleteSlice") throw new Error("delete acknowledgement missing")
        if (command === "StopRoomEnvironment") {
          return { RoomEnvironmentUpdated: { environment: { session_id: "room-owned" } } }
        }
        if (command === "EndSession") return { SessionEnded: { session: { id: "room-owned" } } }
        if (command === "StopSlice") return { SliceStopped: { slice: { id: "slice-owned" } } }
        throw new Error(`unexpected cleanup request: ${command}`)
      },
      async close() {},
    },
    sessionId: "room-owned",
    sliceId: "slice-owned",
    roomEnvironmentStartAttempted: true,
    sessionCreateAttempted: true,
    sliceCreateAttempted: true,
    rootlessWorkspaceFixture: fixture,
    workspaceOwned: false,
    workspace: fixture.workspace,
  }

  const errors = await cleanupOwnedResources({ state, requests, children: [] })

  assert.ok(errors.some(message => message.includes("slice deletion cleanup failed")))
  assert.ok(errors.some(message => message.includes("refusing rootless workspace removal")))
  await access(fixture.workspace)
  await removeRoomDirectDockerWorkspaceFixture(fixture)
})

test("existing-kernel cleanup refuses a replaced workspace after successful resource shutdown", async (t) => {
  const tree = await rootlessWorkspaceTree(t)
  const fixture = await createExistingKernelWorkspaceFixture(tree)
  const movedWorkspace = path.join(tree.root, "moved-owned-workspace")
  await rename(fixture.workspace, movedWorkspace)
  await mkdir(fixture.workspace)
  const requests = {
    endSessionRequest: (sessionId) => ({ EndSession: { session_id: sessionId } }),
    stopSliceRequest: (sliceId) => ({ StopSlice: { slice_ref: sliceId } }),
    deleteSliceRequest: (sliceId) => ({ DeleteSlice: { slice_ref: sliceId } }),
  }
  const state = {
    client: {
      async send(request) {
        const command = Object.keys(request)[0]
        if (command === "EndSession") return { SessionEnded: { session: { id: "room-owned" } } }
        if (command === "StopSlice") return { SliceStopped: { slice: { id: "slice-owned" } } }
        if (command === "DeleteSlice") return { SliceDeleted: { slice: { id: "slice-owned" } } }
        throw new Error(`unexpected cleanup request: ${command}`)
      },
      async close() {},
    },
    sessionId: "room-owned",
    sliceId: "slice-owned",
    roomEnvironmentStartAttempted: false,
    sessionCreateAttempted: true,
    sliceCreateAttempted: true,
    rootlessWorkspaceFixture: fixture,
    workspaceOwned: false,
    workspace: fixture.workspace,
  }

  const errors = await cleanupOwnedResources({ state, requests, children: [] })

  assert.ok(errors.some(message => message.includes("refusing to remove replaced room fixture workspace")))
  await access(fixture.workspace)
  await access(movedWorkspace)
})

test("isolated mode only claims Cloud visibility after exact relay bootstrap and Room-list proof", () => {
  const bootstrap = {
    relayUrl: "ws://127.0.0.1:47000",
    relayToken: "local-only-token",
    target: { daemonId: "kernel-1", machineId: "machine-1", daemonAlias: "kernel-1" },
  }
  const result = assertCloudRelayBootstrap({
    bootstrap,
    relayUrl: bootstrap.relayUrl,
    relayToken: bootstrap.relayToken,
    daemonId: "kernel-1",
    machineId: "machine-1",
    sessionId: "room-1",
    sessions: [{ id: "room-1" }],
  })
  assert.equal(result.status, "verified")
  assert.equal(result.sessionVisible, true)
  for (const mismatch of [
    { bootstrap: { ...bootstrap, target: { ...bootstrap.target, daemonId: "another-kernel" } }, expected: /different kernel/ },
    { bootstrap: { ...bootstrap, target: { ...bootstrap.target, machineId: "another-machine" } }, expected: /different machine/ },
    { bootstrap: { ...bootstrap, relayToken: "wrong-local-token" }, expected: /token does not match/ },
  ]) {
    assert.throws(() => assertCloudRelayBootstrap({
      bootstrap: mismatch.bootstrap,
      relayUrl: bootstrap.relayUrl,
      relayToken: bootstrap.relayToken,
      daemonId: "kernel-1",
      machineId: "machine-1",
      sessionId: "room-1",
      sessions: [{ id: "room-1" }],
    }), mismatch.expected)
  }
  assert.throws(() => assertCloudRelayBootstrap({
    bootstrap,
    relayUrl: "ws://127.0.0.1:47001",
    relayToken: bootstrap.relayToken,
    daemonId: "kernel-1",
    machineId: "machine-1",
    sessionId: "room-1",
    sessions: [{ id: "room-1" }],
  }), /different relay/)
  assert.throws(() => assertCloudRelayBootstrap({
    bootstrap,
    relayUrl: bootstrap.relayUrl,
    relayToken: bootstrap.relayToken,
    daemonId: "kernel-1",
    machineId: "machine-1",
    sessionId: "room-1",
    sessions: [{ id: "other-room" }],
  }), /cannot see the setup Room session/)
})

test("existing-kernel preflight proves daemon and machine identity before any mutation", () => {
  const snapshot = assertExistingKernelSnapshot({
    sessions: [setup.session],
    slices: [setup.slice],
    expectedDaemonId: "kernel-1",
    expectedMachineId: "machine-1",
  })
  assert.deepEqual(snapshot.sessionIds, ["room-1"])
  assert.deepEqual(snapshot.sliceIds, ["slice-1"])
  assert.equal(snapshot.machineId, "machine-1")

  assert.throws(() => assertExistingKernelSnapshot({
    sessions: [{ ...setup.session, host_daemon_id: "wrong-kernel" }],
    slices: [setup.slice],
    expectedDaemonId: "kernel-1",
  }), /different daemon/)
  assert.throws(() => assertExistingKernelSnapshot({
    sessions: [setup.session],
    slices: [{ ...setup.slice, owner_kernel_id: "wrong-kernel" }],
    expectedDaemonId: "kernel-1",
  }), /different daemon/)
  assert.throws(() => assertExistingKernelSnapshot({
    sessions: [setup.session],
    slices: [setup.slice],
    expectedDaemonId: "kernel-1",
    expectedMachineId: "wrong-machine",
  }), /different machine/)
  assert.throws(() => assertExistingKernelSnapshot({
    sessions: [],
    slices: [],
    expectedDaemonId: "kernel-1",
  }), /cannot verify existing kernel identity before mutation/)
})

test("new Room and slice IDs must be unique and match the selected kernel", () => {
  assert.equal(assertNewSliceIdentity({
    slice: setup.slice,
    expectedDaemonId: "kernel-1",
    expectedMachineId: "machine-1",
    workspace,
    priorSliceIds: ["old-slice"],
  }), setup.slice)
  assert.throws(() => assertNewSliceIdentity({
    slice: setup.slice,
    expectedDaemonId: "wrong-kernel",
    expectedMachineId: "machine-1",
    workspace,
    priorSliceIds: [],
  }), /different daemon/)
  assert.throws(() => assertNewSliceIdentity({
    slice: setup.slice,
    expectedDaemonId: "kernel-1",
    expectedMachineId: "machine-1",
    workspace,
    priorSliceIds: ["slice-1"],
  }), /collides with an existing slice/)

  assert.equal(assertNewSessionIdentity({
    session: setup.session,
    expectedDaemonId: "kernel-1",
    expectedMachineId: "machine-1",
    workspace,
    worktree: workspace,
    priorSessionIds: [],
  }), setup.session)
  assert.throws(() => assertNewSessionIdentity({
    session: { ...setup.session, host_daemon_id: "wrong-kernel" },
    expectedDaemonId: "kernel-1",
    expectedMachineId: "machine-1",
    workspace,
    worktree: workspace,
    priorSessionIds: [],
  }), /different daemon/)
  assert.throws(() => assertNewSessionIdentity({
    session: setup.session,
    expectedDaemonId: "kernel-1",
    expectedMachineId: "machine-1",
    workspace,
    worktree: workspace,
    priorSessionIds: ["room-1"],
  }), /collides with an existing session/)
  assert.throws(() => assertNewSessionIdentity({
    session: { ...setup.session, agents: [{ id: "unexpected-agent" }] },
    expectedDaemonId: "kernel-1",
    expectedMachineId: "machine-1",
    workspace,
    worktree: workspace,
    priorSessionIds: [],
  }), /unexpectedly contains existing agents/)
  assert.throws(() => assertRoomSliceBinding(setup.binding, {
    sessionId: "another-room",
    slice: setup.slice,
    daemonId: "kernel-1",
  }), /different session/)
  assert.throws(() => assertRoomSliceBinding(setup.binding, {
    sessionId: "room-1",
    slice: { ...setup.slice, id: "another-slice" },
    daemonId: "kernel-1",
  }), /different slice/)
})

test("baseline comes from the public Room state and requires a clean new action history", () => {
  const captured = buildRoomBaseline({
    sessionId: "room-1",
    sliceId: "slice-1",
    environment: setup.environment,
    resourceInventory: setup.resourceInventory,
    actionHistory: { actions: [], next_before_sequence: null },
    capturedAt: "2026-09-23T10:00:00.000Z",
  })
  assert.equal(captured.source, "kernel public Room and resource inventory requests")
  assert.equal(captured.environmentId, "environment-1")
  assert.deepEqual(captured.actionHistory, [])
  assert.throws(() => buildRoomBaseline({
    sessionId: "room-1",
    sliceId: "slice-1",
    environment: setup.environment,
    resourceInventory: setup.resourceInventory,
    actionHistory: { actions: [{ action_id: "made-up" }], next_before_sequence: null },
    capturedAt: "2026-09-23T10:00:00.000Z",
  }), /action history is not empty/)
})

test("isolated manifest carries exact observer inputs without claiming Browser or Web evidence", () => {
  const manifest = buildSetupManifest(manifestInput())
  assert.equal(manifest.status, "ready_for_observers")
  assert.equal(manifest.setupMode, "isolated_local")
  assert.equal(manifest.kernelUrl, "ws://127.0.0.1:52001/kernel")
  assert.equal(manifest.sessionId, "room-1")
  assert.equal(manifest.sliceId, "slice-1")
  assert.equal(manifest.worktree, workspace)
  assert.equal(manifest.display.browserIds[0], "browser-1")
  assert.equal(manifest.display.profileIds[0], "profile-1")
  assert.equal(manifest.display.environmentId, "environment-1")
  assert.equal(manifest.webObservationPath, manifest.webObserver.observationPath)
  assert.equal(manifest.webObserver.evidenceStatus, "not_observed")
  assert.equal(manifest.localCloudTransport.status, "verified")
  assert.ok(manifest.tuiObserver.args.includes("--observe-room-session"))
  assert.ok(manifest.tuiObserver.args.includes("--slice-id"))
  assert.equal("actions" in manifest, false)
  assert.equal("relayToken" in manifest, false)
  assert.equal("relayToken" in manifest.localCloudTransport, false)
  assert.equal("endpointUrl" in manifest.display, false)
  assert.equal("provider" in manifest, false)
})

test("existing-kernel manifest defers Cloud and Web transport to the Mac frontend", () => {
  const directWorkspace = "/var/tmp/drill-c-room-workspace"
  const manifest = buildSetupManifest(manifestInput({
    mode: "existing_kernel",
    relayUrl: null,
    transport: { status: "not_observed", sessionVisible: null },
    priorKernelState: { sessionCount: 7, sliceCount: 2 },
    daemonAlias: null,
    machineAlias: null,
    session: { ...setup.session, workspace_id: directWorkspace, worktree_id: directWorkspace },
    slice: { ...setup.slice, workspace_id: directWorkspace, worktree_id: directWorkspace, workspace_mount: directWorkspace },
    workspace: directWorkspace,
    worktree: directWorkspace,
  }))

  assert.equal(manifest.setupMode, "existing_kernel")
  assert.match(manifest.rootDir, /^\/home\/test\/\.chariox\/dev\/browser-computer-use\/drill-c$/)
  assert.equal(manifest.workspace, directWorkspace)
  assert.equal(manifest.worktree, directWorkspace)
  assert.equal(manifest.kernel.daemonId, "kernel-1")
  assert.equal(manifest.kernel.machineId, "machine-1")
  assert.equal(manifest.relayUrl, null)
  assert.equal(manifest.localCloudTransport.status, "not_observed")
  assert.equal(manifest.localCloudTransport.sessionVisible, null)
  assert.equal(manifest.localCloudTransport.verificationRequiredFrom, "Mac local Cloud frontend")
  assert.equal(manifest.webObserver.openUrl, "http://127.0.0.1:4321/waiting-room")
  assert.equal(manifest.webObserver.transportStatus, "not_observed")
  assert.equal(manifest.webObserver.evidenceStatus, "not_observed")
  assert.equal(manifest.webObserver.relayUrl, null)
  assert.deepEqual(manifest.baseline.actionHistory, [])
  assert.equal(manifest.priorKernelState.sessionCount, 7)
  assert.equal("relayToken" in manifest, false)
})

test("manifest fails closed on mismatched Room, slice, baseline, or transport identity", () => {
  assert.throws(() => buildSetupManifest(manifestInput({
    binding: { ...setup.binding, slice_id: "other-slice" },
  })), /different slice/)
  assert.throws(() => buildSetupManifest(manifestInput({
    transport: { ...setup.transport, sessionVisible: false },
  })), /did not observe the Room session/)
  assert.throws(() => buildSetupManifest(manifestInput({
    baseline: { ...baseline, sessionId: "another-room" },
  })), /baseline belongs to a different Room/)
  assert.throws(() => buildSetupManifest(manifestInput({
    mode: "existing_kernel",
    relayUrl: null,
    transport: { status: "not_observed", sessionVisible: true },
  })), /must not claim the Room is visible to Cloud/)
})

test("setup defaults to isolated dev state and never invokes build tools", () => {
  const options = parseArgs([], { HOME: "/home/test" })
  assert.match(options.rootDir, /^\/home\/test\/\.chariox\/dev\/browser-computer-use\/drill-c-same-host-/)
  assert.equal(options.localCloudUrl, "http://127.0.0.1:4321")
  assert.equal(options.relayToken, "local-browser-terminal-relay-token")
  assert.match(options.kernelBinary, /target\/debug\/chariox-kernel$/)
  assert.match(options.relayBinary, /target\/debug\/chariox-relay$/)
})
