#!/usr/bin/env node

import { createHash, createPrivateKey, createPublicKey, randomUUID, sign } from "node:crypto"
import { spawn, spawnSync } from "node:child_process"
import { chmod, copyFile, lstat, mkdir, readFile, readdir, rename, rm, stat, writeFile } from "node:fs/promises"
import { constants } from "node:fs"
import { basename, dirname, join, resolve } from "node:path"
import {
  acquireManagedReleaseBuilderLease,
  hashManagedReleaseSource,
  isStateDirectoryExternal,
  isTerminalBuildHistoryRecord,
  parseBuildHistoryList,
  reconcileManagedReleaseBuild,
  validateBuildMetadata,
} from "./managed-release-settlement.mjs"

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex")
const BUILD_TARGET = "x86_64-unknown-linux-gnu"
const ARTIFACT_STAGE = "managed-release-artifacts"
const BUILDER_DOCKERFILE = "apps/kernel/slice-linux-docker/docker/Dockerfile"
const ARTIFACTS = ["chariox-kernel", "chariox-managed-bootstrap", "chariox-relay"]
const REQUIRED_OPTIONS = ["source-repository", "source-commit", "builder-signing-key", "output", "builder"]
const BUILDER_NAME_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,62}$/
const NODE_NAME_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,255}$/
const DEFAULT_PREFLIGHT_TIMEOUT_SECONDS = 15
const MAX_PREFLIGHT_TIMEOUT_SECONDS = 60
const DEFAULT_BUILD_TIMEOUT_SECONDS = 2 * 60 * 60
const MAX_BUILD_TIMEOUT_SECONDS = 4 * 60 * 60
const MAX_DOCKER_OUTPUT_BYTES = 4 * 1024 * 1024
const POST_EXIT_OUTPUT_DRAIN_MS = 1_000
const FORCED_OUTPUT_CLOSE_MS = 250
const MAX_BUILDKIT_CPUS = 8
const MAX_BUILDKIT_PIDS = 1024
const MAX_BUILDKIT_MEMORY_BYTES = 16 * 1024 ** 3
const MAX_BUILDKIT_MEMORY_WITH_SWAP_BYTES = 32 * 1024 ** 3
const MIN_BUILDKIT_MEMORY_BYTES = 4 * 1024 ** 3
const BUILD_CANCELLATION_GRACE_MS = 15_000
const BUILD_HISTORY_READ_TIMEOUT_MS = 15_000

function usage() {
  return "usage: build-managed-kernel-release --source-repository <git-worktree> --source-commit <40-hex-commit> --builder-signing-key <ed25519-key> --builder <name> --output <new-directory> [--preflight-timeout-seconds <1-60>] [--build-timeout-seconds <1-14400>]"
}

function parseBoundedSeconds(value, name, maximum) {
  if (!/^[1-9][0-9]*$/.test(value)) throw new Error(`${name} must be an integer from 1 to ${maximum}`)
  const seconds = Number(value)
  if (!Number.isSafeInteger(seconds) || seconds > maximum) {
    throw new Error(`${name} must be an integer from 1 to ${maximum}`)
  }
  return seconds
}

function parseOptions(argv) {
  const options = new Map()
  for (let index = 0; index < argv.length; index += 2) {
    const option = argv[index]
    const value = argv[index + 1]
    if (!option?.startsWith("--") || !value || value.startsWith("--")) throw new Error(usage())
    const name = option.slice(2)
    if (![...REQUIRED_OPTIONS, "preflight-timeout-seconds", "build-timeout-seconds"].includes(name) || options.has(name)) {
      throw new Error(usage())
    }
    if (name === "builder" && !BUILDER_NAME_PATTERN.test(value)) {
      throw new Error("builder name must be 1 to 63 ASCII letters, digits, dots, underscores, or hyphens")
    }
    if (name === "preflight-timeout-seconds") {
      options.set(name, parseBoundedSeconds(value, name, MAX_PREFLIGHT_TIMEOUT_SECONDS))
    } else if (name === "build-timeout-seconds") {
      options.set(name, parseBoundedSeconds(value, name, MAX_BUILD_TIMEOUT_SECONDS))
    } else {
      options.set(name, name === "source-commit" || name === "builder" ? value : resolve(value))
    }
  }
  if (!REQUIRED_OPTIONS.every((name) => options.has(name))) throw new Error(usage())
  const parsed = Object.fromEntries(options)
  parsed["preflight-timeout-seconds"] ??= DEFAULT_PREFLIGHT_TIMEOUT_SECONDS
  parsed["build-timeout-seconds"] ??= DEFAULT_BUILD_TIMEOUT_SECONDS
  return parsed
}

