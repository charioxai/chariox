#!/usr/bin/env node
import { randomUUID } from "node:crypto"
import { spawn } from "node:child_process"
import { homedir } from "node:os"
import { basename, dirname, isAbsolute, join, resolve } from "node:path"
import { mkdir, lstat, readFile, realpath, writeFile } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import {
  acquireManagedReleaseBuilderLease,
  hashManagedReleaseSource,
  isStateDirectoryExternal,
  parseBuildHistoryList,
  reconcileManagedReleaseBuild,
} from "./managed-release-settlement.mjs"

const PROBE_TIMEOUT_MS = 10_000
const CLIENT_TIMEOUT_MS = 120_000
const RUNNING_TIMEOUT_MS = 30_000
const SETTLEMENT_TIMEOUT_MS = 120_000
const CLEANUP_TIMEOUT_MS = 10_000
const POLL_INTERVAL_MS = 250
const MAX_OUTPUT_BYTES = 2 * 1024 * 1024
const OWNER_FILE = ".managed-release-settlement-owner.json"
const BUILDER_PATTERN = /^chariox-settlement-[a-z0-9-]{8,80}$/
const BASE_IMAGE_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._/:@-]*@sha256:[a-f0-9]{64}$/
const MAX_CPUS = 2
const MAX_PIDS = 256
const MAX_MEMORY_BYTES = 8 * 1024 ** 3
const MAX_SWAP_BYTES = 12 * 1024 ** 3

function usage() {
  return [
    "Usage: node scripts/managed-release-settlement-drill.mjs --builder NAME --container-id SHA256 --base-image IMAGE@sha256:DIGEST",
    "       [--scratch-parent ABSOLUTE_PATH] [--evidence-parent ABSOLUTE_PATH]",
    "Requires an isolated, resource-capped Buildx builder and a digest-pinned base already in its cache; the drill does not seed or pull it, and network=none makes a cache miss fail offline.",
  ].join("\n")
}

function parseArgs(argv) {
  const values = new Map()
  for (let index = 0; index < argv.length; index++) {
    const argument = argv[index]
    if (!argument.startsWith("--") || index + 1 >= argv.length || argv[index + 1].startsWith("--")) {
      throw new Error(usage())
    }
    const key = argument.slice(2)
    if (values.has(key)) throw new Error(`duplicate option --${key}`)
    values.set(key, argv[++index])
  }
  const builder = values.get("builder")
  const containerId = values.get("container-id")
  const baseImage = values.get("base-image")
  const allowed = new Set(["builder", "container-id", "base-image", "scratch-parent", "evidence-parent"])
  if ([...values.keys()].some((key) => !allowed.has(key)) ||
      !BUILDER_PATTERN.test(builder ?? "") || !/^[a-f0-9]{64}$/.test(containerId ?? "") ||
      !BASE_IMAGE_PATTERN.test(baseImage ?? "")) {
    throw new Error(usage())
  }
  return {
    builder,
    containerId,
    baseImage,
    scratchParent: values.get("scratch-parent") ?? join(homedir(), ".chariox", "dev", "path1-buildkit-settlement-drill", "scratch"),
    evidenceParent: values.get("evidence-parent") ?? join(homedir(), ".chariox", "dev", "path1-buildkit-settlement-drill", "evidence"),
  }
}

function runCommand(command, args, { cwd, timeoutMs = PROBE_TIMEOUT_MS, env = process.env } = {}) {
  return new Promise((resolvePromise, rejectPromise) => {
    let child
    try {
      child = spawn(command, args, { cwd, env, stdio: ["ignore", "pipe", "pipe"], windowsHide: true })
    } catch (error) {
      rejectPromise(error)
      return
    }
    resolvePromise(trackChild(child, timeoutMs).completion)
  })
}

