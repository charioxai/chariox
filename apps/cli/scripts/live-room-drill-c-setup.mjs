#!/usr/bin/env node

import assert from "node:assert/strict"
import { spawn, execFile } from "node:child_process"
import { constants as fsConstants, createWriteStream } from "node:fs"
import { access, chmod, mkdir, open, rm, symlink, unlink, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { promisify } from "node:util"
import { fileURLToPath, pathToFileURL } from "node:url"
import {
  createRoomDirectDockerWorkspaceFixture,
  removeRoomDirectDockerWorkspaceFixture,
  roomDirectDockerWorkspaceRootEnvironment,
} from "./lib/room-rootless-workspace-fixture.mjs"
import { roomDrillRelayToken } from "./lib/room-drill-relay-token.mjs"

const execFileAsync = promisify(execFile)
const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const kernelClientRoot = path.join(repoRoot, "packages", "kernel-client")
const devRoot = path.join(os.homedir(), ".chariox", "dev", "browser-computer-use")
const defaultLocalCloudUrl = "http://127.0.0.1:4321"
const defaultLocalRelayToken = "local-browser-terminal-relay-token"
const protocolClientRoomReadyTimeoutMs = 180_000
// A two-core local Docker VM can need more than the client's 10-minute
// acknowledgement deadline to compile a changed release image.
const coldSliceProvisionTimeoutMs = 20 * 60_000

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

export function mintDrillCKernelRelayToken({ issuer, secret, machineId, daemonId, nowMs = Date.now() }) {
  return roomDrillRelayToken({
    issuer,
    secret,
    machineId,
    subject: daemonId,
    subjectKind: "kernel",
    actions: ["daemon_register", "daemon_heartbeat", "packet_route", "peer_request", "peer_event"],
    // This isolated loopback relay has no hosted kernel-token renewal. Keep
    // the manual TUI/Web observer session live, then tear it down with setup.
    minimumLifetimeMs: 2 * 60 * 60_000,
    nowMs,
  })
}

export function parseArgs(argv, env = process.env) {
  const homeDir = env.HOME?.trim() ? path.resolve(env.HOME) : os.homedir()
  const taskDevRoot = path.join(homeDir, ".chariox", "dev", "browser-computer-use")
  const targetDir = env.CARGO_TARGET_DIR?.trim()
    ? path.resolve(repoRoot, env.CARGO_TARGET_DIR)
    : path.join(repoRoot, "target")
  const stamp = new Date().toISOString().replace(/[:.]/g, "-")
  const options = {
    existingKernelUrl: null,
    rootlessWorkspaceRoot: env[roomDirectDockerWorkspaceRootEnvironment]?.trim()
      ? path.resolve(env[roomDirectDockerWorkspaceRootEnvironment].trim())
      : null,
    homeDir,
    expectedDaemonId: env.CHARIOX_EXPECTED_DAEMON_ID ?? null,
    expectedMachineId: env.CHARIOX_EXPECTED_MACHINE_ID ?? null,
    rootDir: path.join(taskDevRoot, `drill-c-same-host-${stamp}`),
    rootDirProvided: false,
    manifestPath: null,
    manifestPathProvided: false,
    localCloudUrl: env.CHARIOX_LOCAL_CLOUD_URL ?? defaultLocalCloudUrl,
    relayUrl: null,
    relayToken: null,
    relayUrlArgumentProvided: false,
    relayTokenArgumentProvided: false,
    scopedRelayIssuer: env.CHARIOX_RELAY_SCOPED_ISSUER?.trim() || null,
    scopedRelaySecret: env.CHARIOX_RELAY_SCOPED_HMAC_SECRET?.trim() || null,
    sliceImageBuildPolicy: "never",
    allowProviderSandboxCompatibility: false,
    activeKernelRegistryDir: env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR
      ?? (env.XDG_CONFIG_HOME?.trim()
        ? path.join(path.resolve(env.XDG_CONFIG_HOME), "chariox", "kernels", "active")
        : path.join(homeDir, ".chariox", "kernels", "active")),
    kernelBinary: env.CHARIOX_KERNEL_BINARY ?? path.join(targetDir, "debug", "chariox-kernel"),
    relayBinary: env.CHARIOX_RELAY_BINARY ?? path.join(targetDir, "debug", "chariox-relay"),
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    const next = () => {
      const value = argv[index + 1]
      if (!value) throw new Error(`missing value for ${arg}`)
      index += 1
      return value
    }
    if (arg === "--root-dir") {
      options.rootDir = path.resolve(next())
      options.rootDirProvided = true
    }
    else if (arg === "--manifest") {
      options.manifestPath = path.resolve(next())
      options.manifestPathProvided = true
    }
    else if (arg === "--existing-kernel") options.existingKernelUrl = next()
    else if (arg === "--rootless-workspace-root") options.rootlessWorkspaceRoot = path.resolve(next())
    else if (arg === "--expected-daemon-id") options.expectedDaemonId = next()
    else if (arg === "--expected-machine-id") options.expectedMachineId = next()
    else if (arg === "--allow-slice-image-build") options.sliceImageBuildPolicy = "auto"
    else if (arg === "--allow-provider-sandbox-compatibility") options.allowProviderSandboxCompatibility = true
    else if (arg === "--local-cloud-url") options.localCloudUrl = next()
    else if (arg === "--relay-url") {
      options.relayUrl = next()
      options.relayUrlArgumentProvided = true
    }
    else if (arg === "--relay-token") {
      options.relayToken = next()
      options.relayTokenArgumentProvided = true
    }
    else if (arg === "--active-kernel-registry-dir") options.activeKernelRegistryDir = path.resolve(next())
    else if (arg === "--kernel-binary") options.kernelBinary = path.resolve(next())
    else if (arg === "--relay-binary") options.relayBinary = path.resolve(next())
    else if (arg === "--help" || arg === "-h") options.help = true
    else throw new Error(`unknown option: ${arg}`)
  }
  options.rootDir = path.resolve(options.rootDir)
  options.manifestPath ??= path.join(options.rootDir, "setup-manifest.json")
  options.manifestPath = path.resolve(options.manifestPath)
  options.activeKernelRegistryDir = path.resolve(options.activeKernelRegistryDir)
  options.kernelBinary = path.resolve(options.kernelBinary)
  options.relayBinary = path.resolve(options.relayBinary)
  options.mode = options.existingKernelUrl ? "existing_kernel" : "isolated_local"
  if (options.mode === "isolated_local") {
    assert.equal(Boolean(options.scopedRelayIssuer), Boolean(options.scopedRelaySecret),
      "isolated scoped relay requires both issuer and HMAC secret")
    if (options.scopedRelayIssuer) {
      assert.equal(options.relayTokenArgumentProvided, false,
        "isolated scoped relay mints its own kernel token; do not pass --relay-token")
    }
    if (!options.relayUrlArgumentProvided) options.relayUrl = env.CHARIOX_LOCAL_RELAY_URL ?? null
    if (!options.relayTokenArgumentProvided) {
      options.relayToken = env.CHARIOX_LOCAL_RELAY_TOKEN ?? defaultLocalRelayToken
    }
  } else {
    if (!options.relayUrlArgumentProvided) options.relayUrl = null
    options.relayToken = null
  }
  if (options.mode === "existing_kernel" && !options.rootDirProvided) {
    options.rootDir = path.join(taskDevRoot, `drill-c-existing-kernel-${stamp}`)
    if (!options.manifestPathProvided) options.manifestPath = path.join(options.rootDir, "setup-manifest.json")
  }
  options.manifestPath = path.resolve(options.manifestPath)
  return options
}

export function isolatedKernelUserConfig(options) {
  assert.equal(options.mode, "isolated_local",
    "slice image build policy only applies to an isolated-local kernel")
  assert.ok(["never", "auto"].includes(options.sliceImageBuildPolicy),
    "isolated slice image build policy must be never or auto")
  return `[slices.linux]\nbuild_image = "${options.sliceImageBuildPolicy}"\n`
}

export function assertModeOptions(options, env = process.env) {
  if (options.mode === "existing_kernel") {
    assert.equal(options.sliceImageBuildPolicy, "never",
      "--allow-slice-image-build only applies to isolated-local mode")
    assert.equal(options.allowProviderSandboxCompatibility, false,
      "--allow-provider-sandbox-compatibility only applies to isolated-local mode")
    const endpoint = assertLoopbackUrl(options.existingKernelUrl, "--existing-kernel", ["ws:"])
    assert.ok(["", "/", "/kernel"].includes(endpoint.pathname),
      "--existing-kernel must address the local kernel WebSocket endpoint")
    assert.ok(options.expectedDaemonId?.trim(), "--existing-kernel requires --expected-daemon-id")
    assertRootlessWorkspaceRoot({
      workspaceRoot: options.rootlessWorkspaceRoot,
      homeDir: options.homeDir,
      repositoryRoot: repoRoot,
    })
    if (options.expectedMachineId != null) {
      assert.ok(options.expectedMachineId.trim(), "--expected-machine-id must not be empty")
    }
    assert.equal(options.relayTokenArgumentProvided, false, "--existing-kernel does not accept a relay token")
    assert.equal(options.relayToken, null, "existing-kernel mode must not retain a relay token argument")
    if (options.relayUrl != null) {
      const relayEndpoint = assertLoopbackUrl(options.relayUrl, "--relay-url", ["ws:"])
      assert.ok(["", "/"].includes(relayEndpoint.pathname), "--relay-url must address the same-host relay WebSocket endpoint")
      assert.notEqual(relayEndpoint.origin, endpoint.origin,
        "--relay-url must use an endpoint distinct from --existing-kernel")
      assert.ok(typeof env.CHARIOX_DRILL_C_RELAY_TOKEN === "string"
        && env.CHARIOX_DRILL_C_RELAY_TOKEN.trim().length > 0,
      "--relay-url in existing-kernel mode requires CHARIOX_DRILL_C_RELAY_TOKEN")
    }
    return
  }
  assert.equal(options.expectedDaemonId, null, "--expected-daemon-id requires --existing-kernel")
  assert.equal(options.expectedMachineId, null, "--expected-machine-id requires --existing-kernel")
}

export function requiresDirectDockerAccess(mode) {
  // Drill C validates an ordinary Path-1 kernel, not the legacy shared-host
  // broker. Check same-user Docker access before creating a durable slice.
  return mode === "isolated_local" || mode === "existing_kernel"
}

export function assertRootlessWorkspaceRoot({ workspaceRoot, homeDir = os.homedir(), repositoryRoot = repoRoot }) {
  assert.ok(typeof workspaceRoot === "string" && workspaceRoot.length > 0,
    `existing-kernel mode requires --rootless-workspace-root or ${roomDirectDockerWorkspaceRootEnvironment}`)
  assert.ok(path.isAbsolute(workspaceRoot), "rootless workspace root must be absolute")
  const selectedRoot = path.resolve(workspaceRoot)
  assert.ok(isWithin(selectedRoot, "/var/tmp"), "rootless workspace root must be under explicit engine-visible /var/tmp")
  assert.ok(!pathsOverlap(selectedRoot, homeDir), "rootless workspace root must be outside the invoking user's home")
  assert.ok(!pathsOverlap(selectedRoot, repositoryRoot), "rootless workspace root must be outside the source repository")
  return selectedRoot
}

export async function createExistingKernelWorkspaceFixture({
  workspaceRoot,
  homeDir = os.homedir(),
  repositoryRoot = repoRoot,
}) {
  const selectedRoot = assertRootlessWorkspaceRoot({ workspaceRoot, homeDir, repositoryRoot })
  return await createRoomDirectDockerWorkspaceFixture({
    workspaceRoot: selectedRoot,
    forbiddenRoots: [repositoryRoot, homeDir],
    // This proves only the invoking user's traversal and write access. The real rootless
    // engine mount check is CreateSlice followed by StartSlice through the selected kernel.
    verifyEngineAccess: probeInvokingUserWorkspaceAccess,
  })
}

async function probeInvokingUserWorkspaceAccess(target, { writable }) {
  const accessMode = fsConstants.R_OK | fsConstants.X_OK | (writable ? fsConstants.W_OK : 0)
  await access(target, accessMode)
  if (!writable) return

  const probePath = path.join(target, `.drill-c-write-probe-${process.pid}`)
  const probe = await open(probePath, "wx", 0o600)
  try {
    await probe.writeFile("workspace write probe")
    await probe.sync()
  } finally {
    await probe.close()
    await unlink(probePath)
  }
}

export function assertLoopbackUrl(value, label, protocols) {
  let url
  try {
    url = new URL(value)
  } catch {
    throw new Error(`${label} must be an absolute URL`)
  }
  assert.ok(protocols.includes(url.protocol), `${label} must use ${protocols.join(" or ")}`)
  assert.ok(["127.0.0.1", "localhost", "[::1]", "::1"].includes(url.hostname), `${label} must use a loopback host`)
  assert.equal(url.username, "", `${label} must not contain a username`)
  assert.equal(url.password, "", `${label} must not contain a password`)
  assert.equal(url.search, "", `${label} must not contain a query`)
  assert.equal(url.hash, "", `${label} must not contain a fragment`)
  return url
}

export function assertCloudRelayBootstrap({ bootstrap, relayUrl, relayToken, daemonId, machineId, sessionId, sessions, scopedThumbprint = null }) {
  assert.ok(bootstrap && typeof bootstrap === "object", "local Cloud omitted relay bootstrap")
  assert.ok(bootstrap.relayUrl === relayUrl, "local Cloud bootstrap selected a different relay")
  if (scopedThumbprint) {
    assert.notEqual(bootstrap.relayToken, relayToken, "Cloud must issue a distinct scoped browser credential")
    const tokenParts = bootstrap.relayToken?.split(".")
    assert.equal(tokenParts?.length, 3, "Cloud omitted a signed scoped browser credential")
    const claims = JSON.parse(Buffer.from(tokenParts[1], "base64url").toString("utf8"))
    assert.equal(claims.public_key_thumbprint, scopedThumbprint,
      "Cloud browser credential does not bind the viewer key")
    assert.deepEqual(claims.allowed_targets, [daemonId],
      "Cloud browser credential selected a different kernel")
  } else {
    assert.ok(bootstrap.relayToken === relayToken, "local Cloud bootstrap token does not match the isolated local relay")
  }
  assert.ok(bootstrap.target?.daemonId === daemonId, "local Cloud bootstrap selected a different kernel")
  assert.ok(bootstrap.target?.machineId === machineId, "local Cloud bootstrap selected a different machine")
  assert.ok(Array.isArray(sessions), "relay kernel session list is malformed")
  assert.ok(sessions.some((session) => session?.id === sessionId),
    "local Cloud relay client cannot see the setup Room session")
  return {
    status: "verified",
    relayUrl,
    targetDaemonId: daemonId,
    targetMachineId: machineId,
    sessionVisible: true,
  }
}

export function assertExistingKernelSnapshot({ sessions, slices, expectedDaemonId, expectedMachineId = null }) {
  assert.ok(Array.isArray(sessions), "existing kernel returned a malformed session inventory")
  assert.ok(Array.isArray(slices), "existing kernel returned a malformed slice inventory")
  const sessionIds = uniqueIds(sessions, "session")
  const sliceIds = uniqueIds(slices, "slice")
  const machineIds = new Set()
  for (const session of sessions) {
    assert.equal(session.host_daemon_id, expectedDaemonId,
      `existing kernel session ${session.id} belongs to a different daemon`)
    assert.ok(typeof session.host_machine_id === "string" && session.host_machine_id.length > 0,
      `existing kernel session ${session.id} omitted its machine identity`)
    machineIds.add(session.host_machine_id)
  }
  for (const slice of slices) {
    assert.equal(slice.owner_kernel_id, expectedDaemonId,
      `existing kernel slice ${slice.id} belongs to a different daemon`)
    assert.ok(typeof slice.owner_machine_id === "string" && slice.owner_machine_id.length > 0,
      `existing kernel slice ${slice.id} omitted its machine identity`)
    machineIds.add(slice.owner_machine_id)
  }
  assert.ok(sessionIds.length + sliceIds.length > 0,
    "cannot verify existing kernel identity before mutation: ListSessions and ListSlices returned no identity records")
  assert.equal(machineIds.size, 1, "existing kernel inventories disagree about machine identity")
  const machineId = [...machineIds][0]
  if (expectedMachineId != null) {
    assert.equal(machineId, expectedMachineId, "existing kernel belongs to a different machine")
  }
  return {
    sessionIds,
    sliceIds,
    sessionCount: sessionIds.length,
    sliceCount: sliceIds.length,
    machineId,
  }
}

export function assertNewSliceIdentity({
  slice,
  expectedDaemonId,
  expectedMachineId,
  workspace,
  priorSliceIds,
  requireDisplayPorts = false,
}) {
  assertNewId(slice?.id, priorSliceIds, "slice")
  assert.equal(slice.owner_kernel_id, expectedDaemonId, "new slice belongs to a different daemon")
  assert.equal(slice.owner_machine_id, expectedMachineId, "new slice belongs to a different machine")
  assert.equal(slice.workspace_id, workspace, "new slice retained a different workspace identity")
  assert.equal(slice.worktree_id, workspace, "new slice retained a different worktree identity")
  assert.equal(slice.workspace_mount, workspace, "new slice retained a different workspace mount")
  assert.equal(slice.backend, "local_docker", "new slice must use the local Docker backend")
  assert.equal(slice.display_mode, "headed", "new slice must use a headed display")
  assert.equal(slice.display_endpoint?.kind, "selkies", "new slice must use Selkies")
  if (requireDisplayPorts) {
    assert.ok(slice.local_docker_ports && typeof slice.local_docker_ports === "object",
      "running slice omitted its local Docker display ports")
    assert.ok(Number.isInteger(slice.local_docker_ports.novnc)
      && slice.local_docker_ports.novnc > 0 && slice.local_docker_ports.novnc <= 65_535,
    "running slice omitted its assigned Selkies display port")
  }
  return slice
}

export function assertNewSessionIdentity({ session, createdAgent, expectedDaemonId, expectedMachineId, workspace, worktree, priorSessionIds }) {
  assertNewId(session?.id, priorSessionIds, "session")
  assert.equal(session.host_daemon_id, expectedDaemonId, "new Room belongs to a different daemon")
  assert.equal(session.host_machine_id, expectedMachineId, "new Room belongs to a different machine")
  assert.equal(session.workspace_id, workspace, "new Room retained a different workspace identity")
  assert.equal(session.worktree_id, worktree, "new Room retained a different worktree identity")
  assert.equal(session.active_provider_run_id, null, "new Room unexpectedly has an active provider run")
  assert.ok(Array.isArray(session.agents), "new Room omitted its agent inventory")
  assert.equal(session.agents.length, 1, "new Room unexpectedly contains extra agents")
  assert.ok(createdAgent?.id, "new Room omitted its default agent")
  assert.equal(createdAgent.session_id, session.id, "new Room default agent belongs to a different Room")
  assert.equal(createdAgent.worktree_id, worktree, "new Room default agent belongs to a different worktree")
  assert.equal(session.agents[0].id, createdAgent.id, "new Room default agent mismatch")
  return session
}

export function assertRoomSliceBinding(binding, { sessionId, slice, daemonId }) {
  assert.equal(binding?.session_id, sessionId, "Room is bound to a different session")
  assert.equal(binding?.slice_id, slice.id, "Room is bound to a different slice")
  assert.equal(binding?.owner_kernel_id, daemonId, "Room is owned by a different daemon")
  assert.equal(binding?.worker_kernel_ref, slice.worker_kernel_ref, "Room binding points to a different worker kernel")
  return binding
}

export function buildRoomBaseline({ sessionId, sliceId, environment, resourceInventory, actionHistory, capturedAt }) {
  assert.equal(environment?.session_id, sessionId, "baseline environment belongs to a different Room")
  assert.equal(environment?.lifecycle, "ready", "baseline Room environment is not ready")
  assert.equal(resourceInventory?.session_id, sessionId, "baseline inventory belongs to a different Room")
  assert.equal(resourceInventory?.slice_id, sliceId, "baseline inventory belongs to a different slice")
  assert.equal(resourceInventory?.environment_id, environment.environment_id,
    "baseline inventory belongs to a different environment")
  assert.ok(Array.isArray(actionHistory?.actions), "baseline omitted kernel action history")
  assert.deepEqual(actionHistory.actions, [], "new Room action history is not empty at the baseline checkpoint")
  assert.equal(actionHistory.next_before_sequence, null, "baseline action history has an unexpected continuation cursor")
  assert.ok(Array.isArray(environment.actions), "baseline omitted the Room snapshot action list")
  assert.deepEqual(environment.actions, [], "new Room snapshot already contains actions at the baseline checkpoint")
  assert.ok(Array.isArray(environment.tabs), "baseline omitted the Room tab inventory")
  const focusedTab = environment.tabs?.find((tab) => tab.tab_id === environment.focused_tab_id)
  assert.ok(focusedTab, "baseline requires the kernel focused tab record")
  assert.ok(Array.isArray(resourceInventory.browser_ids) && resourceInventory.browser_ids.length > 0
    && resourceInventory.browser_ids.every((id) => typeof id === "string" && id.length > 0),
    "baseline requires a live browser identity")
  assert.ok(Array.isArray(resourceInventory.profile_ids) && resourceInventory.profile_ids.length > 0
    && resourceInventory.profile_ids.every((id) => typeof id === "string" && id.length > 0),
    "baseline requires a live browser profile identity")
  return {
    capturedAt,
    source: "kernel public Room and resource inventory requests",
    sessionId,
    sliceId,
    environmentId: environment.environment_id,
    runtimeGeneration: environment.runtime_generation,
    lifecycle: environment.lifecycle,
    focusedTabId: environment.focused_tab_id,
    viewport: environment.viewport,
    tabs: environment.tabs,
    browserIds: [...resourceInventory.browser_ids],
    profileIds: [...resourceInventory.profile_ids],
    actionHistory: [...actionHistory.actions],
    environmentActions: [...(environment.actions ?? [])],
  }
}

function uniqueIds(records, kind) {
  const ids = records.map((record) => record?.id)
  assert.ok(ids.every((id) => typeof id === "string" && id.length > 0),
    `existing kernel returned a ${kind} without an id`)
  assert.equal(new Set(ids).size, ids.length, `existing kernel returned duplicate ${kind} ids`)
  return ids
}

function assertNewId(id, priorIds, kind) {
  assert.ok(typeof id === "string" && id.length > 0, `kernel did not return a ${kind} id`)
  assert.ok(!priorIds.includes(id), `new ${kind} id collides with an existing ${kind}`)
}

export function buildSetupManifest({
  mode = "isolated_local",
  createdAt,
  sourceCommit,
  rootDir,
  manifestPath,
  cloudUrl,
  kernelUrl,
  relayUrl,
  daemonId,
  daemonAlias,
  machineId,
  machineAlias,
  session,
  slice,
  binding,
  environment,
  resourceInventory,
  workspace,
  worktree,
  transport,
  baseline,
  priorKernelState,
}) {
  for (const [name, value] of Object.entries({
    sourceCommit,
    rootDir,
    manifestPath,
    cloudUrl,
    kernelUrl,
    daemonId,
    machineId,
    sessionId: session?.id,
    sliceId: slice?.id,
    environmentId: environment?.environment_id,
  })) assert.ok(typeof value === "string" && value.length > 0, `setup manifest requires ${name}`)
  if (mode === "isolated_local") {
    assert.ok(typeof relayUrl === "string" && relayUrl.length > 0, "isolated setup manifest requires its local relay URL")
    assert.equal(transport?.status, "verified", "isolated setup manifest requires verified local Cloud relay visibility")
    assert.equal(transport?.relayUrl, relayUrl, "setup manifest Cloud transport selected a different relay")
    assert.equal(transport?.targetDaemonId, daemonId, "setup manifest Cloud transport selected a different kernel")
    assert.equal(transport?.targetMachineId, machineId, "setup manifest Cloud transport selected a different machine")
    assert.equal(transport?.sessionVisible, true, "setup manifest Cloud transport did not observe the Room session")
  } else {
    assert.equal(mode, "existing_kernel", "setup manifest has an unknown mode")
    if (relayUrl != null) {
      const relayEndpoint = assertLoopbackUrl(relayUrl, "existing-kernel relay URL", ["ws:"])
      assert.ok(["", "/"].includes(relayEndpoint.pathname),
        "existing-kernel relay URL must address the same-host relay WebSocket endpoint")
      assert.notEqual(relayEndpoint.origin, new URL(kernelUrl).origin,
        "existing-kernel relay URL must use an endpoint distinct from the local kernel")
    }
    assert.equal(transport?.status, "not_observed", "existing-kernel mode must leave Cloud relay transport unobserved")
    assert.equal(transport?.sessionVisible, null, "existing-kernel mode must not claim the Room is visible to Cloud")
    assert.equal(transport?.relayUrl ?? null, null, "existing-kernel mode must not claim Cloud selected a relay")
  }
  assert.ok(path.isAbsolute(rootDir) && path.isAbsolute(manifestPath), "setup manifest paths must be absolute")
  assert.ok(path.isAbsolute(workspace) && path.isAbsolute(worktree), "Room workspace and worktree must be absolute")
  assertRoomSliceBinding(binding, { sessionId: session.id, slice, daemonId })
  assert.equal(environment?.session_id, session.id, "setup manifest environment Room mismatch")
  assert.equal(environment?.lifecycle, "ready", "setup manifest Room environment is not ready")
  assert.equal(resourceInventory?.environment_id, environment.environment_id, "setup manifest inventory Environment mismatch")
  assert.equal(resourceInventory?.session_id, session.id, "setup manifest inventory Room mismatch")
  assert.equal(resourceInventory?.slice_id, slice.id, "setup manifest inventory slice mismatch")
  assert.equal(slice.backend, "local_docker", "setup manifest slice must use the local Docker backend")
  assert.equal(slice.workspace_mount, workspace, "setup manifest slice does not use the Room workspace")
  assert.equal(slice.display_mode, "headed", "setup manifest slice must use a headed display")
  assert.equal(slice.display_endpoint?.kind, "selkies", "setup manifest slice must use the Selkies display backend")
  assert.ok(typeof slice.display_endpoint?.url === "string" && slice.display_endpoint.url.length > 0,
    "headed slice omitted the live display endpoint")
  assert.ok(slice.local_docker_ports && typeof slice.local_docker_ports === "object",
    "setup manifest requires the local Docker display port identity")
  assert.ok(Number.isInteger(slice.local_docker_ports.novnc)
    && slice.local_docker_ports.novnc > 0 && slice.local_docker_ports.novnc <= 65_535,
  "setup manifest requires the assigned Selkies display port")
  assert.ok(Array.isArray(resourceInventory?.browser_ids) && resourceInventory.browser_ids.length > 0,
    "setup manifest requires live browser identities")
  assert.ok(Array.isArray(resourceInventory?.profile_ids) && resourceInventory.profile_ids.length > 0,
    "setup manifest requires live browser profile identities")
  const focusedTab = environment.tabs?.find((tab) => tab.tab_id === environment.focused_tab_id)
  assert.ok(focusedTab, "setup manifest requires the kernel focused tab record")
  assert.equal(baseline?.sessionId, session.id, "setup manifest baseline belongs to a different Room")
  assert.equal(baseline?.sliceId, slice.id, "setup manifest baseline belongs to a different slice")
  assert.equal(baseline?.environmentId, environment.environment_id,
    "setup manifest baseline belongs to a different environment")
  assert.equal(baseline?.lifecycle, "ready", "setup manifest baseline Room is not ready")
  assert.equal(baseline?.runtimeGeneration, environment.runtime_generation,
    "setup manifest baseline belongs to a different runtime generation")
  assert.equal(baseline?.focusedTabId, environment.focused_tab_id,
    "setup manifest baseline focused tab does not match the kernel snapshot")
  assert.deepEqual(baseline?.browserIds, resourceInventory.browser_ids,
    "setup manifest baseline browser inventory does not match the kernel inventory")
  assert.deepEqual(baseline?.profileIds, resourceInventory.profile_ids,
    "setup manifest baseline profile inventory does not match the kernel inventory")
  assert.deepEqual(baseline?.actionHistory, [], "setup manifest baseline contains unobserved Room actions")
  assert.ok(priorKernelState && Number.isInteger(priorKernelState.sessionCount)
    && Number.isInteger(priorKernelState.sliceCount), "setup manifest requires the read-only pre-setup kernel inventory")

  const tuiManifestPath = path.join(rootDir, "tui-observer", "manifest.json")
  const webObservationPath = path.join(rootDir, "evidence", "drill-c-live-observation.json")
  const tuiObserverArgs = [
    path.join(repoRoot, "apps", "cli", "scripts", "live-tui-web-parity-visual-session.mjs"),
    "--observe-room-session", session.id,
    "--kernel-url", kernelUrl,
    "--slice-id", slice.id,
    "--workspace", workspace,
    "--worktree", worktree,
    "--web-observation", webObservationPath,
    "--root-dir", path.join(rootDir, "tui-observer"),
    "--manifest", tuiManifestPath,
    ...(relayUrl ? ["--relay-url", relayUrl, "--target-daemon-id", daemonId] : []),
  ]
  return {
    schema: "chariox.drill_c.same_host_setup.v1",
    status: "ready_for_observers",
    setupMode: mode,
    createdAt,
    sourceCommit,
    rootDir,
    manifestPath,
    kernel: { url: kernelUrl, daemonId, daemonAlias, machineId, machineAlias },
    kernelUrl,
    relayUrl,
    sessionId: session.id,
    sliceId: slice.id,
    workspace,
    worktree,
    webObservationPath,
    room: {
      sessionId: session.id,
      environmentId: environment.environment_id,
      runtimeGeneration: environment.runtime_generation,
      focusedTabId: environment.focused_tab_id,
      sliceBinding: binding,
      browserIds: [...resourceInventory.browser_ids],
      profileIds: [...resourceInventory.profile_ids],
      viewport: environment.viewport,
      tabs: environment.tabs,
    },
    slice: {
      id: slice.id,
      name: slice.name,
      backend: slice.backend,
      displayMode: slice.display_mode,
      displayBackend: slice.display_endpoint.kind,
      workerKernelRef: binding.worker_kernel_ref,
      workspaceMount: slice.workspace_mount,
    },
    display: {
      sliceId: slice.id,
      environmentId: environment.environment_id,
      runtimeGeneration: environment.runtime_generation,
      focusedTabId: environment.focused_tab_id,
      mode: slice.display_mode,
      backend: slice.display_endpoint.kind,
      localDockerPorts: slice.local_docker_ports ?? null,
      viewport: environment.viewport,
      browserIds: [...resourceInventory.browser_ids],
      profileIds: [...resourceInventory.profile_ids],
    },
    baseline,
    priorKernelState,
    localCloudTransport: mode === "isolated_local"
      ? {
          status: transport.status,
          relayUrl: transport.relayUrl,
          targetDaemonId: transport.targetDaemonId,
          targetMachineId: transport.targetMachineId,
          sessionVisible: transport.sessionVisible,
          verifiedAt: transport.verifiedAt,
          cloudApiUrl: transport.cloudApiUrl,
        }
      : {
          status: "not_observed",
          relayUrl: null,
          expectedDaemonId: daemonId,
          expectedMachineId: machineId,
          sessionVisible: null,
          verificationRequiredFrom: "Mac local Cloud frontend",
          reason: "Room was created through the existing kernel local API; Cloud and Web visibility were not checked from this host.",
        },
    webObserver: {
      client: "production-local-web-view",
      openUrl: cloudUrl ? `${cloudUrl.replace(/\/$/, "")}/waiting-room` : null,
      targetDaemonId: daemonId,
      sessionId: session.id,
      relayUrl: mode === "existing_kernel" ? null : relayUrl,
      observationPath: webObservationPath,
      evidenceSchema: "chariox.browser_computer.drill_c.web_observer.v1",
      requiredEvidence: [
        "sessionId, environmentId, sliceId, runtimeGeneration",
        "display.viewport, display.browserIds, display.profileIds",
        "browser.focusedTabId and browser.tab { tabId, url, title, documentRevision }",
        "actions.computer and actions.webTakeover with actionId, actorId, sequence, mode, kind, state",
      ],
      evidenceStatus: "not_observed",
      transportStatus: mode === "existing_kernel" ? "not_observed" : "verified",
      verificationRequiredFrom: mode === "existing_kernel" ? "Mac local Cloud frontend" : null,
    },
    tuiObserver: {
      executable: "node",
      args: tuiObserverArgs,
      manifestPath: tuiManifestPath,
      remoteRelay: {
        url: relayUrl,
        targetDaemonId: daemonId,
        credentialEnvironment: "CHARIOX_DRILL_C_RELAY_TOKEN",
        status: relayUrl ? "relay_url_selected" : "relay_url_required",
      },
      verify: {
        executable: "node",
        args: [
          path.join(repoRoot, "apps", "cli", "scripts", "tui-web-parity-visual-control.mjs"),
          "--manifest", tuiManifestPath,
          "--action", "drill-c-verify",
        ],
      },
    },
    cleanup: mode === "isolated_local"
      ? "Ctrl+C stops the Room environment, deletes the drill slice and session, then stops this harness's kernel and relay."
      : "Ctrl+C stops and deletes only this harness's Room and slice, then removes its inode-guarded workspace after matching cleanup acknowledgements; the selected kernel and relay remain running.",
  }
}

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/live-room-drill-c-setup.mjs [options]",
    "",
    "Options:",
    "  --root-dir PATH",
    "  --manifest PATH",
    "  --existing-kernel URL              use the already-running local kernel; do not start or stop it",
    "  --allow-slice-image-build          permit auto-building the isolated slice image; default requires a compatible cache",
    "  --allow-provider-sandbox-compatibility  opt in to the provisioner's Docker grants and pre-start Bubblewrap probe for an isolated slice",
    "  --rootless-workspace-root PATH     required for existing-kernel; safe root under /var/tmp",
    "  --expected-daemon-id ID             required with --existing-kernel",
    "  --expected-machine-id ID            optional additional identity check",
    "  --local-cloud-url URL             default: http://127.0.0.1:4321",
    "  --relay-url URL                   Cloud relay in isolated mode; same-host relay with --existing-kernel",
    "  --relay-token TOKEN               isolated mode only; existing-kernel reads CHARIOX_DRILL_C_RELAY_TOKEN",
    "  --active-kernel-registry-dir PATH defaults to ~/.chariox/kernels/active",
    "  --kernel-binary PATH              prebuilt binary; this script never builds it",
    "  --relay-binary PATH               prebuilt binary; this script never builds it",
    "",
    "The script waits after setup so the attach-only TUI and Web observers can run.",
    "Isolated mode defaults to slices.linux.build_image = never and fails before container mutation if the cached image is missing or stale. --allow-slice-image-build restores auto-build behavior and may trigger a costly release compile.",
    "An isolated real-provider slice may need --allow-provider-sandbox-compatibility; this grants the provisioner's bounded Docker setup capabilities and fails before runtime start if its Bubblewrap probe fails.",
    `Existing-kernel mode requires a loopback ws:// URL and an explicit --rootless-workspace-root (or ${roomDirectDockerWorkspaceRootEnvironment}); --relay-url additionally requires CHARIOX_DRILL_C_RELAY_TOKEN. Cloud/Web transport stays unobserved.`,
    "Ctrl+C cleans up only this run's Room and slice; its workspace is removed only after matching cleanup acknowledgements.",
  ].join("\n"))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  assertModeOptions(options, process.env)
  assertSetupPaths(options)
  assertLoopbackUrl(options.localCloudUrl, "--local-cloud-url", ["http:"])

  const existingKernel = options.mode === "existing_kernel"
  const cloud = existingKernel ? null : await connectLocalCloud(options.localCloudUrl)
  let relayUrl = options.mode === "existing_kernel" ? options.relayUrl : null
  let ports = null
  let kernelUrl = options.existingKernelUrl
  let daemonId = options.expectedDaemonId
  let daemonAlias = null
  let machineId = null
  let machineAlias = null
  if (!existingKernel) {
    const initialDashboard = await cloud.dashboard()
    relayUrl = selectLocalRelayUrl(initialDashboard, options.relayUrl)
    const relayEndpoint = assertLoopbackUrl(relayUrl, "local Cloud relay URL", ["ws:"])
    assert.ok(relayEndpoint.pathname === "/", "local Cloud relay URL must not contain a path")
    const relayPort = Number(relayEndpoint.port || "80")
    const relayHost = relayEndpoint.hostname.replace(/^\[|\]$/g, "")
    assert.ok(await portIsAvailable(relayPort, relayHost),
      `isolated relay port ${relayPort} is already in use; stop its owner or configure the local Cloud frontend and rerun with a free matching relay URL`)
    ports = await allocateKernelPorts([relayPort])
    await assertExecutable(options.kernelBinary, "prebuilt chariox-kernel")
    await assertExecutable(options.relayBinary, "prebuilt chariox-relay")
    daemonId = `drill-c-home-${process.pid}-${Date.now()}`
    daemonAlias = daemonId
    machineId = `drill-c-machine-${process.pid}-${Date.now()}`
    machineAlias = machineId
    kernelUrl = `ws://127.0.0.1:${ports.kernel}/kernel`
    if (options.scopedRelayIssuer) {
      options.relayToken = mintDrillCKernelRelayToken({
        issuer: options.scopedRelayIssuer,
        secret: options.scopedRelaySecret,
        machineId,
        daemonId,
      })
    }
  }
  await assertKernelClientBuilt()
  if (requiresDirectDockerAccess(options.mode)) await assertDockerReady()

  const stamp = `${process.pid}-${Date.now()}`
  const sliceName = `drill-c-${stamp}`
  const workerKernelRef = `drill-c-worker-${stamp}`
  const kernelHome = path.join(options.rootDir, "kernel-home")
  let workspace = path.join(options.rootDir, "workspace")
  let worktree = workspace
  const logDir = path.join(options.rootDir, "logs")
  const clientModule = await import(pathToFileURL(path.join(kernelClientRoot, "dist", "ipc.js")).href)
  const requests = await import(pathToFileURL(path.join(kernelClientRoot, "dist", "ipc-requests.js")).href)
  const { LocalIpcClient } = clientModule
  const children = []
  const state = {
    client: null,
    sessionId: null,
    sliceId: null,
    workspace,
    workspaceOwned: false,
    rootlessWorkspaceFixture: null,
    sliceCreateAttempted: false,
    sessionCreateAttempted: false,
    roomEnvironmentStartAttempted: false,
    presenceRecordPath: existingKernel ? null : path.join(options.activeKernelRegistryDir, `${daemonId}.json`),
    manifest: null,
    manifestPath: options.manifestPath,
  }
  let stopped = false
  let stopSignal = null
  const stopRequested = new Promise((resolve) => {
    const requestStop = (signal) => {
      if (stopped) return
      stopped = true
      stopSignal = signal
      resolve()
    }
    process.once("SIGINT", () => requestStop("SIGINT"))
    process.once("SIGTERM", () => requestStop("SIGTERM"))
  })

  try {
    await mkdir(devRoot, { recursive: true, mode: 0o700 })
    await mkdir(options.rootDir, { recursive: false, mode: 0o700 })
    if (existingKernel) {
      state.rootlessWorkspaceFixture = await createExistingKernelWorkspaceFixture({
        workspaceRoot: options.rootlessWorkspaceRoot,
        homeDir: options.homeDir,
        repositoryRoot: repoRoot,
      })
      workspace = state.rootlessWorkspaceFixture.workspace
      worktree = workspace
      state.workspace = workspace
    } else {
      await mkdir(workspace, { recursive: false, mode: 0o700 })
      state.workspaceOwned = true
      await chmod(workspace, 0o777)
    }
    await mkdir(logDir, { recursive: true, mode: 0o700 })
    if (!existingKernel) {
      await mkdir(kernelHome, { recursive: false, mode: 0o700 })
      await writeFile(path.join(kernelHome, "config.toml"), isolatedKernelUserConfig(options), {
        encoding: "utf8",
        mode: 0o600,
        flag: "wx",
      })
      await mkdir(path.dirname(options.activeKernelRegistryDir), { recursive: true, mode: 0o700 })
      await mkdir(options.activeKernelRegistryDir, { recursive: true, mode: 0o700 })
      const isolatedRegistryLink = path.join(kernelHome, "kernels", "active")
      await mkdir(path.dirname(isolatedRegistryLink), { recursive: true, mode: 0o700 })
      await symlink(options.activeKernelRegistryDir, isolatedRegistryLink, "dir")

      const relayEndpoint = new URL(relayUrl)
      const relayHost = relayEndpoint.hostname.replace(/^\[|\]$/g, "")
      const relayPort = Number(relayEndpoint.port || "80")
      children.push(spawnLogged("relay", options.relayBinary,
        relayProcessEnvironment(relayUrl, relayHost, relayPort, options), logDir))
      await waitForTcp(relayHost, relayPort, 15_000, children[0])

      children.push(spawnLogged("kernel", options.kernelBinary, kernelProcessEnvironment({
        daemonId,
        daemonAlias,
        machineId,
        machineAlias,
        kernelHome,
        kernelPort: ports.kernel,
        mcpPort: ports.mcp,
        codexPort: ports.codex,
        opencodePort: ports.opencode,
        relayUrl,
        relayToken: options.relayToken,
        logDir: path.join(logDir, "kernel-runtime"),
        sliceRoot: path.join(options.rootDir, "slices"),
        allowProviderSandboxCompatibility: options.allowProviderSandboxCompatibility,
      }), logDir))
    }
    state.client = await connectKernel(LocalIpcClient, requests, kernelUrl, children)

    const inventory = await readKernelInventory(state.client, requests)
    let priorKernelState
    if (existingKernel) {
      priorKernelState = assertExistingKernelSnapshot({
        ...inventory,
        expectedDaemonId: daemonId,
        expectedMachineId: options.expectedMachineId,
      })
      machineId = priorKernelState.machineId
    } else {
      assert.deepEqual(inventory.sessions, [], "new isolated kernel unexpectedly has existing Rooms")
      assert.deepEqual(inventory.slices, [], "new isolated kernel unexpectedly has existing slices")
      priorKernelState = { sessionCount: 0, sliceCount: 0 }
    }

    state.sliceCreateAttempted = true
    const createdSlice = unwrap(await state.client.send(requests.createSliceRequest({
      name: sliceName,
      backend: "local_docker",
      displayMode: "headed",
      displayBackend: "selkies",
      workspaceId: workspace,
      worktreeId: worktree,
      workspaceMount: workspace,
      workerKernelRef,
      base: "clean",
    })), "SliceCreated").slice
    assertNewId(createdSlice?.id, priorKernelState.sliceIds ?? [], "slice")
    state.sliceId = createdSlice.id
    let slice = assertNewSliceIdentity({
      slice: createdSlice,
      expectedDaemonId: daemonId,
      expectedMachineId: machineId,
      workspace,
      priorSliceIds: priorKernelState.sliceIds ?? [],
    })
    state.sessionCreateAttempted = true
    const created = unwrap(await state.client.send(requests.createSessionRequest(
      workspace,
      worktree,
      `Drill C same-host ${stamp}`,
    )), "SessionCreated")
    const { session, agent: createdAgent } = created
    assertNewId(session?.id, priorKernelState.sessionIds ?? [], "session")
    state.sessionId = session.id
    assertNewSessionIdentity({
      session,
      createdAgent,
      expectedDaemonId: daemonId,
      expectedMachineId: machineId,
      workspace,
      worktree,
      priorSessionIds: priorKernelState.sessionIds ?? [],
    })
    const binding = unwrap(await state.client.send(requests.bindRoomEnvironmentSliceRequest(state.sessionId, state.sliceId)),
      "RoomEnvironmentSlice").binding
    assertRoomSliceBinding(binding, { sessionId: state.sessionId, slice, daemonId })

    // The worker reads its Room binding when the slice starts. Binding an
    // already-running slice leaves that worker without the provisioned scope.
    slice = await startSliceAndWaitForRunning(state.client, requests, state.sliceId, children)
    slice = assertNewSliceIdentity({
      slice,
      expectedDaemonId: daemonId,
      expectedMachineId: machineId,
      workspace,
      priorSliceIds: priorKernelState.sliceIds ?? [],
      requireDisplayPorts: true,
    })

    state.roomEnvironmentStartAttempted = true
    unwrap(await state.client.send(requests.startRoomEnvironmentRequest(state.sessionId, {
      css_width: 1280,
      css_height: 800,
      device_scale_factor: 1,
      desktop_pixel_width: 1280,
      desktop_pixel_height: 800,
    })), "RoomEnvironmentUpdated")
    const { environment, resourceInventory, actionHistory } = await waitForRoomReady(
      state.client,
      requests,
      state.sessionId,
      slice,
      daemonId,
      children,
    )

    const baseline = buildRoomBaseline({
      sessionId: state.sessionId,
      sliceId: state.sliceId,
      environment,
      resourceInventory,
      actionHistory,
      capturedAt: new Date().toISOString(),
    })
    const transport = existingKernel
      ? { status: "not_observed", sessionVisible: null }
      : await waitForCloudBootstrap({
          cloud,
          daemonId,
          machineId,
          relayUrl,
          relayToken: options.relayToken,
          scopedThumbprint: options.scopedRelayIssuer ? "a".repeat(64) : null,
          sessionId: state.sessionId,
          LocalIpcClient,
          requests,
          timeoutMs: 45_000,
          children,
        })
    const sourceCommit = (await execFileAsync("git", ["rev-parse", "HEAD"], { cwd: repoRoot })).stdout.trim()
    state.manifest = buildSetupManifest({
      mode: options.mode,
      createdAt: new Date().toISOString(),
      sourceCommit,
      rootDir: options.rootDir,
      manifestPath: options.manifestPath,
      cloudUrl: new URL(options.localCloudUrl).origin,
      kernelUrl,
      relayUrl,
      daemonId,
      daemonAlias,
      machineId,
      machineAlias,
      session,
      slice,
      binding,
      environment,
      resourceInventory,
      workspace,
      worktree,
      transport,
      baseline,
      priorKernelState: {
        sessionCount: priorKernelState.sessionCount,
        sliceCount: priorKernelState.sliceCount,
      },
    })
    await mkdir(path.dirname(options.manifestPath), { recursive: true, mode: 0o700 })
    await writeFile(options.manifestPath, `${JSON.stringify(state.manifest, null, 2)}\n`, {
      encoding: "utf8",
      mode: 0o600,
      flag: "wx",
    })
    console.log(`[drill-c-setup] ready-for-observers manifest: ${options.manifestPath}`)
    if (existingKernel) {
      console.log(`[drill-c-setup] Room ${state.sessionId} created on verified existing kernel ${daemonId}`)
      console.log("[drill-c-setup] Cloud/Web transport remains not observed; root must verify it from the Mac local Cloud frontend")
      console.log(`[drill-c-setup] Mac-side Cloud URL hint (not probed here): ${state.manifest.webObserver.openUrl}`)
    } else {
      console.log(`[drill-c-setup] local Cloud transport verified for Room ${state.sessionId} via target ${daemonId}`)
    }
    console.log(`[drill-c-setup] TUI observer args: ${JSON.stringify(state.manifest.tuiObserver.args)}`)
    if (!existingKernel) {
      console.log(`[drill-c-setup] open ${state.manifest.webObserver.openUrl}; use the Web observer contract in the manifest`)
    }
    console.log("[drill-c-setup] run the manifest verify command after Web takeover, then press Ctrl+C to clean up")
    await stopRequested
  } catch (error) {
    console.error(`[drill-c-setup] ${error?.stack ?? String(error)}`)
    process.exitCode = 1
  } finally {
    const cleanupErrors = await cleanupOwnedResources({ state, requests, children })
    for (const message of cleanupErrors) console.error(`[drill-c-setup] cleanup: ${message}`)
    if (cleanupErrors.length > 0) process.exitCode = 1
    if (state.manifest && stopSignal) {
      state.manifest = {
        ...state.manifest,
        status: cleanupErrors.length === 0 ? "stopped" : "cleanup_incomplete",
        stoppedAt: new Date().toISOString(),
      }
      await writeFile(options.manifestPath, `${JSON.stringify(state.manifest, null, 2)}\n`, { encoding: "utf8", mode: 0o600 })
      console.log(`[drill-c-setup] stopped after ${stopSignal}`)
    }
  }
}

