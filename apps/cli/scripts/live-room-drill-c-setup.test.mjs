import assert from "node:assert/strict"
import test from "node:test"

import {
  assertCloudRelayBootstrap,
  assertLoopbackUrl,
  buildSetupManifest,
  parseArgs,
} from "./live-room-drill-c-setup.mjs"

const setup = {
  session: { id: "room-1" },
  slice: {
    id: "slice-1",
    name: "drill-c-local",
    backend: "local_docker",
    display_mode: "headed",
    display_endpoint: { kind: "selkies", url: "http://127.0.0.1:6080/" },
    worker_kernel_ref: "worker-1",
    workspace_mount: "/home/test/.chariox/dev/browser-computer-use/drill-c/workspace",
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

test("Drill C setup rejects non-loopback Cloud and relay endpoints", () => {
  assert.throws(() => assertLoopbackUrl("https://cloud.example.test", "local Cloud URL", ["http:", "https:"]), /loopback host/)
  assert.throws(() => assertLoopbackUrl("wss://relay.example.test", "local relay URL", ["ws:"]), /must use ws:/)
  assert.throws(() => assertLoopbackUrl("ws://user@127.0.0.1:47000", "relay URL", ["ws:"]), /username/)
})

test("Drill C relay bootstrap must expose this exact kernel, machine, relay, token, and Room", () => {
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
  assert.throws(() => assertCloudRelayBootstrap({
    bootstrap: { ...bootstrap, target: { ...bootstrap.target, daemonId: "another-kernel" } },
    relayUrl: bootstrap.relayUrl,
    relayToken: bootstrap.relayToken,
    daemonId: "kernel-1",
    machineId: "machine-1",
    sessionId: "room-1",
    sessions: [{ id: "room-1" }],
  }), /different kernel/)
  assert.throws(() => assertCloudRelayBootstrap({
    bootstrap: { ...bootstrap, target: { ...bootstrap.target, machineId: "another-machine" } },
    relayUrl: bootstrap.relayUrl,
    relayToken: bootstrap.relayToken,
    daemonId: "kernel-1",
    machineId: "machine-1",
    sessionId: "room-1",
    sessions: [{ id: "room-1" }],
  }), /different machine/)
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
    bootstrap: { ...bootstrap, relayToken: "wrong-local-token" },
    relayUrl: bootstrap.relayUrl,
    relayToken: bootstrap.relayToken,
    daemonId: "kernel-1",
    machineId: "machine-1",
    sessionId: "room-1",
    sessions: [{ id: "room-1" }],
  }), /token does not match/)
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

test("Drill C manifest carries real observer inputs and no action or Web evidence claims", () => {
  const manifest = buildSetupManifest({
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
    workspace: setup.slice.workspace_mount,
    worktree: setup.slice.workspace_mount,
  })

  assert.equal(manifest.status, "ready_for_observers")
  assert.equal(manifest.kernelUrl, "ws://127.0.0.1:52001/kernel")
  assert.equal(manifest.sessionId, "room-1")
  assert.equal(manifest.sliceId, "slice-1")
  assert.equal(manifest.display.browserIds[0], "browser-1")
  assert.equal(manifest.display.profileIds[0], "profile-1")
  assert.equal(manifest.display.environmentId, "environment-1")
  assert.equal(manifest.webObservationPath, manifest.webObserver.observationPath)
  assert.equal(manifest.webObserver.evidenceStatus, "not_observed")
  assert.equal(manifest.webObserver.observationPath, "/home/test/.chariox/dev/browser-computer-use/drill-c/evidence/drill-c-live-observation.json")
  assert.ok(manifest.tuiObserver.args.includes("--observe-room-session"))
  assert.ok(manifest.tuiObserver.args.includes("--slice-id"))
  assert.equal("actions" in manifest, false)
  assert.equal("relayToken" in manifest, false)
  assert.equal("relayToken" in manifest.localCloudTransport, false)
  assert.equal("endpointUrl" in manifest.display, false)
  assert.equal("provider" in manifest, false)
})

test("Drill C manifest fails closed on invented environment or slice identity", () => {
  assert.throws(() => buildSetupManifest({
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
    binding: { ...setup.binding, slice_id: "other-slice" },
    workspace: setup.slice.workspace_mount,
    worktree: setup.slice.workspace_mount,
  }), /slice binding mismatch/)
  assert.throws(() => buildSetupManifest({
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
    resourceInventory: { ...setup.resourceInventory, browser_ids: [] },
    workspace: setup.slice.workspace_mount,
    worktree: setup.slice.workspace_mount,
  }), /requires live browser identities/)
})

test("Drill C setup defaults to isolated dev state and never invokes build tools", () => {
  const options = parseArgs([], { HOME: "/home/test" })
  assert.match(options.rootDir, /^\/home\/test\/\.chariox\/dev\/browser-computer-use\/drill-c-same-host-/)
  assert.equal(options.localCloudUrl, "http://127.0.0.1:4321")
  assert.equal(options.relayToken, "local-browser-terminal-relay-token")
  assert.match(options.kernelBinary, /target\/debug\/chariox-kernel$/)
  assert.match(options.relayBinary, /target\/debug\/chariox-relay$/)
})