function trackChild(child, timeoutMs) {
  let stdout = ""
  let stderr = ""
  let outputBytes = 0
  let timedOut = false
  let outputLimitExceeded = false
  let finished = false
  let exitResult = null
  let spawnError = null
  let resolveCompletion
  let drainScheduled = false
  const timers = new Set()
  const completion = new Promise((resolvePromise) => { resolveCompletion = resolvePromise })
  const schedule = (delayMs, callback) => {
    const timer = setTimeout(() => {
      timers.delete(timer)
      callback()
    }, delayMs)
    timers.add(timer)
    return timer
  }
  const finish = (result) => {
    if (finished) return
    finished = true
    for (const timer of timers) clearTimeout(timer)
    timers.clear()
    resolveCompletion({
      stdout,
      stderr,
      timedOut,
      outputLimitExceeded,
      ...result,
    })
  }
  const closeStreams = () => {
    for (const stream of [child.stdout, child.stderr]) {
      if (stream && !stream.destroyed) stream.destroy()
    }
  }
  const scheduleBoundedDrain = () => {
    if (drainScheduled || finished) return
    drainScheduled = true
    schedule(1_000, () => {
      if (finished) return
      closeStreams()
      schedule(250, () => {
        if (finished) return
        if (exitResult) {
          finish({ ...exitResult, reaped: true, streamsClosed: true, forcedPipeClose: true })
        } else if (spawnError && child.pid == null) {
          finish({ status: null, signal: null, error: spawnError.message, reaped: true, streamsClosed: true, forcedPipeClose: true })
        } else {
          finish({ status: null, signal: "unreaped", reaped: false, streamsClosed: true, forcedPipeClose: true })
        }
      })
    })
  }
  const requestKill = (reason) => {
    if (reason === "timeout") timedOut = true
    if (reason === "output-limit") outputLimitExceeded = true
    if (!finished) child.kill("SIGKILL")
    scheduleBoundedDrain()
  }
  const collect = (stream, destination) => stream?.on("data", (chunk) => {
    if (finished || outputLimitExceeded) return
    outputBytes += chunk.length
    if (outputBytes > MAX_OUTPUT_BYTES) {
      requestKill("output-limit")
      return
    }
    if (destination === "stdout") stdout += chunk.toString("utf8")
    else stderr += chunk.toString("utf8")
  })
  collect(child.stdout, "stdout")
  collect(child.stderr, "stderr")
  child.once("error", (error) => {
    spawnError = error
    scheduleBoundedDrain()
  })
  child.once("exit", (status, signal) => {
    exitResult = { status, signal }
    scheduleBoundedDrain()
  })
  child.once("close", (status, signal) => finish({ status, signal, reaped: true, streamsClosed: true }))
  const timeout = schedule(timeoutMs, () => requestKill("timeout"))
  timeout.unref?.()
  return {
    completion,
    get closed() { return finished },
    terminate: requestKill,
  }
}

async function checkedDocker(args, label, options = {}) {
  const result = await runCommand("docker", args, options)
  if (result.timedOut) throw new Error(`${label} exceeded its ${options.timeoutMs ?? PROBE_TIMEOUT_MS}ms deadline`)
  if (result.outputLimitExceeded) throw new Error(`${label} exceeded the output limit`)
  if (!result.reaped || !result.streamsClosed) throw new Error(`${label} did not close and reap within its deadline`)
  if (result.status !== 0) {
    const detail = result.stderr.trim().slice(-2000)
    throw new Error(`${label} failed${detail ? `: ${detail}` : ""}`)
  }
  return result.stdout
}

function endpointArgs(endpoint) {
  if (/^(unix|tcp|ssh):\/\/[^\s]+$/.test(endpoint)) return ["--host", endpoint]
  if (/^[A-Za-z0-9][A-Za-z0-9_.-]{0,255}$/.test(endpoint)) return ["--context", endpoint]
  throw new Error("Buildx node endpoint is unsupported")
}