function assertSetupPaths(options) {
  assert.ok(isWithin(options.rootDir, devRoot), "--root-dir must be under ~/.chariox/dev/browser-computer-use")
  assert.ok(!isWithin(options.rootDir, repoRoot), "setup state must be outside the repository")
  assert.ok(isWithin(options.manifestPath, options.rootDir), "--manifest must be inside --root-dir")
  if (options.mode === "isolated_local") {
    assert.ok(!isWithin(options.activeKernelRegistryDir, repoRoot), "active kernel registry must be outside the repository")
    assert.ok(options.relayToken.trim().length > 0, "local relay token must not be empty")
  }
}

function isWithin(candidate, parent) {
  const relative = path.relative(parent, candidate)
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative))
}

function pathsOverlap(left, right) {
  return isWithin(left, right) || isWithin(right, left)
}

function selectLocalRelayUrl(dashboard, configuredRelayUrl) {
  const realmRelayUrl = dashboard?.realm?.relayUrl
  assert.ok(typeof realmRelayUrl === "string" && realmRelayUrl.length > 0,
    "local Cloud dashboard omitted its configured relay URL")
  assertLoopbackUrl(realmRelayUrl, "local Cloud relay URL", ["ws:"])
  if (configuredRelayUrl != null) {
    assert.equal(configuredRelayUrl, realmRelayUrl, "--relay-url must exactly match local Cloud's configured relay URL")
  }
  return realmRelayUrl
}

