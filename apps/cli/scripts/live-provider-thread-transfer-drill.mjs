#!/usr/bin/env node
import { spawn } from "node:child_process"
import { createWriteStream } from "node:fs"
import { access, mkdir, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  getProviderRunRequest,
} from "../dist/ipc-requests.js"
import {
  assertBinary,
  makeAvailablePorts,
  terminateChild,
} from "./lib/drill-runtime-helpers.mjs"
import { writeIsolatedKernelConfig } from "./lib/drill-kernel-storage.mjs"
import { sanitizeDrillMetadata } from "./lib/drill-secrets.mjs"
import {
  RELAY_ISSUER,
  RELAY_SECRET,
  cleanupSliceModeProviderCredentials,
  defaultLocalDockerSliceImage,
  kernelBinary,
  makeWorkerResumePorts,
  parseArgs,
  printHelp,
  providerThreadSliceBuildEnv,
  providerThreadSliceConfigLines,
  prepareIsolatedWorkerProviderEnv,
  prepareSliceModeProviderEnv,
  realProviderEnv,
  relayBinary,
  relayClaims,
  repoRoot,
  signRelayToken,
  spawnLogged,
  waitForLocalDaemon,
  waitForRelayTarget,
  workerResumeDaemonEnv,
} from "./lib/live-provider-thread-transfer-runtime.mjs"
import { resolveProviderThreadDrillPaths } from "./lib/provider-thread-drill-paths.mjs"
import {
  runLocalReloadScenario,
  runWorkerResumeScenario,
  selectWorkerKernel,
} from "./lib/live-provider-thread-transfer-local-worker-scenarios.mjs"
import {
  runLiveMigrateToSliceScenario,
  runSliceRestartScenario,
} from "./lib/live-provider-thread-transfer-slice-scenarios.mjs"