function parseBuilderInspect(output, expectedName) {
  const fields = new Map()
  const nodes = []
  let inNodes = false
  let current
  for (const line of output.split(/\r?\n/)) {
    if (!inNodes) {
      const match = /^([A-Za-z][A-Za-z ]*):\s*(.*?)\s*$/.exec(line)
      if (!match) continue
      if (match[1] === "Nodes") {
        inNodes = true
        continue
      }
      const values = fields.get(match[1]) ?? []
      values.push(match[2])
      fields.set(match[1], values)
      continue
    }
    const match = /^(\s*)([A-Za-z][A-Za-z ]*):\s*(.*?)\s*$/.exec(line)
    if (!match || ![0, 2].includes(match[1].length)) continue
    if (match[2] === "Name") {
      current = new Map()
      nodes.push(current)
    }
    if (current) {
      const values = current.get(match[2]) ?? []
      values.push(match[3])
      current.set(match[2], values)
    }
  }
  const one = (source, key) => {
    const values = source.get(key) ?? []
    if (values.length !== 1 || !values[0]) throw new Error(`Buildx inspect is missing or ambiguous ${key}`)
    return values[0]
  }
  const name = one(fields, "Name")
  if (name !== expectedName || one(fields, "Driver") !== "docker-container") {
    throw new Error("requested builder identity or docker-container driver does not match")
  }
  if (!BUILDER_PATTERN.test(name) || nodes.length !== 1) {
    throw new Error("drill requires its uniquely named single-node builder")
  }
  const node = nodes[0]
  const nodeName = one(node, "Name")
  const endpoint = one(node, "Endpoint")
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(nodeName) || one(node, "Status") !== "running") {
    throw new Error("Buildx node is not running or has a malformed identity")
  }
  return { name, nodes: [{ name: nodeName, endpoint }] }
}

function parseContainerInspect(output) {
  let container
  try {
    container = JSON.parse(output.trim())
  } catch {
    throw new Error("Docker container inspect output is malformed")
  }
  if (Array.isArray(container)) {
    if (container.length !== 1) throw new Error("Docker container inspect was ambiguous")
    container = container[0]
  }
  if (!container || typeof container !== "object" || Array.isArray(container)) {
    throw new Error("Docker container inspect output is malformed")
  }
  return container
}

function inspectResources(container, node, expectedContainerId) {
  if (container.Id !== expectedContainerId || !/^[a-f0-9]{64}$/.test(container.Id) ||
      container.Name !== `/buildx_buildkit_${node.name}` || container.State?.Running !== true ||
      !Number.isSafeInteger(container.State?.Pid) || container.State.Pid <= 0 ||
      !Number.isFinite(Date.parse(container.State?.StartedAt ?? ""))) {
    throw new Error("running BuildKit container does not match the explicitly admitted container identity")
  }
  const host = container.HostConfig
  if (!host || host.Init !== true || host.NetworkMode !== "none") {
    throw new Error("owned BuildKit container must read back init=true and network=none")
  }
  const pids = host.PidsLimit
  const memory = host.Memory
  const swap = host.MemorySwap
  if (!Number.isSafeInteger(pids) || pids < 1 || pids > MAX_PIDS ||
      !Number.isSafeInteger(memory) || memory < 1024 ** 3 || memory > MAX_MEMORY_BYTES ||
      !Number.isSafeInteger(swap) || swap < memory || swap > MAX_SWAP_BYTES) {
    throw new Error("owned BuildKit container must read back finite PID, memory, and memory-swap caps")
  }
  const nanoCpus = host.NanoCpus ?? 0
  const quota = host.CpuQuota ?? 0
  const period = host.CpuPeriod ?? 0
  const cpuSet = host.CpusetCpus ?? ""
  if (![nanoCpus, quota, period].every((value) => Number.isSafeInteger(value) && value >= 0)) {
    throw new Error("BuildKit container CPU caps are malformed")
  }
  const caps = []
  if (nanoCpus > 0) caps.push(nanoCpus / 1e9)
  if (quota > 0 || period > 0) {
    if (quota === 0 || period === 0) throw new Error("BuildKit container CPU quota is incomplete")
    caps.push(quota / period)
  }
  if (cpuSet) {
    let count = 0
    const seen = new Set()
    for (const range of cpuSet.split(",")) {
      const match = /^(0|[1-9]\d*)(?:-(0|[1-9]\d*))?$/.exec(range)
      if (!match) throw new Error("BuildKit container CPU set is malformed")
      const start = Number(match[1])
      const end = Number(match[2] ?? match[1])
      if (end < start || end - start > MAX_CPUS) throw new Error("BuildKit CPU set exceeds the drill limit")
      for (let cpu = start; cpu <= end; cpu++) {
        if (seen.has(cpu)) throw new Error("BuildKit container CPU set is duplicated")
        seen.add(cpu)
      }
    }
    count = seen.size
    if (count > 0) caps.push(count)
  }
  if (caps.length === 0) throw new Error("owned BuildKit container has no hard CPU cap")
  const cpus = Math.min(...caps)
  if (!Number.isFinite(cpus) || cpus < 1 || cpus > MAX_CPUS) {
    throw new Error("owned BuildKit container CPU cap exceeds the drill limit")
  }
  return {
    name: node.name,
    endpoint: node.endpoint,
    containerId: container.Id,
    startedAt: container.State.StartedAt,
    pidsLimit: pids,
    effectiveCpus: cpus,
    memoryBytes: memory,
    memoryWithSwapBytes: swap,
    init: host.Init,
    networkMode: host.NetworkMode,
  }
}