async function connectLocalCloud(rawUrl) {
  const baseUrl = assertLoopbackUrl(rawUrl, "--local-cloud-url", ["http:"]).origin
  const shellResponse = await fetch(new URL("/waiting-room", baseUrl))
  assert.equal(shellResponse.status, 200, "local Cloud frontend /waiting-room is unavailable")
  assert.match(shellResponse.headers.get("content-type") ?? "", /text\/html/i,
    "local Cloud /waiting-room is not the production-local Web frontend")

  const csrfResponse = await fetch(new URL("/auth/csrf", baseUrl))
  const csrfBody = await responseJson(csrfResponse, "/auth/csrf")
  assert.equal(csrfResponse.status, 200, "local Cloud CSRF bootstrap failed")
  assert.ok(typeof csrfBody.csrfToken === "string" && csrfBody.csrfToken.length > 0,
    "local Cloud omitted its CSRF token")
  const csrfCookie = responseCookies(csrfResponse).join("; ")
  assert.ok(csrfCookie.length > 0, "local Cloud omitted its CSRF cookie")

  const sessionResponse = await fetch(new URL("/auth/cloud-session", baseUrl), {
    method: "POST",
    headers: {
      "content-type": "application/json",
      cookie: csrfCookie,
      "csrf-token": csrfBody.csrfToken,
    },
    body: "{}",
  })
  const sessionBody = await responseJson(sessionResponse, "/auth/cloud-session")
  assert.equal(sessionResponse.status, 200, "local Cloud browser session bootstrap failed")
  assert.ok(typeof sessionBody.cloudSessionToken === "string" && sessionBody.cloudSessionToken.length > 0,
    "local Cloud omitted its local browser session token")
  const cloudSessionToken = sessionBody.cloudSessionToken
  const sessionCookies = responseCookies(sessionResponse)
  const cookie = [...new Set([...csrfCookie.split("; "), ...sessionCookies])].filter(Boolean).join("; ")

  async function requestJson(route, { method = "GET", body = undefined } = {}) {
    const response = await fetch(new URL(route, baseUrl), {
      method,
      headers: {
        accept: "application/json",
        ...(method === "GET" ? {} : { "content-type": "application/json" }),
        cookie,
        "x-chariox-browser-session": cloudSessionToken,
        ...(method === "GET" ? {} : { "csrf-token": csrfBody.csrfToken }),
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    })
    const payload = await responseJson(response, route)
    assert.ok(response.ok, `local Cloud ${route} failed with status ${response.status}`)
    return payload
  }

  return {
    baseUrl,
    async dashboard() {
      return await requestJson("/dashboard")
    },
    async bootstrap(targetDaemonId, publicKeyThumbprint = null) {
      return await requestJson("/browser/relay-kernel/bootstrap", {
        method: "POST",
        body: { targetDaemonId, ...(publicKeyThumbprint ? { publicKeyThumbprint } : {}) },
      })
    },
  }
}

async function responseJson(response, route) {
  try {
    return await response.json()
  } catch {
    throw new Error(`local Cloud ${route} returned invalid JSON`)
  }
}

function responseCookies(response) {
  const setCookies = typeof response.headers.getSetCookie === "function"
    ? response.headers.getSetCookie()
    : [response.headers.get("set-cookie")].filter(Boolean)
  return setCookies.map((value) => value.split(";", 1)[0]).filter(Boolean)
}

async function assertExecutable(binary, label) {
  try {
    await access(binary, fsConstants.X_OK)
  } catch {
    throw new Error(`missing ${label} at ${binary}; this setup script requires prebuilt binaries and never runs Cargo`)
  }
}

async function assertKernelClientBuilt() {
  for (const name of ["ipc.js", "ipc-requests.js"]) {
    const file = path.join(kernelClientRoot, "dist", name)
    try {
      await access(file)
    } catch {
      throw new Error(`missing ${file}; build the kernel-client before invoking this source-only setup harness`)
    }
  }
}

async function assertDockerReady() {
  try {
    await execFileAsync("docker", ["info", "--format", "{{.ServerVersion}}"], { timeout: 10_000 })
  } catch {
    throw new Error("local Docker engine is unavailable; no slice or Room was created")
  }
}

function relayProcessEnvironment(relayUrl, relayHost, relayPort, options) {
  const env = { ...process.env }
  for (const name of [
    "CHARIOX_RELAY_TOKEN",
    "CHARIOX_RELAY_SCOPED_ISSUER",
    "CHARIOX_RELAY_SCOPED_HMAC_SECRET",
    "CHARIOX_RELAY_ALLOW_OPEN_ACCESS",
  ]) {
    delete env[name]
  }
  return {
    ...env,
    CHARIOX_RELAY_HOST: relayHost,
    CHARIOX_RELAY_PORT: String(relayPort),
    CHARIOX_RELAY_URL: relayUrl,
    ...(options.scopedRelayIssuer
      ? {
          CHARIOX_RELAY_SCOPED_ISSUER: options.scopedRelayIssuer,
          CHARIOX_RELAY_SCOPED_HMAC_SECRET: options.scopedRelaySecret,
        }
      : { CHARIOX_RELAY_TOKEN: options.relayToken }),
  }
}

export function kernelProcessEnvironment(input) {
  const env = { ...process.env }
  for (const name of [
    "CHARIOX_RELAY_SCOPED_ISSUER",
    "CHARIOX_RELAY_SCOPED_HMAC_SECRET",
    "CHARIOX_RELAY_ALLOW_OPEN_ACCESS",
    "CHARIOX_PROVIDER_DEV_STUB",
    "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
    "CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY",
  ]) {
    delete env[name]
  }
  return {
    ...env,
    CHARIOX_HOME: input.kernelHome,
    CHARIOX_KERNEL_PORT: String(input.kernelPort),
    CHARIOX_MCP_PORT: String(input.mcpPort),
    CHARIOX_CODEX_PORT: String(input.codexPort),
    CHARIOX_OPENCODE_PORT: String(input.opencodePort),
    CHARIOX_DAEMON_ID: input.daemonId,
    CHARIOX_DAEMON_ALIAS: input.daemonAlias,
    CHARIOX_MACHINE_ID: input.machineId,
    CHARIOX_MACHINE_ALIAS: input.machineAlias,
    CHARIOX_RELAY_URL: input.relayUrl,
    CHARIOX_RELAY_TOKEN: input.relayToken,
    CHARIOX_ACCEPT_REMOTE_LEASES: "1",
    CHARIOX_LOG_DIR: input.logDir,
    CHARIOX_DAEMON_SOCKET: path.join(input.kernelHome, "daemon.sock"),
    CHARIOX_SESSION_HISTORY_DIR: path.join(input.kernelHome, "history"),
    CHARIOX_SLICE_ROOT: input.sliceRoot,
    ...(input.allowProviderSandboxCompatibility
      ? { CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY: "1" }
      : {}),
  }
}

async function allocateKernelPorts(excludedPorts = []) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const kernel = 52_000 + Math.floor(Math.random() * 8_000)
    const ports = { kernel, mcp: kernel + 1, codex: kernel + 2, opencode: kernel + 3 }
    if (Object.values(ports).some((port) => excludedPorts.includes(port))) continue
    if ((await Promise.all(Object.values(ports).map(portIsAvailable))).every(Boolean)) return ports
  }
  throw new Error("could not allocate a local kernel port set")
}