function signalProcessTree(child, signal) {
  if (process.platform === "win32") {
    child.kill(signal)
    return
  }
  if (child.pid) process.kill(-child.pid, signal)
}

function runCommand(command, args, {
  env,
  timeoutMs,
  captureOutput = false,
  streamOutput = false,
  gracefulCancellation = false,
}) {
  return new Promise((resolvePromise) => {
    const child = spawn(command, args, {
      env,
      detached: process.platform !== "win32",
      stdio: captureOutput || streamOutput ? ["ignore", "pipe", "pipe"] : "inherit",
    })
    const stdout = []
    const stderr = []
    let stdoutBytes = 0
    let stderrBytes = 0
    let timedOut = false
    let outputLimitExceeded = false
    let spawnError
    let forceKillTimer
    let forcedCloseTimer
    let postExitDrainTimer
    let terminationStarted = false
    let interruptedSignal
    let parentSignalCount = 0
    let exitedStatus = null
    let exitedSignal = null
    let settled = false

    const finish = (status = exitedStatus, signal = exitedSignal, forceClose = false) => {
      if (settled) return
      settled = true
      clearTimeout(timeout)
      if (forceKillTimer) clearTimeout(forceKillTimer)
      if (forcedCloseTimer) clearTimeout(forcedCloseTimer)
      if (postExitDrainTimer) clearTimeout(postExitDrainTimer)
      for (const [parentSignal, handler] of parentSignalHandlers) {
        process.removeListener(parentSignal, handler)
      }
      if (forceClose) {
        child.stdout?.unpipe?.()
        child.stderr?.unpipe?.()
        child.stdout?.destroy?.()
        child.stderr?.destroy?.()
      }
      resolvePromise({
        status,
        signal,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
        timedOut,
        outputLimitExceeded,
        spawnError,
        interruptedSignal,
        stdioStalled: forceClose,
      })
    }

    const terminate = () => {
      if (terminationStarted) return
      terminationStarted = true
      try {
        // The CLI plugin shutdown path has no remote cancellation acknowledgment.
        // Signal the owned process group to stop the local Docker/Buildx clients.
        signalProcessTree(child, "SIGTERM")
      } catch {}
      forceKillTimer = setTimeout(() => {
        try { signalProcessTree(child, "SIGKILL") } catch {}
        forcedCloseTimer = setTimeout(() => finish(exitedStatus, exitedSignal, true), FORCED_OUTPUT_CLOSE_MS)
        forcedCloseTimer.unref?.()
      }, gracefulCancellation ? BUILD_CANCELLATION_GRACE_MS : 250)
      forceKillTimer.unref?.()
    }
    const timeout = setTimeout(() => {
      timedOut = true
      terminate()
    }, timeoutMs)

    const parentSignalHandlers = new Map()
    if (gracefulCancellation) {
      for (const signal of ["SIGINT", "SIGTERM"]) {
        const handler = () => {
          parentSignalCount += 1
          if (parentSignalCount > 1) {
            try { signalProcessTree(child, "SIGKILL") } catch {}
            forcedCloseTimer = setTimeout(() => finish(exitedStatus, exitedSignal, true), FORCED_OUTPUT_CLOSE_MS)
            forcedCloseTimer.unref?.()
            return
          }
          interruptedSignal = signal
          terminate()
        }
        parentSignalHandlers.set(signal, handler)
        process.on(signal, handler)
      }
    }

    if (captureOutput || streamOutput) {
      const collect = (chunks, label) => (chunk) => {
        if (label === "stdout") stdoutBytes += chunk.length
        else stderrBytes += chunk.length
        if (stdoutBytes + stderrBytes > MAX_DOCKER_OUTPUT_BYTES) {
          if (!outputLimitExceeded) {
            outputLimitExceeded = true
            terminate()
          }
          return
        }
        chunks.push(chunk)
      }
      if (streamOutput) {
        child.stdout.pipe(process.stdout)
        child.stderr.pipe(process.stderr)
      } else {
        child.stdout.on("data", collect(stdout, "stdout"))
        child.stderr.on("data", collect(stderr, "stderr"))
      }
    }
    child.on("error", (error) => { spawnError = error })
    child.on("exit", (status, signal) => {
      exitedStatus = status
      exitedSignal = signal
      if (!terminationStarted) {
        // A detached descendant can inherit a pipe and hold `close` open after
        // the command itself exits. Drain briefly, then close those descriptors.
        postExitDrainTimer = setTimeout(() => finish(status, signal, true), POST_EXIT_OUTPUT_DRAIN_MS)
      }
    })
    child.on("close", (status, signal) => finish(status, signal))
  })
}

