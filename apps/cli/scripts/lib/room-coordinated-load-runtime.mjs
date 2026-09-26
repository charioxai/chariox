import { spawn } from "node:child_process"
import { randomUUID } from "node:crypto"
import { access, mkdir, rm, stat } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { performance } from "node:perf_hooks"
import { pathToFileURL } from "node:url"

import { roomTuiPtyInvocation } from "./room-tui-pty.mjs"
import { assertPreparedIdentities, COORDINATED_LOAD_VIEWERS } from "./room-coordinated-load-runner.mjs"
import { countListeners, readDockerInventory, readDockerStats, readListeners } from "./room-coordinated-load-resource-observer.mjs"
import {
  currentOwnedPids,
  readAutomationSnapshot,
  readOwnedProcessMetrics,
  removeAutomationSocket,
  sleep,
  stopProcessGroup,
  tuiEnvironment,
  validateTuiSnapshot,
  waitForAutomationRoom,
} from "./room-coordinated-load-tui-process.mjs"
const statusVariants = new Set([
  "Created", "Running", "Waiting", "Completing", "Paused", "Completed", "Failed", "Stopped",
  "created", "running", "waiting", "completing", "paused", "completed", "failed", "stopped",
])

export async function createRoomCoordinatedLoadRuntime({ plan, repoRoot, runDirectory, signal = null }, dependencies = {}) {
  if (process.platform !== "linux" && process.platform !== "darwin") {
    throw new Error("the local coordinated load runner currently supports Linux and macOS")
  }
  await mkdir(runDirectory, { recursive: true, mode: 0o700 })
  const runStat = await stat(runDirectory)
  if (!runStat.isDirectory() || (runStat.mode & 0o077) !== 0) {
    throw new Error("external evidence run directory must be private")
  }
  const protocol = dependencies.protocol ?? await (async () => {
    const [{ LocalIpcClient }, requests, { openSelkiesDisplayStream }] = await Promise.all([
      import(pathToFileUrl(path.join(repoRoot, "packages/kernel-client/dist/ipc.js"))),
      import(pathToFileUrl(path.join(repoRoot, "packages/kernel-client/dist/ipc-requests.js"))),
      import(pathToFileUrl(path.join(repoRoot, "packages/kernel-client/dist/display-stream.js"))),
    ])
    return { LocalIpcClient, requests, openSelkiesDisplayStream }
  })()
  const { LocalIpcClient, requests, openSelkiesDisplayStream } = protocol
  const processApi = {
    currentOwnedPids, readAutomationSnapshot, readOwnedProcessMetrics, removeAutomationSocket,
    sleep, stopProcessGroup, tuiEnvironment, validateTuiSnapshot, waitForAutomationRoom,
    ...dependencies.tuiProcess,
  }
  const launch = dependencies.spawn ?? spawn
  const makeTuiInvocation = dependencies.roomTuiPtyInvocation ?? roomTuiPtyInvocation
  const resourceOptions = dependencies.execFileAsync ? { execFileAsync: dependencies.execFileAsync } : undefined

  const clients = new Map()
  const viewers = new Map()
  const tuis = new Map()
  const workflows = new Map()
  const relayToken = dependencies.relayToken ?? readRelayToken(plan.relay.tokenEnv)
  const localKernelAuthEnvironment = dependencies.localKernelAuthEnvironment ?? Object.fromEntries(
    ["CHARIOX_KERNEL_LOCAL_AUTH_TOKEN", "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE"]
      .filter((name) => typeof process.env[name] === "string")
      .map((name) => [name, process.env[name]]),
  )
  const clientFor = (route) => {
    if (clients.has(route)) return clients.get(route)
    const client = route === "local"
      ? new LocalIpcClient(plan.homeKernel.url)
      : new LocalIpcClient(plan.relay.url, {
          relayAuthToken: relayToken,
          targetDaemonId: plan.relay.targetDaemonId,
          kernelPingIntervalMs: 60_000,
          kernelMaxMissedPongs: 10,
        })
    clients.set(route, client)
    return client
  }
  const unwrap = (response, variant) => {
    if (!response || typeof response !== "object" || !Object.hasOwn(response, variant)) {
      throw new Error("public kernel response omitted its expected variant")
    }
    return response[variant]
  }
  const own = (kind, id, privateState, extras = {}) => {
    const stopToken = randomUUID()
    taskStates.set(stopToken, { runId: plan.runId, kind, id, ...privateState })
    return { ownerRunId: plan.runId, kind, id, started: true, stopToken, ...extras }
  }
  const taskStates = new Map()

  const runtime = {
    now: () => performance.now(),
    sleep: (milliseconds, abortSignal = signal) => processApi.sleep(milliseconds, abortSignal),

    async captureInventory(_plan, { phase, ownedTasks }) {
      const docker = await readDockerInventory(plan, resourceOptions)
      const targetNames = new Set(plan.headedSlices.map((slice) => `chariox-slice-${slice.sliceName}`))
      const containers = docker.containers
        .filter((container) => targetNames.has(container.name))
        .map((container) => `${container.id}|${container.name}|${container.status}|${container.ports}`)
        .sort()
      if (containers.length !== plan.headedSlices.length) throw new Error("prepared slice container inventory is incomplete")
      if (docker.containers.filter((container) => targetNames.has(container.name)).some((container) => container.status !== "Up")) {
        throw new Error("a prepared headed-slice container is not running")
      }

      const targetVolumes = new Set(plan.headedSlices.map((slice) => `chariox-slice-${slice.sliceName}-home`))
      const volumes = docker.volumes.filter((name) => targetVolumes.has(name)).sort()
      if (volumes.length !== targetVolumes.size) throw new Error("prepared slice volume inventory is incomplete")
      const expectedPorts = [...new Set(plan.headedSlices.flatMap((slice) => slice.publishedPorts))].sort((a, b) => a - b)
      const ports = await readListeners(expectedPorts, resourceOptions)
      const failedStartTasks = [...tuis.values()].map((tui) => ({ kind: "tui", processGroupId: tui.processGroupId }))
      const ownedPids = await processApi.currentOwnedPids([...ownedTasks, ...failedStartTasks])
      return {
        ownedProcessIds: ownedPids,
        containers,
        volumes,
        ports,
        remainingOwnedPids: phase === "after" ? ownedPids : [],
      }
    },

    async verifyPrepared() {
      try {
        const client = clientFor("local")
        const sliceResponse = unwrap(await client.send(requests.listSlicesRequest()), "SlicesListed")
        if (!Array.isArray(sliceResponse.slices)) throw new Error("authoritative slice inventory is unavailable")
        const slices = []
        for (const expected of plan.headedSlices) {
          const slice = unwrap(await client.send(requests.getSliceRequest(expected.sliceId)), "Slice").slice
          const publicRecord = sliceResponse.slices.find((item) => item?.id === expected.sliceId)
          if (!publicRecord || !slice || slice.id !== expected.sliceId || slice.name !== expected.sliceName
            || slice.status !== "running" || slice.display_mode !== "headed" || slice.backend !== "local_docker"
            || slice.owner_kernel_id !== plan.homeKernel.kernelId
            || slice.owner_machine_id !== plan.homeKernel.machineId
            || stableNumbers(slice.local_docker_ports) !== stableNumbers(expected.ports)) {
            throw new Error("public slice identity, headed mode, or running status differs from the approved config")
          }
          const binding = unwrap(await client.send(requests.getRoomEnvironmentSliceRequest(expected.roomId)), "RoomEnvironmentSlice").binding
          const environment = unwrap(await client.send(requests.getRoomEnvironmentStateRequest(expected.roomId)), "RoomEnvironmentState").environment
          if (!binding || binding.session_id !== expected.roomId || binding.slice_id !== expected.sliceId
            || binding.owner_kernel_id !== plan.homeKernel.kernelId
            || !environment || environment.session_id !== expected.roomId
            || environment.environment_id !== expected.environmentId
            || environment.runtime_generation !== expected.runtimeGeneration
            || environment.lifecycle !== "ready") {
            throw new Error("public Room Environment binding or readiness differs from the approved config")
          }
          const workerKernelId = slice.worker_kernel_id ?? slice.owner_kernel_id
          const workerMachineId = slice.worker_machine_id ?? slice.owner_machine_id
          slices.push({
            sliceId: slice.id,
            sliceName: slice.name,
            roomId: expected.roomId,
            environmentId: environment.environment_id,
            runtimeGeneration: environment.runtime_generation,
            displayMode: slice.display_mode,
            sliceStatus: slice.status,
            environmentLifecycle: environment.lifecycle,
            workerKernelId,
            workerMachineId,
          })
        }
        const snapshot = { approvedMaxHeadedSlices: plan.approval.approvedMaxHeadedSlices, slices }
        assertPreparedIdentities(plan, snapshot)
        return snapshot
      } catch (error) {
        await closeAllClients()
        throw error
      }
    },

    async startViewer(_plan, viewer) {
      const client = clientFor(viewer.route)
      const clientId = `${plan.runId}-${viewer.id}`
      const attachment = unwrap(
        await client.send(requests.attachToSessionRequest(targetRoom().roomId, clientId)),
        "SessionAttached",
      ).attachment
      if (!attachment?.id) throw new Error("kernel did not return an owned viewer attachment")
      let stream
      try {
        stream = await openSelkiesDisplayStream({
          client,
          sliceId: targetRoom().sliceId,
          sessionId: targetRoom().roomId,
          attachmentId: attachment.id,
          connectTimeoutMs: 10_000,
          signal,
        })
        await stream.sendControl("START_VIDEO", { signal })
        const started = await receiveMessage(stream, 10_000, signal)
        if (started.kind !== "text" || new TextDecoder().decode(started.data) !== "VIDEO_STARTED") {
          throw new Error("Selkies viewer did not start its normal video stream")
        }
        const firstFrame = await nextBinaryFrame(stream, 15_000, signal)
        viewers.set(viewer.id, { client, stream, attachmentId: attachment.id, route: viewer.route, frameBytes: firstFrame.byteLength })
        return own("viewer", viewer.id, { client, stream, attachmentId: attachment.id, route: viewer.route }, {
          streamId: stream.endpoint.stream_id,
        })
      } catch (error) {
        await stream?.close().catch(() => undefined)
        await client.send(requests.detachFromSessionRequest(attachment.id)).catch(() => undefined)
        await closeUnusedClient(viewer.route)
        throw error
      }
    },

    async startTui(_plan, tui) {
      const automationSocket = path.join(runDirectory, `${tui.id}.sock`)
      const cliPath = path.join(repoRoot, "apps/cli/dist/index.js")
      await access(cliPath)
      const home = path.join(runDirectory, `${tui.id}-home`)
      await mkdir(home, { recursive: true, mode: 0o700 })
      const clientId = `${plan.runId}-${tui.id}`
      const connectionArgs = tui.route === "local"
        ? ["--kernel-url", plan.homeKernel.url]
        : ["--relay-url", plan.relay.url, "--relay-token-env", plan.relay.tokenEnv,
            "--target-daemon-id", plan.relay.targetDaemonId]
      const args = [
        cliPath,
        ...connectionArgs,
        "--automation-socket", automationSocket,
        "--session", targetRoom().roomId,
        "--workspace", plan.workspace.path,
        "--worktree", plan.workspace.worktree,
        "--client-id", clientId,
      ]
      const invocation = makeTuiInvocation(args)
      const environment = processApi.tuiEnvironment(
        home,
        tui.route === "relay" ? relayToken : null,
        plan.relay.tokenEnv,
        tui.route === "local" ? localKernelAuthEnvironment : {},
      )
      const child = launch(invocation.command, invocation.args, {
        cwd: repoRoot,
        env: environment,
        detached: true,
        stdio: ["ignore", "ignore", "ignore"],
      })
      const tuiState = { child, automationSocket, home, route: tui.route, attachmentId: null, processGroupId: child.pid, startupError: false }
      child.once("error", () => { tuiState.startupError = true })
      try {
        await processApi.waitForAutomationRoom(tuiState, targetRoom().roomId, signal)
        tuis.set(tui.id, tuiState)
        return own("tui", tui.id, tuiState, { pid: child.pid, processGroupId: child.pid })
      } catch (error) {
        let stopped = !Number.isSafeInteger(child.pid)
        if (!stopped) {
          try {
            await processApi.stopProcessGroup(child.pid, child)
            stopped = true
          } catch {
            tuis.set(tui.id, tuiState)
          }
        }
        if (tuiState.attachmentId) {
          await clientFor(tui.route).send(requests.detachFromSessionRequest(tuiState.attachmentId)).catch(() => undefined)
          await closeUnusedClient(tui.route)
        }
        if (stopped) {
          await processApi.removeAutomationSocket(automationSocket)
          await rm(home, { recursive: true, force: true })
          tuis.delete(tui.id)
        }
        throw error
      }
    },

    async startWorkflow() {
      const local = viewers.get("web-local")
      if (!local) throw new Error("workflow requires the owned local viewer kernel connection")
      const response = unwrap(await local.client.send(requests.invokeWorkflowEndpointRequest(
        plan.workflow.roomId,
        plan.workflow.workflowId,
        plan.workflow.endpointId,
        plan.workflow.prompt,
      )), "WorkflowRunInvoked")
      const workflowRunId = response.workflow_run?.id
      if (typeof workflowRunId !== "string" || !workflowRunId) {
        throw new Error("kernel did not return the started workflow run identity")
      }
      const state = { workflowRunId, client: local.client }
      workflows.set(workflowRunId, state)
      return own("workflow", plan.workflow.workflowId, state, { workflowRunId })
    },

    async injectSlowViewer(task, delayMs) {
      const state = privateTask(task, "viewer")
      const before = performance.now()
      await processApi.sleep(delayMs, signal)
      await nextBinaryFrame(state.stream, 10_000, signal)
      return {
        viewerId: task.id,
        requestedDelayMs: delayMs,
        observedDelayMs: Math.round(performance.now() - before),
      }
    },

    async sample(_plan, { sampleIndex, ownedTasks, workflowTask, elapsedMs }) {
      const local = viewers.get("web-local")
      const relay = viewers.get("web-relay")
      const localTui = tuis.get("local-tui")
      const relayTui = tuis.get("relay-tui")
      const [localLatency, relayLatency, localFrame, relayFrame, localSnapshot, relaySnapshot,
        workflowStatus, dockerStats, processes, openListenerCount] = await Promise.all([
        measureRoomLatency(local.client, requests, plan.headedSlices),
        measureRoomLatency(relay.client, requests, plan.headedSlices),
        nextBinaryFrame(local.stream, 10_000, signal),
        nextBinaryFrame(relay.stream, 10_000, signal),
        processApi.readAutomationSnapshot(localTui.automationSocket, signal),
        processApi.readAutomationSnapshot(relayTui.automationSocket, signal),
        readWorkflowStatus(workflowTask),
        readDockerStats(plan, resourceOptions),
        processApi.readOwnedProcessMetrics(ownedTasks),
        countListeners([...new Set(plan.headedSlices.flatMap((slice) => slice.publishedPorts))], resourceOptions),
      ])
      processApi.validateTuiSnapshot(localSnapshot, targetRoom().roomId, localTui.attachmentId)
      processApi.validateTuiSnapshot(relaySnapshot, targetRoom().roomId, relayTui.attachmentId)
      return {
        sampleIndex,
        capturedAt: new Date().toISOString(),
        elapsedMs,
        localKernelLatencyMs: localLatency,
        relayKernelLatencyMs: relayLatency,
        aggregateContainerMemoryBytes: dockerStats.memoryBytes,
        aggregateContainerCpuPercent: dockerStats.cpuPercent,
        viewerLocalFrameBytes: localFrame.byteLength,
        viewerRelayFrameBytes: relayFrame.byteLength,
        hostFreeMemoryBytes: os.freemem(),
        processRssBytes: processes.rssBytes,
        ownedProcessCount: processes.count,
        openListenerCount,
        workflowStatus,
      }
    },

    async stopOwnedTask(task) {
      const state = privateTask(task, task?.kind)
      if (task.kind === "workflow") {
        let current = await readWorkflowStatus(task).catch(() => "unknown")
        if (!new Set(["completed", "failed", "stopped"]).has(current)) {
          await state.client.send(requests.cancelWorkflowRunRequest(plan.workflow.roomId, state.workflowRunId))
          current = await waitWorkflowTerminal(task)
        }
        workflows.delete(state.workflowRunId)
      } else if (task.kind === "viewer") {
        let cleanupFailed = false
        await state.stream.sendControl("STOP_VIDEO", { signal }).catch(() => undefined)
        await state.stream.close().catch(() => { cleanupFailed = true })
        await state.client.send(requests.detachFromSessionRequest(state.attachmentId)).catch(() => { cleanupFailed = true })
        viewers.delete(task.id)
        await closeUnusedClient(state.route).catch(() => { cleanupFailed = true })
        if (cleanupFailed) throw new Error("owned viewer cleanup was incomplete")
      } else if (task.kind === "tui") {
        let cleanupFailed = false
        try { await processApi.stopProcessGroup(state.processGroupId, state.child) } catch { cleanupFailed = true }
        try {
          const detachClient = clientFor(state.route)
          await detachClient.send(requests.detachFromSessionRequest(state.attachmentId))
        } catch { cleanupFailed = true }
        await closeUnusedClient(state.route).catch(() => { cleanupFailed = true })
        if (!cleanupFailed) {
          await processApi.removeAutomationSocket(state.automationSocket)
          await rm(state.home, { recursive: true, force: true })
          tuis.delete(task.id)
        }
        if (cleanupFailed) throw new Error("owned TUI cleanup was incomplete")
      } else {
        throw new Error("cannot stop an unrecognized owned task")
      }
      taskStates.delete(task.stopToken)
      return { ownerRunId: plan.runId, stopped: true, remaining: false }
    },
  }

  function targetRoom() {
    return plan.headedSlices.find((slice) => slice.roomId === plan.workflow.roomId)
  }

  function privateTask(task, kind) {
    const state = taskStates.get(task?.stopToken)
    if (!state || state.runId !== plan.runId || state.kind !== kind || state.id !== task.id) {
      throw new Error("task handle is not owned by this coordinated run")
    }
    return state
  }

  async function closeClient(route) {
    const client = clients.get(route)
    if (!client) return
    clients.delete(route)
    await client.close?.().catch(() => undefined)
  }

  async function closeAllClients() {
    for (const route of [...clients.keys()]) await closeClient(route)
  }

  async function closeUnusedClient(route) {
    if ([...viewers.values()].some((viewer) => viewer.route === route)) return
    await closeClient(route)
  }

  async function readWorkflowStatus(task) {
    const workflow = workflows.get(task.workflowRunId)
    if (!workflow || workflow.workflowRunId !== task.workflowRunId) throw new Error("workflow run is not owned by this run")
    const response = unwrap(await workflow.client.send(
      requests.getWorkflowRunRequest(plan.workflow.roomId, workflow.workflowRunId),
    ), "WorkflowRun")
    if (response.workflow_run?.id !== workflow.workflowRunId) throw new Error("kernel returned a different workflow run")
    const status = response.workflow_run?.status ?? response.workflow_run?.state
    if (typeof status !== "string" || !statusVariants.has(status)) throw new Error("kernel workflow status is malformed")
    return status.toLowerCase()
  }

  async function waitWorkflowTerminal(task) {
    const deadline = performance.now() + 5_000
    while (performance.now() < deadline) {
      const state = await readWorkflowStatus(task)
      if (["completed", "failed", "stopped"].includes(state)) return state
      await processApi.sleep(100, null)
    }
    throw new Error("owned workflow did not settle after cancellation")
  }

  return runtime
}