async function portIsAvailable(port, host = "127.0.0.1") {
  return await new Promise((resolve) => {
    const server = net.createServer()
    server.once("error", () => resolve(false))
    server.listen(port, host, () => server.close(() => resolve(true)))
  })
}

function spawnLogged(label, binary, env, logDir) {
  const log = createWriteStream(path.join(logDir, `${label}.log`), { flags: "wx", mode: 0o600 })
  // A terminal Ctrl+C must reach the harness but not kill its kernel before
  // scoped Room and slice cleanup can complete over the local socket.
  const child = spawn(binary, [], { cwd: repoRoot, env, stdio: ["ignore", "pipe", "pipe"], detached: true })
  child.setupLabel = label
  child.setupLog = log
  child.spawnError = null
  child.once("error", (error) => { child.spawnError = error })
  child.stdout.pipe(log, { end: false })
  child.stderr.pipe(log, { end: false })
  child.once("exit", () => log.end())
  return child
}

async function waitForTcp(host, port, timeoutMs, child) {
  await waitFor(async () => {
    assertChildAlive(child)
    return await new Promise((resolve) => {
      const socket = net.connect({ host, port })
      socket.once("connect", () => { socket.destroy(); resolve(true) })
      socket.once("error", () => { socket.destroy(); resolve(false) })
    })
  }, timeoutMs, `relay did not listen on ${host}:${port}`)
}