class DockerBuildxEngine {
  constructor({ builder, containerId }) {
    this.builder = builder
    this.containerId = containerId
    this.node = null
    this.dockerEnvironment = process.env
  }

  async inspectBuilder() {
    const output = await checkedDocker(["buildx", "inspect", this.builder], "docker buildx inspect")
    const parsed = parseBuilderInspect(output, this.builder)
    const node = parsed.nodes[0]
    const inspectArgs = [
      ...endpointArgs(node.endpoint), "inspect", "--type", "container", "--format", "{{json .}}",
      `buildx_buildkit_${node.name}`,
    ]
    const containerOutput = await checkedDocker(inspectArgs, "docker inspect")
    const measured = inspectResources(parseContainerInspect(containerOutput), node, this.containerId)
    this.node = node
    return { name: parsed.name, nodes: [measured] }
  }

  async historyList() {
    return checkedDocker(
      ["buildx", "history", "ls", "--builder", this.builder, "--format", "json", "--no-trunc"],
      "docker buildx history ls",
    )
  }

  async historyInspect(id, sourceDirectory) {
    if (!this.node) throw new Error("Buildx node was not inspected")
    const output = await checkedDocker(
      ["buildx", "history", "inspect", "--builder", this.builder, "--format", "json", id],
      "docker buildx history inspect",
      { cwd: sourceDirectory },
    )
    try {
      const record = JSON.parse(output)
      if (!record || typeof record !== "object" || Array.isArray(record)) throw new Error()
      return record
    } catch {
      throw new Error("docker buildx history inspect output is malformed")
    }
  }

  startBuild(sourceDirectory, { baseImage, invocationId }) {
    const args = [
      "buildx", "build", "--builder", this.builder, "--pull=false", "--network=none",
      "--target", "managed-release-artifacts", "--output", "type=cacheonly",
      "--progress=plain", "--build-arg", `DRILL_NONCE=${invocationId}`, sourceDirectory,
    ]
    let child
    try {
      child = spawn("docker", args, {
        cwd: sourceDirectory,
        env: this.dockerEnvironment,
        stdio: ["ignore", "pipe", "pipe"],
        windowsHide: true,
      })
    } catch (error) {
      throw error
    }
    return trackBuildClient(child, CLIENT_TIMEOUT_MS)
  }
}