function requiredSingleField(fields, name, description) {
  const values = fields.get(name) ?? []
  if (values.length !== 1 || !values[0]) throw new Error(`managed builder ${description} is missing or ambiguous`)
  return values[0]
}

function parseBuildxInspect(output, expectedBuilderName) {
  const topFields = new Map()
  const nodes = []
  let inNodes = false
  let currentNode

  for (const line of output.split(/\r?\n/)) {
    if (!inNodes) {
      const match = /^([A-Za-z][A-Za-z ]*):\s*(.*?)\s*$/.exec(line)
      if (!match) continue
      if (match[1] === "Nodes") {
        inNodes = true
        continue
      }
      const values = topFields.get(match[1]) ?? []
      values.push(match[2])
      topFields.set(match[1], values)
      continue
    }

    const match = /^(\s*)([A-Za-z][A-Za-z ]*):\s*(.*?)\s*$/.exec(line)
    if (!match) continue
    const indentation = match[1].length
    if (indentation !== 0 && indentation !== 2) continue
    if (match[2] === "Name") {
      currentNode = new Map()
      nodes.push(currentNode)
    }
    if (currentNode) {
      const values = currentNode.get(match[2]) ?? []
      values.push(match[3])
      currentNode.set(match[2], values)
    }
  }

  const name = requiredSingleField(topFields, "Name", "name")
  const driver = requiredSingleField(topFields, "Driver", "driver")
  if (name !== expectedBuilderName) throw new Error("managed builder identity does not match the requested builder")
  if (driver !== "docker-container") throw new Error("managed builder must use the docker-container driver")
  if (nodes.length === 0) throw new Error("managed builder has no inspectable nodes")

  return {
    name,
    nodes: nodes.map((node) => {
      const nodeName = requiredSingleField(node, "Name", "node name")
      const endpoint = requiredSingleField(node, "Endpoint", "node endpoint")
      const status = requiredSingleField(node, "Status", "node status")
      if (!NODE_NAME_PATTERN.test(nodeName)) throw new Error("managed builder node name is malformed")
      if (status !== "running") throw new Error("managed builder nodes must already be running")
      return { name: nodeName, endpoint }
    }),
  }
}

function dockerEndpointArguments(endpoint) {
  if (/^(unix|tcp|ssh):\/\/[^\s]+$/.test(endpoint)) return ["--host", endpoint]
  if (/^[A-Za-z0-9][A-Za-z0-9_.-]{0,255}$/.test(endpoint)) return ["--context", endpoint]
  throw new Error("managed builder node endpoint is unsupported")
}