async function readKernelInventory(client, requests) {
  const sessions = unwrap(await client.send(requests.listSessionsRequest()), "SessionsListed").sessions
  const slices = unwrap(await client.send(requests.listSlicesRequest()), "SlicesListed").slices
  return { sessions, slices }
}

async function connectKernel(LocalIpcClient, requests, kernelUrl, children) {
  let lastError = null
  const deadline = Date.now() + 60_000
  while (Date.now() < deadline) {
    for (const child of children) assertChildAlive(child)
    const client = new LocalIpcClient(kernelUrl)
    try {
      unwrap(await client.send(requests.listSlicesRequest()), "SlicesListed")
      return client
    } catch (error) {
      lastError = error
      await client.close?.().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`kernel did not accept local public requests: ${lastError?.message ?? String(lastError)}`)
}

async function waitForSliceRunning(client, requests, sliceId, children) {
  return await waitFor(async () => {
    children.forEach(assertChildAlive)
    const slice = unwrap(await client.send(requests.getSliceRequest(sliceId)), "Slice").slice
    if (slice.status === "failed" || slice.status === "error") {
      throw new Error(`local Docker slice entered ${slice.status}: ${slice.error ?? slice.message ?? "no diagnostic"}`)
    }
    return slice.status === "running" ? slice : false
  }, coldSliceProvisionTimeoutMs, `headed local Docker slice ${sliceId} did not become running`)
}

export async function startSliceAndWaitForRunning(client, requests, sliceId, children) {
  try {
    await client.send(requests.startSliceRequest(sliceId))
  } catch (error) {
    // Cold release builds may outlive the IPC response deadline while the
    // kernel's accepted start operation continues. Its slice state, not the
    // lost acknowledgement, determines whether setup can proceed.
    if (error?.code !== "request_timeout") throw error
    console.warn("[drill-c-setup] slice start acknowledgement timed out; checking slice state")
  }
  return await waitForSliceRunning(client, requests, sliceId, children)
}

async function waitForRoomReady(client, requests, sessionId, slice, daemonId, children) {
  return await waitFor(async () => {
    children.forEach(assertChildAlive)
    const environment = unwrap(await client.send(requests.getRoomEnvironmentStateRequest(sessionId)), "RoomEnvironmentState").environment
    if (environment.lifecycle === "failed" || environment.lifecycle === "error") {
      throw new Error(`Room environment entered ${environment.lifecycle}: ${environment.error ?? "no diagnostic"}`)
    }
    if (environment.lifecycle !== "ready" || !environment.focused_tab_id) return false
    assert.equal(environment.session_id, sessionId, "Room environment belongs to a different session")
    const binding = unwrap(await client.send(requests.getRoomEnvironmentSliceRequest(sessionId)), "RoomEnvironmentSlice").binding
    assertRoomSliceBinding(binding, { sessionId, slice, daemonId })
    const resourceInventory = unwrap(await client.send(
      requests.getRoomEnvironmentResourceInventoryRequest(sessionId, slice.id),
    ), "RoomEnvironmentResourceInventory").inventory
    assert.equal(resourceInventory?.session_id, sessionId, "Room inventory belongs to a different session")
    assert.equal(resourceInventory?.slice_id, slice.id, "Room inventory belongs to a different slice")
    assert.ok(Array.isArray(resourceInventory?.browser_ids), "kernel omitted Room browser identities")
    assert.ok(Array.isArray(resourceInventory?.profile_ids), "kernel omitted Room profile identities")
    assert.ok(Array.isArray(environment.actions), "kernel omitted Room snapshot actions")
    if (!resourceInventory.browser_ids.length || !resourceInventory.profile_ids.length) return false
    if (!environment.tabs?.some((tab) => tab.tab_id === environment.focused_tab_id)) return false
    const actionHistory = unwrap(await client.send(
      requests.listRoomEnvironmentActionHistoryRequest(sessionId),
    ), "RoomEnvironmentActionHistoryListed").page
    if (!Array.isArray(actionHistory?.actions)) throw new Error("kernel omitted Room action history at baseline")
    if (actionHistory.actions.length > 0 || environment.actions?.length > 0) {
      throw new Error("new Room action baseline already contains actions")
    }
    return { environment, resourceInventory, actionHistory }
  }, protocolClientRoomReadyTimeoutMs, "headed Room did not publish a ready browser, tab, and display inventory")
}

async function waitForCloudBootstrap(input) {
  let lastError = null
  const deadline = Date.now() + input.timeoutMs
  while (Date.now() < deadline) {
    input.children?.forEach(assertChildAlive)
    try {
      const dashboard = await input.cloud.dashboard()
      const target = dashboard.relayTargets?.find((item) => item.daemonId === input.daemonId)
      if (!target || target.machineId !== input.machineId) {
        await sleep(500)
        continue
      }
      const bootstrap = await input.cloud.bootstrap(input.daemonId, input.scopedThumbprint)
      assert.ok(bootstrap.relayUrl === input.relayUrl, "local Cloud bootstrap selected a different relay")
      if (!input.scopedThumbprint) {
        assert.ok(bootstrap.relayToken === input.relayToken,
          "local Cloud bootstrap token does not match the isolated local relay")
      }
      assert.ok(bootstrap.target?.daemonId === input.daemonId, "local Cloud bootstrap selected a different kernel")
      assert.ok(bootstrap.target?.machineId === input.machineId, "local Cloud bootstrap selected a different machine")
      const relayClient = new input.LocalIpcClient(bootstrap.relayUrl, {
        relayAuthToken: bootstrap.relayToken,
        targetDaemonAlias: bootstrap.target?.daemonAlias ?? input.daemonId,
        kernelPingIntervalMs: 60_000,
        kernelMaxMissedPongs: 10,
      })
      try {
        const sessions = unwrap(await relayClient.send(input.requests.listSessionsRequest()), "SessionsListed").sessions
        const transport = assertCloudRelayBootstrap({
          bootstrap,
          relayUrl: input.relayUrl,
          relayToken: input.relayToken,
          daemonId: input.daemonId,
          machineId: input.machineId,
          sessionId: input.sessionId,
          sessions,
          scopedThumbprint: input.scopedThumbprint,
        })
        return { ...transport, verifiedAt: new Date().toISOString(), cloudApiUrl: input.cloud.baseUrl }
      } finally {
        await relayClient.close?.().catch(() => {})
      }
    } catch (error) {
      lastError = error
      if (/local Cloud bootstrap (?:selected|token)|cannot see the setup Room/.test(String(error?.message))) throw error
      await sleep(500)
    }
  }
  throw new Error(`local Cloud did not expose the new Room over its product relay bootstrap: ${lastError?.message ?? "target not discovered"}`)
}

async function waitFor(observe, timeoutMs, message) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const value = await observe()
      if (value) return value
    } catch (error) {
      lastError = error
      if (error?.fatal === true) throw error
      if (/entered (?:failed|error)|different (?:slice|session|daemon|machine|worker kernel)|mismatch|baseline|exited before setup completed|failed to start/.test(error?.message ?? "")) throw error
    }
    await sleep(250)
  }
  throw new Error(`${message}${lastError ? `: ${lastError.message}` : ""}`)
}