function trackBuildClient(child, timeoutMs) {
  const tracked = trackChild(child, timeoutMs)
  return {
    completion: tracked.completion,
    get closed() { return tracked.closed },
    async interrupt() {
      if (tracked.closed) return { ...(await tracked.completion), interrupted: false }
      const sent = child.kill("SIGINT")
      const waitForClose = async (waitMs) => {
        let timer
        const timed = new Promise((resolvePromise) => {
          timer = setTimeout(() => resolvePromise(null), waitMs)
        })
        const result = await Promise.race([tracked.completion, timed])
        clearTimeout(timer)
        return result
      }
      const result = await waitForClose(5_000)
      if (result) return { ...result, interrupted: sent }
      tracked.terminate("interrupt-timeout")
      const forced = await waitForClose(2_000) ?? { status: null, signal: "unreaped", reaped: false, streamsClosed: false }
      return { ...forced, interrupted: sent, forced: true }
    },
  }
}

function parseHistoryRows(output) {
  const refs = parseBuildHistoryList(output)
  if (!output.trim()) return []
  return output.trim().split(/\r?\n/).map((row, index) => {
    let parsed
    try {
      parsed = JSON.parse(row.trim())
    } catch {
      throw new Error(`Buildx history row ${index + 1} is malformed`)
    }
    return { ref: refs[index], status: parsed.status.toLowerCase() }
  })
}

async function createOwnedDirectory(parent, kind) {
  const token = randomUUID()
  const directory = join(parent, `${kind}-${token}`)
  await mkdir(directory, { mode: 0o700 })
  await writeFile(join(directory, OWNER_FILE), JSON.stringify({ kind, token }), { flag: "wx", mode: 0o600 })
  return { directory, token, kind }
}

async function removeOwnedDirectory(owned, timeoutMs = CLEANUP_TIMEOUT_MS) {
  const started = Date.now()
  const metadata = await lstat(owned.directory)
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) throw new Error("owned scratch root changed type")
  const marker = JSON.parse(await readFile(join(owned.directory, OWNER_FILE), "utf8"))
  if (marker.kind !== owned.kind || marker.token !== owned.token || basename(owned.directory) !== `${owned.kind}-${owned.token}`) {
    throw new Error("owned scratch cleanup refused a foreign owner marker")
  }
  const parent = await realpath(dirname(owned.directory))
  const actual = await realpath(owned.directory)
  if (dirname(actual) !== parent || basename(actual) !== basename(owned.directory)) {
    throw new Error("owned scratch cleanup path no longer matches its recorded parent")
  }
  const remainingMs = timeoutMs - (Date.now() - started)
  if (remainingMs <= 0) throw new Error("owned scratch cleanup exceeded its deadline")
  const result = await runCommand("rm", ["-rf", "--", actual], { timeoutMs: remainingMs })
  if (result.timedOut) throw new Error("owned scratch cleanup exceeded its deadline")
  if (!result.reaped || !result.streamsClosed || result.status !== 0) throw new Error("owned scratch cleanup command failed or did not close and reap")
}

function barrierFor({ builderName, invocationId, sourceCommit, sourceTree, sourceDigest, sourceDirectory, runDirectory, home, startedAt, historyBaseline, builderFingerprint }) {
  const outputPath = join(home, "unproduced-managed-release-output")
  return {
    schemaVersion: 1,
    builderName,
    invocationId,
    sourceCommit,
    sourceTree,
    sourceDigest,
    sourceDirectory,
    runDirectory,
    outputPath,
    pendingDirectory: join(dirname(outputPath), `.new-${basename(outputPath)}-${invocationId}`),
    startedAt,
    historyBaseline,
    builderFingerprint,
    buildRef: null,
  }
}

async function gitIdentity(repository) {
  const commit = await runCommand("git", ["rev-parse", "HEAD"], { cwd: repository })
  const tree = await runCommand("git", ["rev-parse", "HEAD^{tree}"], { cwd: repository })
  if (commit.status !== 0 || tree.status !== 0 || !/^[a-f0-9]{40}\n?$/.test(commit.stdout) || !/^[a-f0-9]{40}\n?$/.test(tree.stdout)) {
    throw new Error("could not identify the checked out source commit and tree")
  }
  return { sourceCommit: commit.stdout.trim(), sourceTree: tree.stdout.trim() }
}

