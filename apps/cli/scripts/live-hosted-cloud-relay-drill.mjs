#!/usr/bin/env node
import { mkdir, mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import {
  runHostedSecondKernelAssertions,
  runHostedTokenRotationAssertions,
  withHostedKernelIsolation,
} from "./lib/hosted-cloud-kernel-scenarios.mjs"
import { runHostedMultiUserAssertions } from "./lib/hosted-cloud-multi-user-scenarios.mjs"
import { runHostedRemoteCliAssertions, runHostedRemoteCliPairingAssertions } from "./lib/hosted-cloud-remote-cli-scenarios.mjs"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import { withDevStubProviderInventory } from "./lib/drill-runtime-helpers.mjs"
import {
  allowDevStubProvider,
  assert,
  buildKernelIfNeeded,
  cleanupHostedCloudIdentity,
  closeClient,
  createHostedCommandDeps,
  createCloudDrillClient,
  loadCloudRelayDrillModules,
  waitForCloudRelayTarget,
  connectSessionScopedCloudClient,
  cleanupHostedCloudTerminal,
  expectReject,
  installSendRetry,
  log,
  makePorts,
  makeWorkerPorts,
  manualCloudDeviceLogin,
  resolveExecutable,
  run,
  runSsh,
  sendWithRetry,
  shellQuote,
  spawnProcess,
  sshArgs,
  terminateChild,
  unwrap,
  waitForCompletion,
  waitForHistoryText,
  waitForLocalDaemon,
  waitForRemoteMachine,
  waitForSession,
} from "./lib/live-hosted-cloud-relay-drill-helpers.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const apiUrl = (process.env.CHARIOX_CLOUD_HOSTED_API_URL ?? "https://staging.chariox.com").replace(/\/$/, "")
const pollTimeoutMs = Number(process.env.CHARIOX_CLOUD_HOSTED_POLL_TIMEOUT_MS ?? 10 * 60 * 1000)
const runMultiUser = process.env.CHARIOX_CLOUD_HOSTED_MULTI_USER === "1"
const runSecondKernel = process.env.CHARIOX_CLOUD_HOSTED_SECOND_KERNEL === "1"
const runRemoteCli = process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI === "1"
const runRemoteCliPairing = process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_PAIRING === "1"
const runTokenRotation = process.env.CHARIOX_CLOUD_HOSTED_TOKEN_ROTATION === "1"
const runWorkspaceLiveSync = process.env.CHARIOX_CLOUD_HOSTED_WORKSPACE_LIVE_SYNC === "1"
const runTrackedWorkspaceLiveSync = process.env.CHARIOX_CLOUD_HOSTED_TRACKED_WORKSPACE_LIVE_SYNC === "1"
const trackedWorkspaceLiveSyncProvider = process.env.CHARIOX_CLOUD_HOSTED_TRACKED_WORKSPACE_LIVE_SYNC_PROVIDER ?? "codex"
const trackedWorkspaceLiveSyncModel = process.env.CHARIOX_CLOUD_HOSTED_TRACKED_WORKSPACE_LIVE_SYNC_MODEL ?? "gpt-5.2"
const remoteCliPairingProvider = process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_PROVIDER ?? "codex"
const remoteCliPairingModel = process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_MODEL ?? "gpt-5.2-codex"
const remoteCliPairingEffort = process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_EFFORT ?? "low"
const remoteCliHost = process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_HOST ?? "root@195.201.123.115"
const remoteCliKey = process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_KEY ?? path.join(os.homedir(), ".ssh/chariox_hetzner_staging")
const remoteCliRepo = process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_REPO ?? "/opt/chariox-cli-drill"
const devAuthSecret = process.env.CHARIOX_CLOUD_DEV_AUTH_SECRET ?? ""

if ((runWorkspaceLiveSync || runTrackedWorkspaceLiveSync) && !runSecondKernel) {
  throw new Error("hosted Workspace Live Sync drills require CHARIOX_CLOUD_HOSTED_SECOND_KERNEL=1")
}

async function main() {
  if (process.argv.includes("--help") || process.argv.includes("-h")) {
    console.log("Usage: node apps/cli/scripts/live-hosted-cloud-relay-drill.mjs\n\nUse --check to validate built modules without live services.\nHosted scenario selection is controlled by CHARIOX_CLOUD_HOSTED_* environment variables.")
    return
  }
  if (process.argv.includes("--check")) {
    await loadCloudRelayDrillModules()
    log("module-composition-ok")
    return
  }
  const ports = await makePorts()
  const runId = `hosted-cloud-relay-${process.pid}-${Date.now()}`
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-hosted-relay-"))
  const evidenceDir = path.join(process.env.CHARIOX_DRILL_EVIDENCE_DIR ?? path.join(os.homedir(), ".codex/evidence/live-hosted-cloud-relay"), runId)
  const workspace = path.join(rootDir, "workspace")
  const homeDir = path.join(rootDir, "home")
  const charioxHome = path.join(homeDir, ".chariox")
  const homeCapabilityRoot = path.join(rootDir, "home-capabilities")
  const homeHistoryDir = path.join(rootDir, "session-history")
  const xdgConfigHome = path.join(homeDir, ".config")
  const xdgStateHome = path.join(homeDir, ".local", "state")
  const xdgRuntimeDir = path.join(homeDir, "run")
  const daemonId = `hosted-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `hosted-home-${process.pid}`
  let clientId = null
  const ownerAccountSlug = `hosted-owner-${process.pid}-${Date.now()}`

  await prepareDrillArtifacts(evidenceDir)
  await mkdir(workspace, { recursive: true })
  await mkdir(charioxHome, { recursive: true })
  await mkdir(xdgConfigHome, { recursive: true })
  await mkdir(xdgStateHome, { recursive: true })
  await mkdir(xdgRuntimeDir, { recursive: true })

  let daemon = null
  let localClient = null
  let remoteClient = null
  let requests = null
  let passed = false
  let failure = null
  let createdSessionId = null
  let terminal = null
  let ownerProfile = null
  let modules = null
  const profileRef = { current: null }

  try {
    log("build-cli")
    const cliBuild = await run("pnpm", ["run", "build"], { cwd: cliRoot, env: process.env })
    if (cliBuild.code !== 0) {
      throw new Error(`chariox cli build failed\n${cliBuild.stdout}\n${cliBuild.stderr}`)
    }

    log("build-kernel")
    const kernelPath = await buildKernelIfNeeded()
    const python = await resolveExecutable(process.env.PYTHON ?? "python3")

    modules = await loadCloudRelayDrillModules()
    const { LocalIpcClient } = modules
    requests = modules.requests
    terminal = createCloudDrillClient(modules, rootDir, "owner-terminal")
    clientId = `cli:${terminal.identity.publicKeyThumbprint}`

    const daemonEnv = withDevStubProviderInventory(withHostedKernelIsolation({
      ...process.env,
      CHARIOX_KERNEL_PORT: String(ports.kernelPort),
      CHARIOX_MCP_PORT: String(ports.mcpPort),
      CHARIOX_OPENCODE_PORT: String(ports.opencodePort),
      CHARIOX_CODEX_PORT: String(ports.codexPort),
      CHARIOX_DAEMON_ID: daemonId,
      CHARIOX_DAEMON_ALIAS: daemonAlias,
      CHARIOX_MACHINE_ID: daemonId,
      CHARIOX_MACHINE_ALIAS: daemonAlias,
      CHARIOX_DAEMON_SOCKET: path.join(rootDir, "daemon.sock"),
      CHARIOX_SESSION_HISTORY_DIR: homeHistoryDir,
      CHARIOX_CAPABILITY_ISOLATION_ROOT: homeCapabilityRoot,
    }, {
      homeDir,
      charioxHome,
      xdgConfigHome,
      xdgStateHome,
      xdgRuntimeDir,
    }))

    log("start-kernel")
    daemon = spawnProcess(kernelPath, [], { cwd: repoRoot, env: daemonEnv, name: "kernel" })

    const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}/kernel`
    await waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace, daemonEnv)
    localClient = new LocalIpcClient(kernelUrl, { localAuthEnvironment: daemonEnv })

    const notices = []
    const handlers = modules.createCommandActionHandlers(createHostedCommandDeps({
      workspace,
      clientId,
      localClient,
      requests,
      profileRef,
      notices,
      ownerAccountSlug,
      terminal,
      modules,
    }))

    // MP-08 / MP-10 / MP-11: enroll this kernel once, then authenticate the
    // owner terminal independently with the CLIENT flow/private product store.
    log("command-cloud-link", { apiUrl })
    await handlers.handleCloudCommand({kind: "cloud", raw: "/cloud link", args: ["link"]})
    assert(profileRef.current?.kernelEnrolled && profileRef.current.kernelId === daemonId, "hosted link should enroll the current kernel")
    assert(!profileRef.current.cloudSessionToken && !profileRef.current.kernelCredential && !profileRef.current.machineCredential, "kernel profile must not export credentials")

    log("command-cloud-login", { apiUrl })
    await handlers.handleCloudCommand({kind: "cloud", raw: "/cloud login", args: ["login"]})
    ownerProfile = await terminal.client.profile()
    assert(ownerProfile?.clientId === clientId && ownerProfile.accountId === profileRef.current.accountId, "hosted terminal should use separate CLIENT authority in the enrolled account")
    assert(!ownerProfile.machineId && !ownerProfile.kernelId, "terminal login must not enroll a kernel")

    log("relay-target-probe", { daemonId })
    await waitForCloudRelayTarget(terminal.client, {daemonId, status: "ONLINE"})
    const clientRelay = {relayUrl: ownerProfile.relayUrl}
    remoteClient = installSendRetry(await terminal.client.connect(daemonId), "owner-relay")

    log("remote-session-create")
    const created = unwrap(
      await sendWithRetry(remoteClient, requests.createSessionRequest(workspace, workspace), "remote-session-create"),
      "SessionCreated",
    )
    createdSessionId = created.session.id
    assert(created.session?.id, "remote cloud session creation should return a session", created)

    const listed = unwrap(
      await sendWithRetry(remoteClient, requests.listSessionsRequest(), "remote-session-list"),
      "SessionsListed",
    )
    assert(
      Array.isArray(listed.sessions) && listed.sessions.some((session) => session.id === created.session.id),
      "remote cloud client should list the created session",
      listed,
    )

    if (runRemoteCli) {
      await runHostedRemoteCliAssertions({
        apiUrl,
        ownerAccountSlug,
        ownerAccountId: ownerProfile.accountId,
        requests,
        homeClient: localClient,
        verificationClient: remoteClient,
        repoRoot,
        remoteCliRepo,
        remoteCliHost,
        log,
        assert,
        unwrap,
        shellQuote,
        sshArgs,
        runSsh,
        spawnProcess,
        terminateChild,
        allowDevStubProvider,
      })
    }

    if (runRemoteCliPairing) {
      await runHostedRemoteCliPairingAssertions({
        apiUrl,
        ownerAccountSlug,
        ownerAccountId: ownerProfile.accountId,
        requests,
        homeClient: localClient,
        verificationClient: remoteClient,
        workspace,
        kernelUrl,
        cliRoot,
        repoRoot,
        remoteCliRepo,
        remoteCliHost,
        remoteCliPairingProvider,
        remoteCliPairingModel,
        remoteCliPairingEffort,
        pollTimeoutMs,
        log,
        assert,
        unwrap,
        shellQuote,
        sshArgs,
        runSsh,
        spawnProcess,
        terminateChild,
        waitForSession,
        waitForHistoryText,
      })
    }

    if (runTokenRotation) {
      await runHostedTokenRotationAssertions({
        requests,
        homeClient: localClient,
        verificationClient: remoteClient,
        sessionId: created.session.id,
        log,
        assert,
        unwrap,
      })
    }

    if (runSecondKernel) {
      await runHostedSecondKernelAssertions({
        LocalIpcClient,
        requests,
        kernelPath,
        rootDir,
        workspace,
        session: created.session,
        kernelUrl,
        homeHistoryDir,
        python,
        homeCapabilityRoot,
        homeOnlyMcpPort: ports.homeOnlyMcpPort,
        collabExtensions: runMultiUser,
        workspaceLiveSync: runWorkspaceLiveSync,
        trackedWorkspaceLiveSync: runTrackedWorkspaceLiveSync,
        trackedWorkspaceLiveSyncProvider,
        trackedWorkspaceLiveSyncModel,
        homeClient: localClient,
        ownerProfile,
        ownerTerminal: terminal,
        modules,
        stateRoot: rootDir,
        homeDaemonId: daemonId,
        connectSessionScopedCloudClient,
        cleanupHostedCloudTerminal,
        apiUrl,
        repoRoot,
        pollTimeoutMs,
        log,
        assert,
        unwrap,
        makeWorkerPorts,
        manualCloudDeviceLogin,
        installSendRetry,
        expectReject,
        waitForLocalDaemon,
        allowDevStubProvider,
        waitForCloudRelayTarget,
        waitForRemoteMachine,
        waitForCompletion,
        closeClient,
        terminateChild,
        spawnProcess,
        cleanupHostedCloudIdentity,
      })
    }

    if (runMultiUser) {
      await runHostedMultiUserAssertions({
        requests,
        ownerRemoteClient: remoteClient,
        ownerProfile,
        ownerTerminal: terminal,
        modules,
        stateRoot: rootDir,
        homeDaemonId: daemonId,
        connectSessionScopedCloudClient,
        cleanupHostedCloudTerminal,
        workspace,
        session: created.session,
        log,
        assert,
        unwrap,
        manualCloudDeviceLogin,
        installSendRetry,
        expectReject,
      })
    } else {
      log("multi-user-skipped", {
        reason: devAuthSecret
          ? "set CHARIOX_CLOUD_HOSTED_MULTI_USER=1"
          : "set CHARIOX_CLOUD_HOSTED_MULTI_USER=1 and approve owner, peer, and third browser logins, or set CHARIOX_CLOUD_DEV_AUTH_SECRET",
      })
    }

    log("pass", {
      apiUrl,
      relayUrl: clientRelay.relayUrl,
      accountSlug: ownerProfile.accountSlug,
      sessionId: created.session.id,
      multiUser: runMultiUser,
      remoteCli: runRemoteCli,
      remoteCliPairing: runRemoteCliPairing,
      secondKernel: runSecondKernel,
      tokenRotation: runTokenRotation,
      workspaceLiveSync: runWorkspaceLiveSync,
      trackedWorkspaceLiveSync: runTrackedWorkspaceLiveSync,
    })
    passed = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    const cleanupErrors = []
    if (createdSessionId && localClient && requests) {
      await localClient.send(requests.endSessionRequest(createdSessionId)).catch((error) => {
        cleanupErrors.push(error)
        log("cloud-session-runtime-cleanup-failed", {
          error: error instanceof Error ? error.message : String(error),
        })
      })
    }
    if (localClient && profileRef.current?.kernelEnrolled) {
      await modules.relayApi.logoutCloudRelay(localClient).catch(error => {
        cleanupErrors.push(error)
        log("cloud-kernel-unlink-failed", {error: error instanceof Error ? error.message : String(error)})
      })
    }
    await closeClient(remoteClient, "remote")
    await closeClient(localClient, "local")
    await terminateChild(daemon)
    if (terminal && await terminal.client.profile()) {
      await cleanupHostedCloudTerminal(terminal, {
        machineIds: [profileRef.current?.machineId],
        reason: "hosted Cloud relay drill cleanup",
      }).catch(error => {
        cleanupErrors.push(error)
        log("cloud-identity-cleanup-failed", {error: error instanceof Error ? error.message : String(error)})
      })
    }
    terminal?.client.stop()
    await rm(rootDir, {recursive: true, force: true}).catch(error => cleanupErrors.push(error))
    const cleanupFailure = cleanupErrors.length > 0
      ? new AggregateError(cleanupErrors, "hosted Cloud relay drill cleanup failed")
      : null
    if (cleanupFailure && !failure) {
      passed = false
      failure = cleanupFailure
    }
    await finalizeDrillArtifacts({
      rootDir: evidenceDir,
      passed,
      failure,
      log,
      metadata: {
        drill: "hosted-cloud-relay",
        apiUrl,
        multiUser: runMultiUser,
        remoteCli: runRemoteCli,
        remoteCliPairing: runRemoteCliPairing,
        secondKernel: runSecondKernel,
        tokenRotation: runTokenRotation,
        workspaceLiveSync: runWorkspaceLiveSync,
        trackedWorkspaceLiveSync: runTrackedWorkspaceLiveSync,
      },
    })
    if (cleanupFailure && failure === cleanupFailure) {
      throw cleanupFailure
    }
  }
}

main().then(() => {
  process.exit(0)
}).catch((error) => {
  console.error(error)
  process.exitCode = 1
})