function assertChildAlive(child) {
  if (child.spawnError) throw new Error(`${child.setupLabel} failed to start: ${child.spawnError.message}`)
  if (child.exitCode != null || child.signalCode != null) {
    throw new Error(`${child.setupLabel} exited before setup completed: code=${child.exitCode} signal=${child.signalCode}`)
  }
}

export async function cleanupOwnedResources({
  state,
  requests,
  children,
  removeWorkspaceFixture = removeRoomDirectDockerWorkspaceFixture,
}) {
  const errors = []
  let sessionCleanupVerified = !state.sessionCreateAttempted && !state.sessionId
  let sliceCleanupVerified = !state.sliceCreateAttempted && !state.sliceId
  if (state.client) {
    if (state.sessionId) {
      sessionCleanupVerified = true
      if (state.roomEnvironmentStartAttempted) {
        sessionCleanupVerified = await verifyCleanupAck({
          state,
          errors,
          label: "Room environment",
          request: requests.stopRoomEnvironmentRequest?.(state.sessionId),
          responseVariant: "RoomEnvironmentUpdated",
          recordId: (response) => response.environment?.session_id,
          expectedId: state.sessionId,
        }) && sessionCleanupVerified
      }
      sessionCleanupVerified = await verifyCleanupAck({
        state,
        errors,
        label: "session",
        request: requests.endSessionRequest?.(state.sessionId),
        responseVariant: "SessionEnded",
        recordId: (response) => response.session?.id,
        expectedId: state.sessionId,
      }) && sessionCleanupVerified
    } else if (state.sessionCreateAttempted) {
      errors.push("session creation was attempted but no unique Room identity was confirmed")
      sessionCleanupVerified = false
    }
    if (state.sliceId) {
      sliceCleanupVerified = await verifyCleanupAck({
        state,
        errors,
        label: "slice stop",
        request: requests.stopSliceRequest(state.sliceId),
        responseVariant: "SliceStopped",
        recordId: (response) => response.slice?.id,
        expectedId: state.sliceId,
      })
      sliceCleanupVerified = await verifyCleanupAck({
        state,
        errors,
        label: "slice deletion",
        request: requests.deleteSliceRequest(state.sliceId),
        responseVariant: "SliceDeleted",
        recordId: (response) => response.slice?.id,
        expectedId: state.sliceId,
      }) && sliceCleanupVerified
    } else if (state.sliceCreateAttempted) {
      errors.push("slice creation was attempted but no unique slice identity was confirmed")
      sliceCleanupVerified = false
    }
    await state.client.close?.().catch((error) => errors.push(`kernel client close failed: ${error.message}`))
  } else if (state.sessionCreateAttempted || state.sliceCreateAttempted) {
    if (state.sessionCreateAttempted) {
      errors.push("cannot verify Room cleanup because the kernel client is unavailable")
      sessionCleanupVerified = false
    }
    if (state.sliceCreateAttempted) {
      errors.push("cannot verify slice cleanup because the kernel client is unavailable")
      sliceCleanupVerified = false
    }
  }
  let childrenStopped = true
  for (const child of [...children].reverse()) {
    try { await terminateChild(child) } catch (error) {
      childrenStopped = false
      errors.push(`${child.setupLabel} stop failed: ${error.message}`)
    }
  }
  if (state.rootlessWorkspaceFixture) {
    const resourcesVerified = sessionCleanupVerified && sliceCleanupVerified && childrenStopped
    if (!resourcesVerified) {
      errors.push("refusing rootless workspace removal because Room or slice producers were not confirmed stopped and deleted")
    } else {
      try {
        await removeWorkspaceFixture(state.rootlessWorkspaceFixture)
      } catch (error) {
        errors.push(`rootless workspace removal failed: ${error.message}`)
      }
    }
  } else if (state.workspaceOwned && state.workspace) {
    try {
      await rm(state.workspace, { recursive: true, force: true })
    } catch (error) {
      errors.push(`workspace removal failed: ${error.message}`)
    }
  }
  if (state.presenceRecordPath) {
    try {
      await access(state.presenceRecordPath)
      errors.push(`kernel presence record remained after shutdown: ${state.presenceRecordPath}`)
    } catch (error) {
      if (error?.code !== "ENOENT") errors.push(`kernel presence check failed: ${error.message}`)
    }
  }
  return errors
}

async function verifyCleanupAck({ state, errors, label, request, responseVariant, recordId, expectedId }) {
  if (!request) {
    errors.push(`${label} cleanup request is unavailable`)
    return false
  }
  try {
    const response = unwrap(await state.client.send(request), responseVariant)
    assert.equal(recordId(response), expectedId, `${label} cleanup acknowledged a different resource`)
    return true
  } catch (error) {
    errors.push(`${label} cleanup failed: ${error.message}`)
    return false
  }
}

async function terminateChild(child) {
  if (!child || child.exitCode != null || child.signalCode != null) return
  child.kill("SIGTERM")
  const exited = await Promise.race([
    new Promise((resolve) => child.once("exit", () => resolve(true))),
    sleep(5_000).then(() => false),
  ])
  if (!exited && child.exitCode == null && child.signalCode == null) {
    child.kill("SIGKILL")
    await Promise.race([
      new Promise((resolve) => child.once("exit", resolve)),
      sleep(2_000),
    ])
  }
}

function unwrap(response, variant) {
  if (response && typeof response === "object" && variant in response) return response[variant]
  throw new Error(`kernel response omitted ${variant}`)
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error?.stack ?? String(error))
    process.exitCode = 1
  })
}