function ensureSafeExternalPath(path, repository) {
  if (!isAbsolute(path)) throw new Error("scratch and evidence parents must be absolute paths")
  const absolute = resolve(path)
  if (!isStateDirectoryExternal(absolute, repository)) throw new Error("scratch and evidence must remain outside the repository")
  return absolute
}

async function canonicalParent(path, repository) {
  const absolute = ensureSafeExternalPath(path, repository)
  await mkdir(absolute, { recursive: true, mode: 0o700 })
  const canonical = await realpath(absolute)
  if (!isStateDirectoryExternal(canonical, repository)) throw new Error("canonical scratch or evidence parent is inside the repository")
  const metadata = await lstat(canonical)
  if (!metadata.isDirectory() || (metadata.mode & 0o077) !== 0) {
    throw new Error("scratch and evidence parent directories must be private")
  }
  return canonical
}

function assertNewOwnedHistory(refs, baseline, builderName, nodeNames) {
  const previous = new Set(baseline)
  const fresh = refs.filter((ref) => !previous.has(ref))
  if (fresh.length === 0) return null
  if (fresh.length !== 1) throw new Error("Buildx history did not identify exactly one new Solve")
  const [builder, node, id, ...extra] = fresh[0].split("/")
  if (builder !== builderName || !nodeNames.includes(node) || !id || extra.length) {
    throw new Error("new Buildx history reference belongs to a foreign builder or node")
  }
  return fresh[0]
}

async function invocationSettlement(engine, barrier, sourceDirectory) {
  if (await hashManagedReleaseSource(sourceDirectory) !== barrier.sourceDigest) {
    return { settled: false, reason: "retained source does not match its invocation digest" }
  }
  const currentBuilderFingerprint = await engine.inspectBuilder()
  return reconcileManagedReleaseBuild({
    barrier,
    currentBuilderFingerprint,
    historyList: async () => parseBuildHistoryList(await engine.historyList()),
    historyInspect: (id, cwd) => engine.historyInspect(id, cwd),
  })
}

async function pause(ms) {
  await new Promise((resolvePromise) => setTimeout(resolvePromise, ms))
}