function dockerInspectObject(output) {
  let parsed
  try {
    parsed = JSON.parse(output.trim())
  } catch {
    throw new Error("managed builder container inspect output is malformed")
  }
  if (Array.isArray(parsed)) {
    if (parsed.length !== 1) throw new Error("managed builder container inspect output is ambiguous")
    parsed = parsed[0]
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("managed builder container inspect output is malformed")
  }
  return parsed
}

function nonnegativeInteger(value, name) {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error(`managed builder container ${name} is malformed`)
  return value
}

function cpusetCpuCount(value) {
  if (typeof value !== "string") throw new Error("managed builder container CPU set is malformed")
  if (value === "") return 0
  const seen = new Set()
  for (const range of value.split(",")) {
    const match = /^(0|[1-9][0-9]*)(?:-(0|[1-9][0-9]*))?$/.exec(range)
    if (!match) throw new Error("managed builder container CPU set is malformed")
    const start = Number(match[1])
    const end = Number(match[2] ?? match[1])
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || end < start || end - start > MAX_BUILDKIT_CPUS) {
      throw new Error("managed builder container CPU set is malformed")
    }
    for (let cpu = start; cpu <= end; cpu++) {
      if (seen.has(cpu)) throw new Error("managed builder container CPU set is malformed")
      seen.add(cpu)
      if (seen.size > MAX_BUILDKIT_CPUS) throw new Error("managed builder CPU cap exceeds the release-build limit")
    }
  }
  return seen.size
}

function validateBuilderContainer(container, node) {
  if (typeof container.Id !== "string" || !/^[a-f0-9]{64}$/.test(container.Id)) {
    throw new Error("managed builder container identity is malformed")
  }
  if (container.Name !== `/buildx_buildkit_${node.name}`) {
    throw new Error("managed builder container does not match its inspected node")
  }
  if (!container.State || container.State.Running !== true) {
    throw new Error("managed builder containers must already be running")
  }
  if (typeof container.State.StartedAt !== "string" || !Number.isFinite(Date.parse(container.State.StartedAt))) {
    throw new Error("managed builder container start identity is malformed")
  }
  const host = container.HostConfig
  if (!host || typeof host !== "object" || Array.isArray(host)) {
    throw new Error("managed builder container resource configuration is missing")
  }
  const pidsLimit = host.PidsLimit
  if (!Number.isSafeInteger(pidsLimit) || pidsLimit <= 0) {
    throw new Error("managed builder requires an explicit finite positive PID limit")
  }
  if (pidsLimit > MAX_BUILDKIT_PIDS) {
    throw new Error("managed builder PID cap exceeds the release-build limit")
  }

  const nanoCpus = nonnegativeInteger(host.NanoCpus, "NanoCpus")
  const cpuQuota = nonnegativeInteger(host.CpuQuota, "CpuQuota")
  const cpuPeriod = nonnegativeInteger(host.CpuPeriod, "CpuPeriod")
  const memory = nonnegativeInteger(host.Memory, "Memory")
  const memorySwap = nonnegativeInteger(host.MemorySwap, "MemorySwap")
  const cpuSet = cpusetCpuCount(host.CpusetCpus)
  const cpuCaps = []

  if (nanoCpus > 0) cpuCaps.push(nanoCpus / 1_000_000_000)
  if (cpuQuota > 0 || cpuPeriod > 0) {
    if (cpuQuota === 0 || cpuPeriod === 0) throw new Error("managed builder CPU quota configuration is malformed")
    cpuCaps.push(cpuQuota / cpuPeriod)
  }
  if (cpuSet > 0) cpuCaps.push(cpuSet)
  if (cpuCaps.length === 0) throw new Error("managed builder has no hard CPU limit")
  const effectiveCpus = Math.min(...cpuCaps)
  if (effectiveCpus < 1 || effectiveCpus > MAX_BUILDKIT_CPUS) {
    throw new Error("managed builder CPU cap is outside the release-build limit")
  }

  if (memory < MIN_BUILDKIT_MEMORY_BYTES || memory > MAX_BUILDKIT_MEMORY_BYTES) {
    throw new Error("managed builder memory cap is outside the release-build limit")
  }
  if (memorySwap === 0 || memorySwap < memory) {
    throw new Error("managed builder requires an explicit finite memory-swap cap")
  }
  const memoryWithSwap = memorySwap
  if (memoryWithSwap > MAX_BUILDKIT_MEMORY_WITH_SWAP_BYTES) {
    throw new Error("managed builder memory-plus-swap cap exceeds the release-build limit")
  }
  return {
    name: node.name,
    endpoint: node.endpoint,
    containerId: container.Id,
    startedAt: container.State.StartedAt,
    pidsLimit,
    effectiveCpus,
    memoryBytes: memory,
    memoryWithSwapBytes: memoryWithSwap,
  }
}