async function runWorkerResumeMatrix({ options, runtimeRoot, evidenceRoot, ports }) {
  await assertBinary(relayBinary, path.join(repoRoot, "apps/relay/Cargo.toml"), "chariox-relay")
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const homeKernelUrl = `ws://127.0.0.1:${ports.homeKernelPort}`
  const workerKernelUrl = `ws://127.0.0.1:${ports.workerKernelPort}`
  const realProvider = realProviderEnv()
  let isolatedHome = null
  let isolatedWorker = null
  if (options.workerState === "isolated") {
    try {
      isolatedHome = await prepareIsolatedWorkerProviderEnv(options.providers, "home")
      isolatedWorker = await prepareIsolatedWorkerProviderEnv(options.providers, "worker")
    } catch (error) {
      await Promise.all([
        isolatedHome?.secretRoot
          ? rm(isolatedHome.secretRoot, { recursive: true, force: true })
          : Promise.resolve(),
        isolatedWorker?.secretRoot
          ? rm(isolatedWorker.secretRoot, { recursive: true, force: true })
          : Promise.resolve(),
      ])
      throw error
    }
  }
  const homeProvider = isolatedHome?.providerEnv ?? realProvider
  const workerProvider = isolatedWorker?.providerEnv ?? realProvider
  const relayEnv = {
    ...process.env,
    CHARIOX_RELAY_HOST: "127.0.0.1",
    CHARIOX_RELAY_PORT: String(ports.relayPort),
    CHARIOX_RELAY_SCOPED_ISSUER: RELAY_ISSUER,
    CHARIOX_RELAY_SCOPED_HMAC_SECRET: RELAY_SECRET,
  }
  const homeDaemonId = `provider-thread-home-${process.pid}-${Date.now()}`
  const workerDaemonId = `provider-thread-worker-${process.pid}-${Date.now()}`
  const workerMachineId = `provider-thread-worker-machine-${process.pid}`
  const clientRelayToken = signRelayToken(relayClaims({
    subject: `provider-thread-client-${process.pid}-${Date.now()}`,
    subjectKind: "client",
    actions: ["client_connect", "client_metadata_read", "packet_route"],
  }))
  const homeRelayToken = signRelayToken(relayClaims({
    subject: homeDaemonId,
    subjectKind: "kernel",
    actions: ["daemon_register", "daemon_heartbeat", "peer_request", "peer_event", "client_metadata_read"],
  }))
  const workerRelayToken = signRelayToken(relayClaims({
    subject: workerDaemonId,
    subjectKind: "kernel",
    actions: ["daemon_register", "daemon_heartbeat", "peer_request", "peer_event", "client_metadata_read"],
  }))

  const homeEnv = workerResumeDaemonEnv({
    ports,
    root: runtimeRoot,
    relayToken: homeRelayToken,
    daemonId: homeDaemonId,
    daemonAlias: "home",
    machineId: `provider-thread-home-machine-${process.pid}`,
    machineAlias: "provider-thread-home",
    acceptRemoteLeases: false,
    socketName: "home-daemon.sock",
    kernelPort: ports.homeKernelPort,
    mcpPort: ports.homeMcpPort,
    openCodePort: ports.homeOpenCodePort,
    codexPort: ports.homeCodexPort,
    providerEnv: homeProvider,
  })
  const workerEnv = workerResumeDaemonEnv({
    ports,
    root: runtimeRoot,
    relayToken: workerRelayToken,
    daemonId: workerDaemonId,
    daemonAlias: "worker",
    machineId: workerMachineId,
    machineAlias: "provider-thread-worker",
    acceptRemoteLeases: true,
    socketName: "worker-daemon.sock",
    kernelPort: ports.workerKernelPort,
    mcpPort: ports.workerMcpPort,
    openCodePort: ports.workerOpenCodePort,
    codexPort: ports.workerCodexPort,
    providerEnv: workerProvider,
  })
  await Promise.all([
    writeIsolatedKernelConfig({
      xdgConfigHome: homeEnv.XDG_CONFIG_HOME,
      storageRoot: path.join(runtimeRoot, "home-kernel-storage"),
    }),
    writeIsolatedKernelConfig({
      xdgConfigHome: workerEnv.XDG_CONFIG_HOME,
      storageRoot: path.join(runtimeRoot, "worker-kernel-storage"),
    }),
  ])

  let relayChild = null
  let homeChild = null
  let workerChild = null
  const matrix = {
    goal: "provider-thread-transfer",
    drill: options.drill,
    run_id: path.basename(runtimeRoot),
    relay_url: relayUrl,
    home_kernel_url: homeKernelUrl,
    worker_kernel_url: workerKernelUrl,
    worker_machine_id: workerMachineId,
    worker_state: options.workerState,
    worker_provider_environment: isolatedWorker?.evidence ?? {
      mode: "shared",
      provider_data_shared: true,
      provider_cache_shared: true,
      provider_home_shared: true,
    },
    home_provider_environment: isolatedHome?.evidence ?? {
      mode: "shared",
      provider_data_shared: true,
      provider_cache_shared: true,
      provider_home_shared: true,
    },
    providers: options.providers,
    started_at_ms: Date.now(),
    results: [],
  }
  try {
    relayChild = spawnLogged(relayBinary, [], {
      cwd: repoRoot,
      env: relayEnv,
      stdoutPath: path.join(runtimeRoot, "relay.stdout.log"),
      stderrPath: path.join(runtimeRoot, "relay.stderr.log"),
    })
    homeChild = spawnLogged(kernelBinary, [], {
      cwd: repoRoot,
      env: homeEnv,
      stdoutPath: path.join(runtimeRoot, "home-kernel.stdout.log"),
      stderrPath: path.join(runtimeRoot, "home-kernel.stderr.log"),
    })
    workerChild = spawnLogged(kernelBinary, [], {
      cwd: repoRoot,
      env: workerEnv,
      stdoutPath: path.join(runtimeRoot, "worker-kernel.stdout.log"),
      stderrPath: path.join(runtimeRoot, "worker-kernel.stderr.log"),
    })

    await waitForLocalDaemon(homeKernelUrl, runtimeRoot, runtimeRoot)
    await waitForLocalDaemon(workerKernelUrl, runtimeRoot, runtimeRoot)
    await waitForRelayTarget(relayUrl, clientRelayToken, "home", Math.min(options.timeoutMs, 120_000), options.pollMs)
    await waitForRelayTarget(relayUrl, clientRelayToken, "worker", Math.min(options.timeoutMs, 120_000), options.pollMs)

    const client = new LocalIpcClient(homeKernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await client.send({ ConfigureRelay: { relay_url: relayUrl, relay_token: homeRelayToken } })
      for (const provider of options.providers) {
        const workerKernel = await selectWorkerKernel(
          client,
          workerMachineId,
          provider,
          Math.min(options.timeoutMs, 120_000),
          options.pollMs,
        )
        const result = await runWorkerResumeScenario({
          provider,
          root: runtimeRoot,
          kernelUrl: homeKernelUrl,
          historyDir: homeEnv.CHARIOX_SESSION_HISTORY_DIR,
          workerMachineId,
          workerKernelId: workerKernel.kernel_id,
          workerKernelUrl,
          sourceProviderEnv: homeProvider,
          destinationProviderEnv: workerProvider,
          options,
        })
        matrix.results.push(result)
        await writeFile(
          path.join(evidenceRoot, `${provider}-worker-resume-result.json`),
          `${JSON.stringify(sanitizeDrillMetadata(result), null, 2)}\n`,
          "utf8",
        )
        console.log(`${provider}: ${result.status}`)
        if (result.status !== "passed") {
          console.log(result.errors.join("\n"))
        }
      }
    } finally {
      await client.close().catch(() => {})
    }
  } finally {
    matrix.finished_at_ms = Date.now()
    matrix.passed = matrix.results.length > 0 && matrix.results.every((result) => result.status === "passed")
    await terminateChild(workerChild)
    await terminateChild(homeChild)
    await terminateChild(relayChild)
    await sleep(500)
    if (isolatedWorker?.secretRoot) {
      await rm(isolatedWorker.secretRoot, { recursive: true, force: true })
    }
    if (isolatedHome?.secretRoot) {
      await rm(isolatedHome.secretRoot, { recursive: true, force: true })
    }
    await sleep(250)
    if (isolatedWorker?.secretRoot) {
      await rm(isolatedWorker.secretRoot, { recursive: true, force: true })
    }
    if (isolatedHome?.secretRoot) {
      await rm(isolatedHome.secretRoot, { recursive: true, force: true })
    }
  }
  return matrix
}