async function scenario({ ownedScratch, evidence, home, builderName, baseImage, repository, engine, sleep = pause }) {
  const events = []
  let lease
  let barrier
  let barrierPath
  let client
  let terminalProof
  let cleanupComplete = false
  const evidencePath = join(evidence.directory, "settlement-report.json")
  const record = async (phase, details = {}) => {
    events.push({ phase, at: new Date().toISOString(), ...details })
  }
  const saveEvidence = async (result) => writeFile(evidencePath, JSON.stringify({
    schemaVersion: 1,
    scope: "BuildKit settlement seam only; no signed release acceptance",
    builderName,
    baseImage,
    scratchPath: ownedScratch.directory,
    scratchRemoved: cleanupComplete,
    terminalProof,
    events,
    ...result,
  }, null, 2), { mode: 0o600 })

  try {
    const builderFingerprint = await engine.inspectBuilder()
    if (builderFingerprint.name !== builderName || builderFingerprint.nodes.length !== 1) {
      throw new Error("builder fingerprint does not match the single admitted node")
    }
    await record("builder-admitted", { resourceReadback: builderFingerprint.nodes[0] })
    const historyBaseline = parseBuildHistoryList(await engine.historyList())
    const invocationId = randomUUID()
    const runDirectory = await leaseAndRunDirectory(invocationId)
    const sourceDirectory = join(runDirectory, "source")
    await mkdir(sourceDirectory, { mode: 0o700 })
    await writeFile(join(sourceDirectory, "Dockerfile"), [
      `FROM ${baseImage} AS managed-release-artifacts`,
      "ARG DRILL_NONCE",
      "RUN test -n \"$DRILL_NONCE\" && sleep 60",
      "COPY fixture.txt /fixture.txt",
      "",
    ].join("\n"), { flag: "wx", mode: 0o600 })
    await writeFile(join(sourceDirectory, "fixture.txt"), `BuildKit settlement fixture ${invocationId}\n`, { flag: "wx", mode: 0o600 })
    const canonicalSource = await realpath(sourceDirectory)
    if (canonicalSource !== sourceDirectory) throw new Error("controlled build context path was not canonical")
    const identity = await gitIdentity(repository)
    const sourceDigest = await hashManagedReleaseSource(sourceDirectory)
    barrier = barrierFor({
      builderName,
      invocationId,
      ...identity,
      sourceDigest,
      sourceDirectory,
      runDirectory,
      home,
      startedAt: new Date().toISOString(),
      historyBaseline,
      builderFingerprint,
    })
    await lease.writeBarrier(barrier)
    await record("barrier-persisted", { invocationId, sourceDigest, historyBaselineCount: historyBaseline.length })

    client = await engine.startBuild(sourceDirectory, { baseImage, invocationId, timeoutMs: CLIENT_TIMEOUT_MS })
    const runningDeadline = Date.now() + RUNNING_TIMEOUT_MS
    let fullRef
    while (Date.now() < runningDeadline) {
      if (client.closed) throw new Error("Buildx client exited before an owned running Solve was observed")
      const rows = parseHistoryRows(await engine.historyList())
      fullRef = assertNewOwnedHistory(rows.map((row) => row.ref), historyBaseline, builderName, builderFingerprint.nodes.map((node) => node.name))
      if (!fullRef) {
        await sleep(POLL_INTERVAL_MS)
        continue
      }
      const row = rows.find((candidate) => candidate.ref === fullRef)
      if (row.status === "running") break
      throw new Error("Buildx Solve became terminal before the client interruption point")
    }
    if (!fullRef) throw new Error("Buildx history did not expose the Solve before the running deadline")

    const [listedBuilder, listedNode, bareId] = fullRef.split("/")
    const runningBarrier = { ...barrier, buildRef: fullRef }
    const runningResult = await reconcileManagedReleaseBuild({
      barrier: runningBarrier,
      currentBuilderFingerprint: builderFingerprint,
      historyList: async () => parseBuildHistoryList(await engine.historyList()),
      historyInspect: (id, cwd) => engine.historyInspect(id, cwd),
    })
    if (listedBuilder !== builderName || !builderFingerprint.nodes.some((node) => node.name === listedNode) ||
        !bareId || runningResult.settled || runningResult.reason !== "build history has no terminal status") {
      throw new Error(`Buildx did not provide an invocation-bound running record: ${runningResult.reason ?? "reference mismatch"}`)
    }
    barrier = runningBarrier
    await lease.writeBarrier(barrier)
    await record("running-full-ref-persisted", { fullRef, inspectedId: bareId, inspectResult: runningResult.reason })

    await lease.release()
    lease = null
    lease = await acquireManagedReleaseBuilderLease({ home, builderName })
    const retained = await lease.readBarrier()
    if (!retained || retained.buildRef !== fullRef) throw new Error("successor could not read the exact persisted full build reference")
    const successorResult = await invocationSettlement(engine, retained, sourceDirectory)
    if (successorResult.settled || successorResult.reason !== "build history has no terminal status") {
      throw new Error(`successor did not reject the unresolved invocation: ${successorResult.reason ?? "unexpected terminal proof"}`)
    }
    barrier = retained
    await record("successor-rejected-unsettled", { fullRef, reason: successorResult.reason })

    const interrupted = await client.interrupt()
    client = null
    if (!interrupted || !interrupted.interrupted || !interrupted.reaped || !interrupted.streamsClosed ||
        interrupted.timedOut || interrupted.outputLimitExceeded || interrupted.error || interrupted.signal === "timeout" || interrupted.signal === "unreaped") {
      throw new Error("Buildx client did not stop within its bounded interruption deadline")
    }
    await record("client-interrupted", { status: interrupted.status, signal: interrupted.signal })

    const settleDeadline = Date.now() + SETTLEMENT_TIMEOUT_MS
    let lastResult
    while (Date.now() < settleDeadline) {
      lastResult = await invocationSettlement(engine, barrier, sourceDirectory)
      if (lastResult.settled) {
        terminalProof = { fullRef: lastResult.buildRef, status: lastResult.status, observedAt: new Date().toISOString() }
        break
      }
      if (lastResult.reason !== "build history has no terminal status") {
        throw new Error(`persisted invocation could not be safely reconciled: ${lastResult.reason}`)
      }
      await sleep(POLL_INTERVAL_MS)
    }
    if (!terminalProof) throw new Error(`Buildx Solve did not reach an exact terminal proof: ${lastResult?.reason ?? "settlement deadline elapsed"}`)
    await record("terminal-proof", terminalProof)

    await lease.removeRunArtifacts(barrier)
    await lease.removeBarrier()
    await lease.release()
    lease = null
    await removeOwnedDirectory(ownedScratch, CLEANUP_TIMEOUT_MS)
    cleanupComplete = true
    const result = { outcome: "settled", terminalProof, evidencePath }
    await saveEvidence({ outcome: result.outcome })
    return result
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    if (client) {
      try {
        const interrupted = await client.interrupt()
        events.push({ phase: "client-stopped-after-drill-error", status: interrupted.status, signal: interrupted.signal })
        client = null
      } catch (interruptError) {
        events.push({ phase: "client-stop-failed", error: interruptError instanceof Error ? interruptError.message : String(interruptError) })
      }
    }
    await record("unproven-preserved", { error: message, barrierPath })
    if (lease) {
      try { await lease.release(); lease = null } catch (releaseError) {
        events.push({ phase: "lease-release-failed", error: releaseError instanceof Error ? releaseError.message : String(releaseError) })
      }
    }
    await saveEvidence({ outcome: terminalProof ? "cleanup-failed" : "unproven", error: message, barrierPath: barrierPath ?? null })
    throw Object.assign(new Error(`${message}; unresolved scratch and evidence were preserved at ${ownedScratch.directory}`), {
      evidencePath,
    })
  }

  async function leaseAndRunDirectory(invocationId) {
    lease = await acquireManagedReleaseBuilderLease({ home, builderName })
    barrierPath = lease.barrierPath
    const existing = await lease.readBarrier()
    if (existing) throw new Error("unique drill state already contains an unresolved invocation")
    return lease.createRunDirectory(invocationId)
  }
}