async function dockerOutput(args, options, label, deadlineMs) {
  const timeoutMs = deadlineMs - Date.now()
  if (timeoutMs <= 0) throw new Error(`${label} timed out`)
  const result = await runCommand("docker", args, {
    env: options.dockerEnvironment,
    timeoutMs,
    captureOutput: true,
  })
  if (result.timedOut) throw new Error(`${label} timed out`)
  if (result.outputLimitExceeded) throw new Error(`${label} output exceeded the inspection limit`)
  if (result.stdioStalled) throw new Error(`${label} output did not close after its process exited`)
  if (result.spawnError || result.status !== 0) throw new Error(`${label} failed`)
  return result.stdout
}

async function verifyManagedBuilder(options) {
  const deadlineMs = options.preflightDeadlineMs
  const builderOutput = await dockerOutput(
    ["buildx", "inspect", options.builder],
    options,
    "docker buildx inspect",
    deadlineMs,
  )
  const builder = parseBuildxInspect(builderOutput, options.builder)
  let inspectedBytes = Buffer.byteLength(builderOutput)
  const nodes = []
  for (const node of builder.nodes) {
    const inspectArgs = [
      ...dockerEndpointArguments(node.endpoint),
      "inspect", "--type", "container", "--format", "{{json .}}", `buildx_buildkit_${node.name}`,
    ]
    const containerOutput = await dockerOutput(inspectArgs, options, "docker inspect", deadlineMs)
    inspectedBytes += Buffer.byteLength(containerOutput)
    if (inspectedBytes > MAX_DOCKER_OUTPUT_BYTES) {
      throw new Error("managed builder inspection output exceeded the aggregate limit")
    }
    nodes.push(validateBuilderContainer(dockerInspectObject(containerOutput), node))
  }
  return { name: builder.name, nodes }
}

async function listManagedBuildHistory(options, deadlineMs) {
  const output = await dockerOutput(
    ["buildx", "history", "ls", "--builder", options.builder, "--format", "json", "--no-trunc"],
    options,
    "docker buildx history ls",
    deadlineMs,
  )
  return parseBuildHistoryList(output)
}

async function inspectManagedBuildHistory(options, reference, deadlineMs) {
  const output = await dockerOutput(
    ["buildx", "history", "inspect", "--builder", options.builder, "--format", "json", reference],
    options,
    "docker buildx history inspect",
    deadlineMs,
  )
  try {
    const record = JSON.parse(output)
    if (!record || typeof record !== "object" || Array.isArray(record)) throw new Error()
    return record
  } catch {
    throw new Error("docker buildx history inspect output is malformed")
  }
}

async function readBuildReference(metadataPath, builder) {
  try {
    const metadata = JSON.parse(await readFile(metadataPath, "utf8"))
    return validateBuildMetadata(metadata, builder.name, builder.nodes)
  } catch {
    return null
  }
}

