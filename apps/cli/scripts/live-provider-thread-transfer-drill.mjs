#!/usr/bin/env node
import { spawn } from "node:child_process"
import { createWriteStream } from "node:fs"
import { mkdir, rm, writeFile } from "node:fs/promises"
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
import {
  RELAY_ISSUER,
  RELAY_SECRET,
  artifactsRoot,
  defaultLocalDockerSliceImage,
  kernelBinary,
  makeWorkerResumePorts,
  parseArgs,
  printHelp,
  prebuildLocalDockerSliceImageIfNeeded,
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
import {
  runLocalReloadScenario,
  runWorkerResumeScenario,
  selectWorkerKernel,
} from "./lib/live-provider-thread-transfer-local-worker-scenarios.mjs"
import {
  runLiveMigrateToSliceScenario,
  runSliceRestartScenario,
} from "./lib/live-provider-thread-transfer-slice-scenarios.mjs"

async function runWorkerResumeMatrix({ options, root, ports }) {
  await assertBinary(relayBinary, path.join(repoRoot, "apps/relay/Cargo.toml"), "arroba-relay")
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
    ARROBA_RELAY_HOST: "127.0.0.1",
    ARROBA_RELAY_PORT: String(ports.relayPort),
    ARROBA_RELAY_SCOPED_ISSUER: RELAY_ISSUER,
    ARROBA_RELAY_SCOPED_HMAC_SECRET: RELAY_SECRET,
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
    root,
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
    root,
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
      storageRoot: path.join(root, "home-kernel-storage"),
    }),
    writeIsolatedKernelConfig({
      xdgConfigHome: workerEnv.XDG_CONFIG_HOME,
      storageRoot: path.join(root, "worker-kernel-storage"),
    }),
  ])

  let relayChild = null
  let homeChild = null
  let workerChild = null
  const matrix = {
    goal: "provider-thread-transfer",
    drill: options.drill,
    run_id: path.basename(root),
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
      stdoutPath: path.join(root, "relay.stdout.log"),
      stderrPath: path.join(root, "relay.stderr.log"),
    })
    homeChild = spawnLogged(kernelBinary, [], {
      cwd: repoRoot,
      env: homeEnv,
      stdoutPath: path.join(root, "home-kernel.stdout.log"),
      stderrPath: path.join(root, "home-kernel.stderr.log"),
    })
    workerChild = spawnLogged(kernelBinary, [], {
      cwd: repoRoot,
      env: workerEnv,
      stdoutPath: path.join(root, "worker-kernel.stdout.log"),
      stderrPath: path.join(root, "worker-kernel.stderr.log"),
    })

    await waitForLocalDaemon(homeKernelUrl, root, root)
    await waitForLocalDaemon(workerKernelUrl, root, root)
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
          root,
          kernelUrl: homeKernelUrl,
          historyDir: homeEnv.ARROBA_SESSION_HISTORY_DIR,
          workerMachineId,
          workerKernelId: workerKernel.kernel_id,
          workerKernelUrl,
          sourceProviderEnv: homeProvider,
          destinationProviderEnv: workerProvider,
          options,
        })
        matrix.results.push(result)
        await writeFile(path.join(root, `${provider}-worker-resume-result.json`), `${JSON.stringify(result, null, 2)}\n`, "utf8")
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
    await writeFile(path.join(root, "matrix.json"), `${JSON.stringify(matrix, null, 2)}\n`, "utf8")
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

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  await assertBinary(kernelBinary, path.join(repoRoot, "apps/kernel/Cargo.toml"), "arroba-kernel")

  const runId = `${Date.now()}-${process.pid}`
  const root = path.join(artifactsRoot, runId)
  await mkdir(root, { recursive: true })
  const ports = options.drill === "worker-resume"
    ? await makeWorkerResumePorts()
    : await makeAvailablePorts()
  const kernelUrl = options.kernel ?? `ws://127.0.0.1:${ports.kernelPort}`
  const historyDir = path.join(root, "history")
  const capabilityRoot = path.join(root, "capabilities")
  const sliceMode = options.drill === "slice-restart"
    || options.drill === "live-migrate-to-slice"
    || options.drill === "live-migrate-roundtrip-slice"
  const sliceXdgConfigHome = path.join(root, "xdg-config")
  const sliceXdgStateHome = path.join(root, "xdg-state")
  const sliceXdgDataHome = path.join(root, "xdg-data")
  const sliceXdgCacheHome = path.join(root, "xdg-cache")
  const sliceRoot = path.join(root, "slices")
  await mkdir(historyDir, { recursive: true })
  await mkdir(capabilityRoot, { recursive: true })
  options.historyDir = historyDir
  let sliceImageBuild = null
  if (sliceMode) {
    await mkdir(sliceXdgStateHome, { recursive: true })
    await mkdir(sliceXdgDataHome, { recursive: true })
    await mkdir(sliceXdgCacheHome, { recursive: true })
    await mkdir(sliceRoot, { recursive: true })
    await writeIsolatedKernelConfig({
      xdgConfigHome: sliceXdgConfigHome,
      storageRoot: path.join(root, "home-kernel-storage"),
      extraToml: [
        "[slices]",
        `root = ${JSON.stringify(sliceRoot)}`,
        "",
        "[slices.linux]",
        `docker_image = ${JSON.stringify(defaultLocalDockerSliceImage)}`,
        `build_image = ${JSON.stringify(options.sliceBuildImage === "always" ? "auto" : options.sliceBuildImage)}`,
      ],
    })
    console.log(`slice-restart: prebuild image policy ${options.sliceBuildImage}`)
    sliceImageBuild = await prebuildLocalDockerSliceImageIfNeeded(root, options.sliceBuildImage, options.timeoutMs)
  }
  const sliceModeProviderEnv = sliceMode
    ? await prepareSliceModeProviderEnv(root, options.providers)
    : null
  if (sliceModeProviderEnv) {
    options.providerStateSourceEnv = sliceModeProviderEnv
  }

  if (options.drill === "worker-resume") {
    const matrix = await runWorkerResumeMatrix({ options, root, ports })
    console.log(`provider thread transfer drill artifacts: ${root}`)
    if (!matrix.passed) {
      throw new Error(`provider thread transfer drill failed; see ${path.join(root, "matrix.json")}`)
    }
    if (options.cleanupOnSuccess) {
      await rm(root, { recursive: true, force: true })
    }
    return
  }

  let daemonChild = null
  const matrix = {
    goal: "provider-thread-transfer",
    drill: options.drill,
    run_id: runId,
    kernel_url: kernelUrl,
    providers: options.providers,
    ...(sliceMode ? {
      slice_image: defaultLocalDockerSliceImage,
      slice_build_image: options.sliceBuildImage,
      slice_root: sliceRoot,
      slice_image_build: sliceImageBuild,
    } : {}),
    started_at_ms: Date.now(),
    results: [],
  }

  try {
    if (options.spawnDaemon) {
      const stdout = createWriteStream(path.join(root, "kernel.stdout.log"), { flags: "a" })
      const stderr = createWriteStream(path.join(root, "kernel.stderr.log"), { flags: "a" })
      const daemonEnv = {
        ...process.env,
        ...(sliceModeProviderEnv ? {
          ...sliceModeProviderEnv,
          ARROBA_LOG_DIR: path.join(root, "logs"),
        } : {}),
        ARROBA_KERNEL_PORT: String(ports.kernelPort),
        ARROBA_MCP_PORT: String(ports.mcpPort),
        ARROBA_OPENCODE_PORT: String(ports.openCodePort),
        ARROBA_CODEX_PORT: String(ports.codexPort),
        ARROBA_DAEMON_ID: `provider-thread-transfer-${runId}`,
        ARROBA_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        ARROBA_SESSION_HISTORY_DIR: historyDir,
        ARROBA_CAPABILITY_ISOLATION_ROOT: capabilityRoot,
        ARROBA_PROVIDER_RUNTIME_INIT_DELAY_MS: "250",
      }
      if (sliceMode) {
        delete daemonEnv.ARROBA_RELAY_URL
        delete daemonEnv.ARROBA_RELAY_TOKEN
        delete daemonEnv.ARROBA_CLOUD_RELAY_URL
        delete daemonEnv.ARROBA_CLOUD_RELAY_TOKEN
      }
      daemonChild = spawn(kernelBinary, [], {
        cwd: repoRoot,
        env: daemonEnv,
        stdio: ["ignore", "pipe", "pipe"],
      })
      daemonChild.stdout?.pipe(stdout)
      daemonChild.stderr?.pipe(stderr)
      await waitForLocalDaemon(kernelUrl, root, root)
    }

    const runScenario = options.drill === "slice-restart"
      ? runSliceRestartScenario
      : options.drill === "live-migrate-to-slice" || options.drill === "live-migrate-roundtrip-slice"
        ? runLiveMigrateToSliceScenario
        : runLocalReloadScenario
    for (const provider of options.providers) {
      const result = await runScenario({ provider, root, kernelUrl, options })
      matrix.results.push(result)
      await writeFile(path.join(root, `${provider}-${options.drill}-result.json`), `${JSON.stringify(result, null, 2)}\n`, "utf8")
      console.log(`${provider}: ${result.status}`)
      if (result.status !== "passed") {
        console.log(result.errors.join("\n"))
      }
    }
  } finally {
    matrix.finished_at_ms = Date.now()
    matrix.passed = matrix.results.length > 0 && matrix.results.every((result) => result.status === "passed")
    await writeFile(path.join(root, "matrix.json"), `${JSON.stringify(matrix, null, 2)}\n`, "utf8")
    await terminateChild(daemonChild)
    if (sliceModeProviderEnv?.ARROBA_PROVIDER_THREAD_CLAUDE_SECRET_ROOT) {
      await rm(sliceModeProviderEnv.ARROBA_PROVIDER_THREAD_CLAUDE_SECRET_ROOT, {
        recursive: true,
        force: true,
      })
    }
  }

  console.log(`provider thread transfer drill artifacts: ${root}`)
  if (!matrix.passed) {
    throw new Error(`provider thread transfer drill failed; see ${path.join(root, "matrix.json")}`)
  }
  if (options.cleanupOnSuccess) {
    await rm(root, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(error.stack ?? error.message ?? String(error))
  process.exit(1)
})