async function pathIsMissing(target) {
  try {
    await access(target)
    return false
  } catch (error) {
    if (error?.code === "ENOENT") return true
    throw error
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  await assertBinary(kernelBinary, path.join(repoRoot, "apps/kernel/Cargo.toml"), "chariox-kernel")

  const runId = `${Date.now()}-${process.pid}`
  const { evidenceRoot, runtimeRoot } = resolveProviderThreadDrillPaths({
    homeDir: os.homedir(),
    runId,
  })
  await mkdir(evidenceRoot, { recursive: true })
  await mkdir(runtimeRoot, { recursive: true })
  let daemonChild = null
  let sliceModeProviderEnv = null
  let fatalError = null
  let matrix = {
    goal: "provider-thread-transfer",
    drill: options.drill,
    run_id: runId,
    providers: options.providers,
    started_at_ms: Date.now(),
    results: [],
    cleanup: {},
  }

  try {
    const ports = options.drill === "worker-resume"
      ? await makeWorkerResumePorts()
      : await makeAvailablePorts()
    const kernelUrl = options.kernel ?? `ws://127.0.0.1:${ports.kernelPort}`
    const historyDir = path.join(runtimeRoot, "history")
    const capabilityRoot = path.join(runtimeRoot, "capabilities")
    const sliceMode = options.drill === "slice-restart"
      || options.drill === "live-migrate-to-slice"
      || options.drill === "live-migrate-roundtrip-slice"
    const daemonHome = sliceMode
      ? path.join(runtimeRoot, "xdg-config", "chariox")
      : runtimeRoot
    matrix.kernel_url = kernelUrl
    await mkdir(historyDir, { recursive: true })
    await mkdir(capabilityRoot, { recursive: true })
    options.historyDir = historyDir

    if (options.drill === "worker-resume") {
      matrix = await runWorkerResumeMatrix({ options, runtimeRoot, evidenceRoot, ports })
      matrix.cleanup ??= {}
    } else {
      if (sliceMode) {
        const sliceXdgConfigHome = path.join(runtimeRoot, "xdg-config")
        const sliceXdgStateHome = path.join(runtimeRoot, "xdg-state")
        const sliceXdgDataHome = path.join(runtimeRoot, "xdg-data")
        const sliceXdgCacheHome = path.join(runtimeRoot, "xdg-cache")
        const sliceRoot = path.join(runtimeRoot, "slices")
        await mkdir(sliceXdgStateHome, { recursive: true })
        await mkdir(sliceXdgDataHome, { recursive: true })
        await mkdir(sliceXdgCacheHome, { recursive: true })
        await mkdir(sliceRoot, { recursive: true })
        await writeIsolatedKernelConfig({
          xdgConfigHome: sliceXdgConfigHome,
          storageRoot: path.join(runtimeRoot, "home-kernel-storage"),
          extraToml: providerThreadSliceConfigLines({
            sliceRoot,
            image: defaultLocalDockerSliceImage,
            buildImage: options.sliceBuildImage,
          }),
        })
        const sliceBuildEnv = providerThreadSliceBuildEnv()
        console.log(`slice-restart: provisioner image policy ${options.sliceBuildImage}`)
        matrix.slice_image = defaultLocalDockerSliceImage
        matrix.slice_build_image = options.sliceBuildImage
        matrix.slice_image_build = {
          image: defaultLocalDockerSliceImage,
          delegated_to: "slice-provisioner",
          cargo_build_profile: sliceBuildEnv.CHARIOX_SLICE_RUNTIME_BUILD_PROFILE,
          cargo_opt_level: sliceBuildEnv.CHARIOX_SLICE_CARGO_PROFILE_RELEASE_OPT_LEVEL,
        }
        sliceModeProviderEnv = await prepareSliceModeProviderEnv(runtimeRoot, options.providers)
        options.providerStateSourceEnv = sliceModeProviderEnv
      }

      if (options.spawnDaemon) {
        const stdout = createWriteStream(path.join(runtimeRoot, "kernel.stdout.log"), { flags: "a" })
        const stderr = createWriteStream(path.join(runtimeRoot, "kernel.stderr.log"), { flags: "a" })
        const daemonEnv = {
          ...process.env,
          ...(sliceModeProviderEnv ? {
            ...sliceModeProviderEnv,
            CHARIOX_LOG_DIR: path.join(runtimeRoot, "logs"),
          } : {}),
          CHARIOX_HOME: daemonHome,
          CHARIOX_KERNEL_PORT: String(ports.kernelPort),
          CHARIOX_MCP_PORT: String(ports.mcpPort),
          CHARIOX_OPENCODE_PORT: String(ports.openCodePort),
          CHARIOX_CODEX_PORT: String(ports.codexPort),
          CHARIOX_DAEMON_ID: `provider-thread-transfer-${runId}`,
          CHARIOX_DAEMON_SOCKET: path.join(runtimeRoot, "daemon.sock"),
          CHARIOX_SESSION_HISTORY_DIR: historyDir,
          CHARIOX_CAPABILITY_ISOLATION_ROOT: capabilityRoot,
          CHARIOX_PROVIDER_RUNTIME_INIT_DELAY_MS: "250",
        }
        if (sliceMode) {
          delete daemonEnv.CHARIOX_RELAY_URL
          delete daemonEnv.CHARIOX_RELAY_TOKEN
          delete daemonEnv.CHARIOX_CLOUD_RELAY_URL
          delete daemonEnv.CHARIOX_CLOUD_RELAY_TOKEN
        }
        daemonChild = spawn(kernelBinary, [], {
          cwd: repoRoot,
          env: daemonEnv,
          stdio: ["ignore", "pipe", "pipe"],
        })
        daemonChild.stdout?.pipe(stdout)
        daemonChild.stderr?.pipe(stderr)
        await waitForLocalDaemon(kernelUrl, runtimeRoot, runtimeRoot)
      }

      const runScenario = options.drill === "slice-restart"
        ? runSliceRestartScenario
        : options.drill === "live-migrate-to-slice" || options.drill === "live-migrate-roundtrip-slice"
          ? runLiveMigrateToSliceScenario
          : runLocalReloadScenario
      for (const provider of options.providers) {
        const result = await runScenario({ provider, root: runtimeRoot, kernelUrl, options })
        matrix.results.push(result)
        await writeFile(
          path.join(evidenceRoot, `${provider}-${options.drill}-result.json`),
          `${JSON.stringify(sanitizeDrillMetadata(result), null, 2)}\n`,
          "utf8",
        )
        console.log(`${provider}: ${result.status}`)
        if (result.status !== "passed") {
          console.log(result.errors.join("\n"))
        }
      }
    }
  } catch (error) {
    fatalError = error
    matrix.fatal_error = error.stack ?? error.message ?? String(error)
  } finally {
    await terminateChild(daemonChild)
    const claudeSecretRoot = sliceModeProviderEnv?.CHARIOX_PROVIDER_THREAD_CLAUDE_SECRET_ROOT ?? null
    try {
      await cleanupSliceModeProviderCredentials(sliceModeProviderEnv)
      matrix.cleanup.provider_credentials_removed = !claudeSecretRoot
        || await pathIsMissing(claudeSecretRoot)
    } catch (error) {
      fatalError ??= error
      matrix.cleanup.provider_credentials_removed = false
      matrix.cleanup.provider_credentials_cleanup_error = error.message ?? String(error)
    }
    try {
      await rm(runtimeRoot, { recursive: true, force: true })
      matrix.cleanup.runtime_root_removed = await pathIsMissing(runtimeRoot)
    } catch (error) {
      fatalError ??= error
      matrix.cleanup.runtime_root_removed = false
      matrix.cleanup.runtime_root_cleanup_error = error.message ?? String(error)
    }
    matrix.finished_at_ms = Date.now()
    matrix.passed = !fatalError
      && matrix.results.length > 0
      && matrix.results.every((result) => result.status === "passed")
      && matrix.cleanup.runtime_root_removed
      && matrix.cleanup.provider_credentials_removed
    await writeFile(
      path.join(evidenceRoot, "matrix.json"),
      `${JSON.stringify(sanitizeDrillMetadata(matrix), null, 2)}\n`,
      "utf8",
    )
  }

  console.log(`provider thread transfer drill evidence: ${evidenceRoot}`)
  if (fatalError) throw fatalError
  if (!matrix.passed) {
    throw new Error(`provider thread transfer drill failed; see ${path.join(evidenceRoot, "matrix.json")}`)
  }
}

main().catch((error) => {
  console.error(error.stack ?? error.message ?? String(error))
  process.exit(1)
})