async function reconcileInvocation(barrier, builder, options) {
  if (await hashManagedReleaseSource(barrier.sourceDirectory) !== barrier.sourceDigest) {
    return { settled: false, reason: "retained source does not match its invocation digest" }
  }
  const deadlineMs = Date.now() + BUILD_HISTORY_READ_TIMEOUT_MS
  return reconcileManagedReleaseBuild({
    barrier,
    currentBuilderFingerprint: builder,
    historyList: () => listManagedBuildHistory(options, deadlineMs),
    historyInspect: (reference) => inspectManagedBuildHistory(options, reference, deadlineMs),
  })
}

function assertBuilderUnchanged(before, after) {
  if (JSON.stringify(before) !== JSON.stringify(after)) {
    throw new Error("managed builder identity or resource limits changed during the release build")
  }
}

function git(repository, args, encoding = "utf8") {
  const result = spawnSync("git", args, {
    cwd: repository,
    encoding,
    maxBuffer: 512 * 1024 * 1024,
    env: gitEnvironment(),
  })
  if (result.status !== 0) throw new Error(`git ${args[0]} failed: ${result.stderr.toString().trim()}`)
  return result.stdout
}

function gitEnvironment() {
  return {
    PATH: process.env.PATH ?? "/usr/bin:/bin",
    HOME: "/nonexistent",
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_GLOBAL: "/dev/null",
    GIT_NO_REPLACE_OBJECTS: "1",
    LC_ALL: "C",
  }
}

async function materializeGitTree(repository, commit, destination) {
  const listing = git(repository, ["ls-tree", "-r", "-z", "--full-tree", commit], null)
  for (const record of listing.toString("utf8").split("\0").filter(Boolean)) {
    const match = /^(100644|100755) blob ([a-f0-9]{40})\t([^\0]+)$/.exec(record)
    if (!match || match[3].startsWith("/") || match[3].split("/").includes("..")) {
      throw new Error("source commit contains an unsupported Git object")
    }
    const path = join(destination, match[3])
    await mkdir(dirname(path), { recursive: true, mode: 0o700 })
    await writeFile(path, git(repository, ["cat-file", "blob", match[2]], null), {
      flag: "wx",
      mode: match[1] === "100755" ? 0o755 : 0o644,
    })
  }
}

async function requirePrivateKey(path) {
  const metadata = await lstat(path)
  if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.size > 64 * 1024) {
    throw new Error("builder signing key must be a bounded regular file")
  }
  if (process.platform !== "win32" && (metadata.mode & 0o077) !== 0) {
    throw new Error("builder signing key must not be readable by group or other users")
  }
  const key = createPrivateKey({ key: await readFile(path), format: "pem", type: "pkcs8" })
  if (key.asymmetricKeyType !== "ed25519") throw new Error("builder signing key must be Ed25519")
  return key
}

function rawPublicKey(privateKey) {
  const der = createPublicKey(privateKey).export({ format: "der", type: "spki" })
  if (!der.subarray(0, ED25519_SPKI_PREFIX.length).equals(ED25519_SPKI_PREFIX)) {
    throw new Error("builder signing key produced an unsupported public key")
  }
  return der.subarray(ED25519_SPKI_PREFIX.length)
}

async function sha256File(path) {
  return `sha256:${createHash("sha256").update(await readFile(path)).digest("hex")}`
}