async function measureRoomLatency(client, requests, slices) {
  const started = performance.now()
  for (const slice of slices) {
    const environment = unwrap(
      await client.send(requests.getRoomEnvironmentStateRequest(slice.roomId)),
      "RoomEnvironmentState",
    ).environment
    if (environment?.session_id !== slice.roomId || environment?.environment_id !== slice.environmentId
      || environment?.runtime_generation !== slice.runtimeGeneration || environment?.lifecycle !== "ready") {
      throw new Error("live client observed a different or unready Room Environment")
    }
  }
  return Math.max(0, performance.now() - started)
}

async function receiveMessage(stream, timeoutMs, signal) {
  return await stream.receive({ timeoutMs, signal })
}

async function nextBinaryFrame(stream, timeoutMs, signal) {
  const deadline = performance.now() + timeoutMs
  while (performance.now() < deadline) {
    const message = await receiveMessage(stream, Math.max(1, deadline - performance.now()), signal)
    if (message.kind === "binary" && message.data.byteLength > 0) return message.data
    if (message.kind !== "text") throw new Error("display stream returned an unsupported message")
  }
  throw new Error("display stream did not deliver a binary frame within the bound")
}

function readRelayToken(environmentName) {
  const token = process.env[environmentName]
  if (typeof token !== "string" || token.length < 16 || token.length > 8_192) {
    throw new Error(`required relay credential environment variable ${environmentName} is unavailable`)
  }
  return token
}

function stableNumbers(value) {
  if (!value || typeof value !== "object") return ""
  return JSON.stringify(Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right))))
}

function pathToFileUrl(filePath) {
  return pathToFileURL(filePath).href
}