async function prepareParents(args, repository) {
  const scratchParent = await canonicalParent(args.scratchParent, repository)
  const evidenceParent = await canonicalParent(args.evidenceParent, repository)
  const scratchOwned = await createOwnedDirectory(scratchParent, "scratch")
  const evidenceOwned = await createOwnedDirectory(evidenceParent, "evidence")
  return { scratchOwned, evidenceOwned }
}

async function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv)
  const repository = await realpath(dirname(dirname(modulePath)))
  const { scratchOwned, evidenceOwned } = await prepareParents(args, repository)
  const home = join(scratchOwned.directory, "home")
  await mkdir(home, { mode: 0o700 })
  const engine = new DockerBuildxEngine({ builder: args.builder, containerId: args.containerId })
  try {
    const result = await scenario({
      ownedScratch: scratchOwned,
      evidence: evidenceOwned,
      home,
      builderName: args.builder,
      baseImage: args.baseImage,
      repository,
      engine,
    })
    process.stdout.write(`${JSON.stringify(result)}\n`)
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
    process.exitCode = 1
  }
}

const modulePath = fileURLToPath(import.meta.url)
if (process.argv[1] && resolve(process.argv[1]) === modulePath) {
  main().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
    process.exitCode = 1
  })
}

export {
  BUILDER_PATTERN,
  OWNER_FILE,
  parseHistoryRows,
  removeOwnedDirectory,
  scenario as runSettlementScenario,
}