async function build(options) {
  if (!/^[a-f0-9]{40}$/.test(options["source-commit"])) {
    throw new Error("source commit must be a full lowercase Git commit ID")
  }
  const repository = options["source-repository"]
  const repositoryMetadata = await lstat(repository)
  if (repositoryMetadata.isSymbolicLink() || !repositoryMetadata.isDirectory()) {
    throw new Error("source repository must be a directory, not a symlink")
  }
  const commit = git(repository, ["rev-parse", "--verify", `${options["source-commit"]}^{commit}`]).trim()
  if (commit !== options["source-commit"]) throw new Error("source commit did not resolve exactly")
  const tree = git(repository, ["rev-parse", `${commit}^{tree}`]).trim()
  const signingKey = await requirePrivateKey(options["builder-signing-key"])

  const dockerEnvironment = Object.fromEntries(
    ["PATH", "HOME", "DOCKER_HOST"].flatMap((name) => process.env[name] ? [[name, process.env[name]]] : []),
  )
  dockerEnvironment.LC_ALL = "C"
  dockerEnvironment.NO_COLOR = "1"
  const verifiedBuilder = await verifyManagedBuilder({
    ...options,
    dockerEnvironment,
    preflightDeadlineMs: Date.now() + options["preflight-timeout-seconds"] * 1000,
  })
  const lease = await acquireManagedReleaseBuilderLease({ builderName: verifiedBuilder.name })
  let priorBarrier = null
  let invocationBarrier = null
  let invocationBarrierWritten = false
  let invocationSettled = false
  let preserveInvocationArtifacts = false
  let runDirectory
  let pending
  try {
    if (!isStateDirectoryExternal(lease.stateDirectory, repository)) {
      throw new Error("managed release state must be outside the source repository")
    }
    priorBarrier = await lease.readBarrier()
    if (priorBarrier) {
      const priorSettlement = await reconcileInvocation(priorBarrier, verifiedBuilder, {
        ...options,
        dockerEnvironment,
      })
      if (!priorSettlement.settled) {
        throw new Error(`managed release builder has an unresolved prior build: ${priorSettlement.reason}`)
      }
      await lease.removeBarrier()
      await lease.removeRunArtifacts(priorBarrier)
      priorBarrier = null
    }

    if (await stat(options.output).then(() => true, () => false)) throw new Error("output must not exist")
    await mkdir(dirname(options.output), { recursive: true })
    const invocationId = randomUUID()
    runDirectory = await lease.createRunDirectory(invocationId)
    const source = join(runDirectory, "source")
    const metadataPath = join(runDirectory, "build-metadata.json")
    const exported = join(runDirectory, "artifacts")
    pending = join(dirname(options.output), `.new-${basename(options.output)}-${invocationId}`)
    await mkdir(source, { mode: 0o700 })
    await mkdir(pending, { mode: 0o755 })
    await materializeGitTree(repository, commit, source)
    const sourceDigest = await hashManagedReleaseSource(source)
    const historyBaseline = await listManagedBuildHistory({ ...options, dockerEnvironment }, Date.now() + options["preflight-timeout-seconds"] * 1000)
    invocationBarrier = {
      schemaVersion: 1,
      invocationId,
      builderName: verifiedBuilder.name,
      builderFingerprint: verifiedBuilder,
      sourceCommit: commit,
      sourceTree: tree,
      sourceDigest,
      sourceDirectory: source,
      runDirectory,
      outputPath: options.output,
      pendingDirectory: pending,
      startedAt: new Date().toISOString(),
      historyBaseline,
      buildRef: null,
    }
    invocationBarrierWritten = true
    await lease.writeBarrier(invocationBarrier)

    const dockerBuild = await runCommand(
      "docker",
      [
        "buildx", "build", "--builder", verifiedBuilder.name,
        "--pull", "--platform", "linux/amd64", "--target", ARTIFACT_STAGE,
        "--metadata-file", metadataPath,
        "--file", join(source, BUILDER_DOCKERFILE),
        "--output", `type=local,dest=${exported}`,
        source,
      ],
      {
        env: dockerEnvironment,
        timeoutMs: options["build-timeout-seconds"] * 1000,
        streamOutput: true,
        gracefulCancellation: true,
      },
    )

    const metadataReference = await readBuildReference(metadataPath, verifiedBuilder)
    if (metadataReference) {
      invocationBarrier = { ...invocationBarrier, buildRef: metadataReference }
      await lease.writeBarrier(invocationBarrier)
    }
    let settlement
    try {
      settlement = await reconcileInvocation(invocationBarrier, verifiedBuilder, {
        ...options,
        dockerEnvironment,
      })
    } catch (error) {
      settlement = { settled: false, reason: error instanceof Error ? error.message : String(error) }
    }
    if (!settlement.settled) {
      preserveInvocationArtifacts = true
      throw new Error(`managed release build settlement is unresolved: ${settlement.reason}`)
    }
    invocationSettled = true
    if (invocationBarrier.buildRef === null) {
      invocationBarrier = { ...invocationBarrier, buildRef: settlement.buildRef }
      await lease.writeBarrier(invocationBarrier)
    }

    if (dockerBuild.timedOut) throw new Error("locked managed release build timed out after remote settlement")
    if (dockerBuild.interruptedSignal) {
      throw new Error(`locked managed release build interrupted by ${dockerBuild.interruptedSignal}`)
    }
    if (dockerBuild.spawnError) throw new Error("locked managed release artifact export could not start")
    if (dockerBuild.stdioStalled) throw new Error("locked managed release build output did not close")
    if (dockerBuild.status !== 0) {
      throw new Error(`locked managed release artifact export failed with status ${dockerBuild.status ?? "unknown"}`)
    }
    if (!new Set(["completed", "success"]).has(String(settlement.status).toLowerCase())) {
      throw new Error("managed release Buildx history did not report a successful build")
    }

    const builderAfterBuild = await verifyManagedBuilder({
      ...options,
      dockerEnvironment,
      preflightDeadlineMs: Date.now() + options["preflight-timeout-seconds"] * 1000,
    })
    assertBuilderUnchanged(verifiedBuilder, builderAfterBuild)
    const exportMetadata = await lstat(exported)
    if (exportMetadata.isSymbolicLink() || !exportMetadata.isDirectory()) {
      throw new Error("managed release artifact export is not a directory")
    }
    const exportedNames = (await readdir(exported)).sort()
    if (exportedNames.length !== ARTIFACTS.length || exportedNames.some((name, index) => name !== ARTIFACTS[index])) {
      throw new Error("managed release artifact export contains an unexpected file set")
    }
    for (const name of ARTIFACTS) {
      const exportedBinary = join(exported, name)
      const metadata = await lstat(exportedBinary)
      if (metadata.isSymbolicLink() || !metadata.isFile()) throw new Error(`build did not export regular ${name}`)
      const sourceBinary = join(pending, name)
      await copyFile(exportedBinary, sourceBinary, constants.COPYFILE_EXCL)
      await chmod(sourceBinary, 0o755)
    }
    const attestation = Buffer.from(JSON.stringify({
      schemaVersion: 1,
      sourceCommit: commit,
      sourceTree: tree,
      target: BUILD_TARGET,
      artifacts: [
        { name: "chariox-kernel", sha256: await sha256File(join(pending, "chariox-kernel")) },
        { name: "chariox-managed-bootstrap", sha256: await sha256File(join(pending, "chariox-managed-bootstrap")) },
        { name: "chariox-relay", sha256: await sha256File(join(pending, "chariox-relay")) },
      ],
    }))
    await writeFile(join(pending, "build-attestation.json"), attestation, { flag: "wx", mode: 0o644 })
    await writeFile(
      join(pending, "build-attestation.sig"),
      sign(null, attestation, signingKey).toString("base64"),
      { flag: "wx", mode: 0o644 },
    )
    await writeFile(join(pending, "builder-public-key"), rawPublicKey(signingKey).toString("base64"), {
      flag: "wx",
      mode: 0o644,
    })
    await rename(pending, options.output)
  } finally {
    try {
      if (!preserveInvocationArtifacts) {
        if (invocationBarrierWritten && invocationSettled) {
          await lease.removeBarrier()
          await lease.removeRunArtifacts(invocationBarrier)
        } else if (!invocationBarrierWritten) {
          if (pending) await rm(pending, { recursive: true, force: true })
          if (runDirectory) await rm(runDirectory, { recursive: true, force: true })
        }
      }
    } finally {
      await lease.release()
    }
  }
}

try {
  await build(parseOptions(process.argv.slice(2)))
} catch (error) {
  process.stderr.write(`${basename(process.argv[1])}: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exitCode = 1
}
