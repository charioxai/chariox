import { access, mkdir, rm, stat, statfs, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { spawn } from "node:child_process"
import { once } from "node:events"
import { fileURLToPath } from "node:url"

import {
  BROWSER_COMPUTER_FUNCTIONAL_EVIDENCE_SCHEMA,
  browserComputerFunctionalCases,
  validateBrowserComputerFunctionalEvidence,
} from "./browser-computer-functional-contract.mjs"
import { sanitizeDrillMetadata, looksLikeDrillSecretValue } from "./drill-secrets.mjs"
import { validateDrillDurationMatchesTimestamps, validateDrillTimestampOrder } from "./drill-time.mjs"

export const BROWSER_COMPUTER_RED_CAP_EVIDENCE_SCHEMA = "chariox.browser_computer.red_cap_evidence.v1"
export const AUTHORITATIVE_MAIN_SHA = "9f5ec7d6e8c8e9300e23bdbb849debc479387a84"
export const RED_CAP_MAX_GROWTH_BYTES = 300 * 1024 * 1024
export const RED_CAP_MIN_FREE_BYTES = 28 * 1024 ** 3
export const RED_CAP_DEFAULT_TIMEOUT_MS = 30_000
export const RED_CAP_MAX_RUN_MS = 12 * 60 * 1000
export const RED_CAP_DEFAULT_IMAGE = "chariox-slice-linux:0.1.0"

const CORE_CASE_IDS = Object.freeze(browserComputerFunctionalCases().map((item) => item.id))
const CASE_DEFINITIONS = new Map(browserComputerFunctionalCases().map((item) => [item.id, item]))
const SAFE_TEMP_PREFIX = "chariox-m0-red-cap-"

export const RED_CAP_FAULTS = Object.freeze([
  Object.freeze({
    id: "relay-loss",
    caseId: "fault.relay-loss",
    offsetMs: 1_000,
    expectedOutcome: "fail",
    signal: "relay disconnect must be bounded and authoritative",
  }),
  Object.freeze({
    id: "controller-crash",
    caseId: "fault.controller-crash",
    offsetMs: 2_000,
    expectedOutcome: "fail",
    signal: "a persistent Browser Controller is not on the current main path",
  }),
  Object.freeze({
    id: "streamer-crash",
    caseId: "fault.streamer-crash",
    offsetMs: 3_000,
    expectedOutcome: "pass",
    signal: "noVNC must degrade and restart without blocking the browser helper",
  }),
  Object.freeze({
    id: "browser-crash",
    caseId: "fault.browser-crash",
    offsetMs: 4_000,
    expectedOutcome: "pass",
    signal: "Chromium loss must invalidate stale targets before recovery",
  }),
  Object.freeze({
    id: "stale-endpoint",
    caseId: "fault.stale-endpoint",
    offsetMs: 5_000,
    expectedOutcome: "pass",
    signal: "a stale noVNC endpoint must be rejected and retry remain bounded",
  }),
  Object.freeze({
    id: "queue-saturation",
    caseId: "fault.queue-saturation",
    offsetMs: 6_000,
    expectedOutcome: "fail",
    signal: "kernel-owned interaction queue and bounded backpressure are not on the current main path",
  }),
  Object.freeze({
    id: "memory-pressure",
    caseId: "fault.memory-pressure",
    offsetMs: 7_000,
    expectedOutcome: "fail",
    signal: "admission control before OOM is a later product milestone",
  }),
])

const KNOWN_RED_CASES = new Map([
  ["browser.structures", "the current one-shot CDP helper does not expose nested-frame or shadow-root target authority"],
  ["shared.single-environment", "M1.1 Room Environment authority is intentionally outside this lane"],
  ["shared.takeover", "M1.1 Room takeover authority is intentionally outside this lane"],
  ["shared.concurrency", "M1.1 Room interaction serialization is intentionally outside this lane"],
  ["fault.relay-loss", "the local red cap does not invent a Room relay authority"],
  ["fault.controller-crash", "the current stack uses one-shot CDP helpers, not a long-running controller"],
  ["fault.queue-saturation", "the current stack has no kernel-owned browser/computer queue in this lane"],
  ["fault.memory-pressure", "the current stack has no M0 admission controller in this lane"],
])

const FILES_REQUIRED_BY_STACK = Object.freeze([
  "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
  "apps/kernel/slice-linux-docker/docker/slice-screen.sh",
  "apps/kernel/slice-linux-docker/docker/browser-cdp.mjs",
])

export const RED_CAP_PROFILES = Object.freeze({
  mac: Object.freeze({
    id: "mac-docker-desktop",
    platform: "darwin",
    topology: "local-docker-desktop",
    display: "novnc",
    browserControl: "one-shot-cdp",
  }),
  "linux-docker": Object.freeze({
    id: "linux-docker",
    platform: "linux",
    topology: "local-docker",
    display: "novnc",
    browserControl: "one-shot-cdp",
  }),
})

export function resolveRedCapProfile(value = "auto", platform = process.platform) {
  if (value === "auto") return platform === "darwin" ? RED_CAP_PROFILES.mac : RED_CAP_PROFILES["linux-docker"]
  const profile = RED_CAP_PROFILES[value]
  if (!profile) throw new Error(`unknown browser/computer red-cap profile ${JSON.stringify(value)}`)
  return profile
}

export function redCapCaseIds() {
  return [...CORE_CASE_IDS]
}

export function redCapFaults() {
  return RED_CAP_FAULTS.map((fault) => ({ ...fault }))
}

export function knownRedCapFailure(caseId) {
  return KNOWN_RED_CASES.get(caseId) ?? null
}

export function assertSafeExternalPath(target, { repoRoot, kind = "path", disposable = false } = {}) {
  if (typeof target !== "string" || target.trim() === "") throw new Error(`${kind} is required`)
  if (!path.isAbsolute(target)) throw new Error(`${kind} must be absolute`)
  const resolved = path.resolve(target)
  if (resolved === path.parse(resolved).root) throw new Error(`${kind} may not be the filesystem root`)
  if (repoRoot) {
    const relative = path.relative(path.resolve(repoRoot), resolved)
    if (relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative))) {
      throw new Error(`${kind} must stay outside the repository: ${resolved}`)
    }
  }
  if (disposable && !path.basename(resolved).startsWith(SAFE_TEMP_PREFIX)) {
    throw new Error(`${kind} must use the disposable ${SAFE_TEMP_PREFIX} prefix`)
  }
  return resolved
}

export async function createRedCapRunPaths({ repoRoot, evidenceRoot, tempRoot } = {}) {
  const runId = `m0-red-cap-${process.pid}-${Date.now().toString(36)}`
  const defaultEvidenceRoot = path.join(
    os.homedir(),
    ".codex",
    "evidence",
    "browser-computer-use",
    "m0",
    runId,
  )
  const artifactRoot = assertSafeExternalPath(evidenceRoot ?? defaultEvidenceRoot, {
    repoRoot,
    kind: "evidence root",
  })
  const ownedTempRoot = tempRoot
    ? await requireFreshDisposableRoot(tempRoot, repoRoot)
    : await import("node:fs/promises").then(({ mkdtemp }) => mkdtemp(path.join(os.tmpdir(), SAFE_TEMP_PREFIX)))
  await mkdir(artifactRoot, { recursive: true, mode: 0o700 })
  await mkdir(ownedTempRoot, { recursive: true, mode: 0o700 })
  const charioxHome = path.join(ownedTempRoot, "chariox-home")
  const workspace = path.join(ownedTempRoot, "workspace")
  await mkdir(charioxHome, { recursive: true, mode: 0o700 })
  await mkdir(workspace, { recursive: true, mode: 0o700 })
  const ownership = {
    schema: "chariox.browser_computer.red_cap_ownership.v1",
    runId,
    tempRoot: ownedTempRoot,
    charioxHome,
    workspace,
    createdAt: new Date().toISOString(),
  }
  await writeFile(path.join(ownedTempRoot, "ownership.json"), `${JSON.stringify(ownership, null, 2)}\n`, { mode: 0o600 })
  return { runId, artifactRoot, tempRoot: ownedTempRoot, charioxHome, workspace, ownership }
}

export function assertOwnedTempRoot(tempRoot, repoRoot) {
  const resolved = assertSafeExternalPath(tempRoot, { repoRoot, kind: "temporary root", disposable: true })
  if (path.basename(resolved).startsWith(SAFE_TEMP_PREFIX) !== true) {
    throw new Error(`temporary root is not an owned red-cap root: ${resolved}`)
  }
  return resolved
}

export function buildRedCapPorts(overrides = {}) {
  const names = ["novnc", "vnc", "kernel", "relay", "mcp"]
  const ports = {}
  for (const [index, name] of names.entries()) {
    const value = overrides[name] ?? (60_80 + index * 37)
    if (!Number.isSafeInteger(Number(value)) || Number(value) < 1024 || Number(value) > 65_535) {
      throw new Error(`red-cap port ${name} must be an integer between 1024 and 65535`)
    }
    ports[name] = Number(value)
  }
  return ports
}

export function buildRedCapCommandPlan({
  profile,
  repoRoot,
  runRoot,
  runId,
  image = RED_CAP_DEFAULT_IMAGE,
  ports = buildRedCapPorts(),
  containerName = `chariox-m0-red-cap-${process.pid}`,
  volumeName = `${containerName}-home`,
} = {}) {
  const resolvedProfile = typeof profile === "string" ? resolveRedCapProfile(profile) : profile
  if (!resolvedProfile?.id) throw new Error("red-cap profile is required")
  const script = path.join(repoRoot, "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh")
  const screen = "/opt/chariox-slice/slice-screen.sh"
  const cdp = "/opt/chariox-slice/browser-cdp.mjs"
  const env = {
    CHARIOX_HOME: path.join(runRoot, "chariox-home"),
    CHARIOX_SLICE_NAME: containerName,
    CHARIOX_SLICE_ID: `${runId}-slice`,
    CHARIOX_SLICE_DOCKER_IMAGE: image,
    CHARIOX_SLICE_BASE_IMAGE: image,
    CHARIOX_SLICE_BUILD_IMAGE: "never",
    CHARIOX_SLICE_START_DESKTOP: "1",
    CHARIOX_SLICE_START_PROVIDER_SERVERS: "0",
    CHARIOX_SLICE_START_RUNTIME: "0",
    CHARIOX_SLICE_WORKSPACE: path.join(runRoot, "workspace"),
    CHARIOX_SLICE_WORKSPACE_SOURCE: path.join(runRoot, "workspace"),
    CHARIOX_SLICE_HOME_VOLUME: volumeName,
    CHARIOX_SLICE_NOVNC_PORT: String(ports.novnc),
    CHARIOX_SLICE_VNC_PORT: String(ports.vnc),
    CHARIOX_SLICE_KERNEL_PORT: String(ports.kernel),
    CHARIOX_SLICE_RELAY_PORT: String(ports.relay),
    CHARIOX_SLICE_MCP_PORT: String(ports.mcp),
    CHARIOX_SLICE_DOCKER_MEMORY: "768m",
    CHARIOX_SLICE_DOCKER_CPUS: "2",
    CHARIOX_SLICE_SCREEN_GEOMETRY: "1280x800x24",
    CHARIOX_SLICE_RELAY_TOKEN: "m0-red-cap-local",
  }
  return {
    profile: resolvedProfile,
    containerName,
    volumeName,
    image,
    ports,
    env,
    provision: {
      command: "/bin/bash",
      args: [script, "provision"],
      cwd: repoRoot,
      env,
    },
    docker: (args, options = {}) => ({
      command: "docker",
      args: ["exec", ...(options.interactive ? ["-i"] : []), "-u", options.user ?? "slice", containerName, ...args],
      cwd: repoRoot,
      env: options.env ?? {},
    }),
    screen: (args, options = {}) => ({
      command: "docker",
      args: ["exec", ...(options.interactive ? ["-i"] : []), "-u", options.user ?? "slice", containerName, screen, ...args],
      cwd: repoRoot,
      env: options.env ?? {},
    }),
    cdp: (args, options = {}) => ({
      command: "docker",
      args: ["exec", ...(options.interactive ? ["-i"] : []), "-u", options.user ?? "slice", containerName, "node", cdp, ...args],
      cwd: repoRoot,
      env: options.env ?? {},
    }),
    destroy: {
      command: "docker",
      args: ["rm", "--force", containerName],
      cwd: repoRoot,
      env: {},
    },
    removeVolume: {
      command: "docker",
      args: ["volume", "rm", "--force", volumeName],
      cwd: repoRoot,
      env: {},
    },
  }
}

export function buildFaultCommand(plan, faultId, { staleUrl } = {}) {
  const fault = RED_CAP_FAULTS.find((item) => item.id === faultId)
  if (!fault) throw new Error(`unknown red-cap fault ${JSON.stringify(faultId)}`)
  if (faultId === "stale-endpoint") {
    if (!staleUrl) throw new Error("stale endpoint fault requires staleUrl")
    return {
      ...fault,
      command: "node",
      args: ["--input-type=module", "-e", "const r=await fetch(process.argv[1]).catch(() => null); if (r?.ok) process.exit(1)", staleUrl],
      cwd: plan.provision.cwd,
    }
  }
  const processPattern = {
    "relay-loss": "chariox-relay",
    "controller-crash": "browser-cdp.mjs",
    "streamer-crash": "websockify.*6080",
    "browser-crash": "chromium.*chariox-slice-chromium",
  }[faultId]
  if (processPattern) {
    return {
      ...fault,
      ...plan.docker(["pkill", "-TERM", "-f", processPattern]),
      args: ["exec", "-u", "slice", plan.containerName, "pkill", "-TERM", "-f", processPattern],
    }
  }
  if (faultId === "queue-saturation") {
    return {
      ...fault,
      ...plan.screen(["status"]),
      args: ["exec", "-u", "slice", plan.containerName, "sh", "-lc", "for i in $(seq 1 12); do /opt/chariox-slice/browser-cdp.mjs text >/dev/null 2>&1 & done; wait"],
    }
  }
  if (faultId === "memory-pressure") {
    return {
      ...fault,
      ...plan.docker(["node", "--max-old-space-size=32", "-e", "Buffer.alloc(24 * 1024 * 1024); setTimeout(() => {}, 250)"]),
    }
  }
  throw new Error(`fault ${faultId} has no command construction`)
}

export async function spawnBounded(command, args = [], {
  cwd,
  env,
  timeoutMs = RED_CAP_DEFAULT_TIMEOUT_MS,
  input,
  label = command,
  registry,
} = {}) {
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0 || timeoutMs > RED_CAP_MAX_RUN_MS) {
    throw new Error(`${label} timeoutMs must be a positive bounded integer`)
  }
  const startedAt = new Date().toISOString()
  const startedMonoNs = process.hrtime.bigint()
  const child = spawn(command, args, {
    cwd,
    env: env ? { ...process.env, ...env } : process.env,
    detached: process.platform !== "win32",
    stdio: [input === undefined ? "ignore" : "pipe", "pipe", "pipe"],
  })
  const record = {
    label,
    command,
    args: [...args],
    cwd: cwd ?? process.cwd(),
    pid: child.pid ?? null,
    startedAt,
    startedMonoNs: startedMonoNs.toString(),
    timeoutMs,
    timedOut: false,
    stdout: "",
    stderr: "",
  }
  registry?.active?.add(child)
  registry?.records?.push(record)
  child.stdout?.on("data", (chunk) => { record.stdout += chunk.toString() })
  child.stderr?.on("data", (chunk) => { record.stderr += chunk.toString() })
  if (input !== undefined) {
    child.stdin.end(input)
  }
  let timer = null
  let settled = false
  const result = await new Promise((resolve) => {
    const finish = (value) => {
      if (settled) return
      settled = true
      if (timer) clearTimeout(timer)
      resolve(value)
    }
    child.once("error", (error) => finish({ code: null, signal: null, spawnError: error.message }))
    child.once("close", (code, signal) => finish({ code, signal }))
    timer = setTimeout(() => {
      record.timedOut = true
      killOwnedChild(child)
    }, timeoutMs)
  })
  const completedMonoNs = process.hrtime.bigint()
  record.completedAt = new Date().toISOString()
  record.completedMonoNs = completedMonoNs.toString()
  record.durationMs = Number(completedMonoNs - startedMonoNs) / 1_000_000
  record.code = result.code
  record.signal = result.signal
  record.spawnError = result.spawnError
  registry?.active?.delete(child)
  return { ...record }
}

export function createProcessRegistry() {
  return { active: new Set(), records: [] }
}

export async function stopOwnedProcesses(registry) {
  const children = [...(registry?.active ?? [])]
  for (const child of children) killOwnedChild(child)
  if (children.length > 0) await Promise.allSettled(children.map((child) => once(child, "close").catch(() => undefined)))
  return children.map((child) => child.pid ?? null).filter((pid) => pid !== null)
}

function killOwnedChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  try {
    if (child.pid && process.platform !== "win32") process.kill(-child.pid, "SIGTERM")
    else child.kill("SIGTERM")
  } catch {}
  setTimeout(() => {
    try {
      if (child.exitCode === null && child.signalCode === null) {
        if (child.pid && process.platform !== "win32") process.kill(-child.pid, "SIGKILL")
        else child.kill("SIGKILL")
      }
    } catch {}
  }, 1_000).unref()
}

export async function collectRedCapResourceSample({ rootPath, runCommand, containerName } = {}) {
  const disk = await statfs(rootPath)
  const processList = runCommand
    ? await runCommand("ps", ["-e", "-o", "pid=,comm="], { timeoutMs: 5_000, label: "resource-process-list" })
    : null
  let docker = null
  if (runCommand && containerName) {
    const stats = await runCommand("docker", ["stats", "--no-stream", "--format", "{{json .}}", containerName], {
      timeoutMs: 10_000,
      label: "resource-docker-stats",
    })
    docker = { code: stats.code, stdout: stats.stdout, stderr: stats.stderr }
  }
  return {
    capturedAt: new Date().toISOString(),
    monotonicNs: process.hrtime.bigint().toString(),
    filesystem: path.resolve(rootPath),
    memory: {
      totalBytes: os.totalmem(),
      availableBytes: os.freemem(),
      processRssBytes: process.memoryUsage().rss,
    },
    disk: {
      totalBytes: Number(disk.blocks) * Number(disk.bsize),
      availableBytes: Number(disk.bavail) * Number(disk.bsize),
    },
    processes: {
      hostCount: processList?.code === 0 ? processList.stdout.trim().split("\n").filter(Boolean).length : null,
    },
    docker,
  }
}

export function evaluateRedCapResourceCeilings({ before, after, artifactBytes = 0, minFreeBytes = RED_CAP_MIN_FREE_BYTES, maxGrowthBytes = RED_CAP_MAX_GROWTH_BYTES } = {}) {
  const available = Number(after?.disk?.availableBytes ?? 0)
  const initial = Number(before?.disk?.availableBytes ?? 0)
  const violations = []
  if (!Number.isSafeInteger(available) || available <= 0) violations.push("post-run disk availability is not a positive integer")
  if (available < minFreeBytes) violations.push(`post-run free disk ${available} is below the ${minFreeBytes} byte safety floor`)
  if (!Number.isSafeInteger(Number(artifactBytes)) || Number(artifactBytes) > maxGrowthBytes) {
    violations.push(`evidence growth ${artifactBytes} exceeds the ${maxGrowthBytes} byte ceiling`)
  }
  if (initial > 0 && available < initial - maxGrowthBytes) {
    violations.push(`disk delta ${initial - available} exceeds the ${maxGrowthBytes} byte ceiling`)
  }
  return {
    ok: violations.length === 0,
    violations,
    minFreeBytes,
    maxGrowthBytes,
    artifactBytes: Number(artifactBytes),
    diskAvailableBefore: initial,
    diskAvailableAfter: available,
  }
}

export async function checkRedCapPrerequisites({
  profile,
  repoRoot,
  expectedMainSha = AUTHORITATIVE_MAIN_SHA,
  image = RED_CAP_DEFAULT_IMAGE,
  rootPath = repoRoot,
  runCommand = spawnBounded,
  minFreeBytes = RED_CAP_MIN_FREE_BYTES,
} = {}) {
  const resolvedProfile = typeof profile === "string" ? resolveRedCapProfile(profile) : profile
  const missing = []
  const checks = {}
  if (resolvedProfile.platform !== process.platform) {
    missing.push(`${resolvedProfile.id} requires ${resolvedProfile.platform}; current host is ${process.platform}`)
  }
  for (const relative of FILES_REQUIRED_BY_STACK) {
    try {
      await access(path.join(repoRoot, relative))
      checks[relative] = true
    } catch {
      missing.push(`required stack file is missing: ${relative}`)
      checks[relative] = false
    }
  }
  const head = await runCommand("git", ["rev-parse", "HEAD"], { cwd: repoRoot, timeoutMs: 5_000, label: "git-head" })
  const originMain = await runCommand("git", ["rev-parse", "refs/remotes/origin/main"], { cwd: repoRoot, timeoutMs: 5_000, label: "git-origin-main" })
  const testedSha = head.code === 0 ? head.stdout.trim() : null
  const originMainSha = originMain.code === 0 ? originMain.stdout.trim() : null
  checks.testedSha = testedSha
  checks.originMainSha = originMainSha
  if (originMainSha !== expectedMainSha) missing.push(`origin/main is ${originMainSha || "unavailable"}; expected ${expectedMainSha}`)
  const dockerVersion = await runCommand("docker", ["--version"], { timeoutMs: 5_000, label: "docker-version" })
  checks.dockerVersion = dockerVersion.code === 0 ? dockerVersion.stdout.trim() : null
  if (dockerVersion.code !== 0) missing.push("Docker CLI is unavailable")
  const dockerInfo = await runCommand("docker", ["info", "--format", "{{.ServerVersion}}"], { timeoutMs: 10_000, label: "docker-info" })
  checks.dockerInfo = dockerInfo.code === 0 ? dockerInfo.stdout.trim() : null
  if (dockerInfo.code !== 0) missing.push("Docker daemon is unavailable")
  if (dockerInfo.code === 0) {
    const imageCheck = await runCommand("docker", ["image", "inspect", image], { timeoutMs: 10_000, label: "docker-image-inspect" })
    checks.image = imageCheck.code === 0 ? image : null
    if (imageCheck.code !== 0) missing.push(`prebuilt Docker image is unavailable: ${image}`)
  }
  const disk = await statfs(rootPath)
  const availableBytes = Number(disk.bavail) * Number(disk.bsize)
  checks.availableBytes = availableBytes
  checks.minFreeBytes = minFreeBytes
  if (availableBytes < minFreeBytes) missing.push(`free disk is ${availableBytes} bytes; at least ${minFreeBytes} bytes is required`)
  return {
    ok: missing.length === 0,
    profile: resolvedProfile,
    image,
    testedSha,
    originMainSha,
    checks,
    missing,
  }
}

export function classifyRedCapAssertion({ status, expectedOutcome, evidence, reason } = {}) {
  if (!["passed", "failed", "blocked"].includes(status)) throw new Error(`invalid red-cap assertion status ${status}`)
  if (!["pass", "fail"].includes(expectedOutcome)) throw new Error(`invalid red-cap expected outcome ${expectedOutcome}`)
  const ok = status === "passed"
  const classification = status === "blocked"
    ? "blocked"
    : expectedOutcome === "fail"
      ? ok ? "green-implementation" : "red-expected"
      : ok ? "green" : "red-unexpected"
  return {
    status,
    expectedOutcome,
    classification,
    ok,
    evidence: evidence || reason || "no evidence recorded",
    ...(reason ? { reason } : {}),
  }
}

export function makeRedCapAssertion({ id, caseId, status, expectedOutcome = "pass", evidence, reason, artifactPaths = [] } = {}) {
  if (!id || !caseId) throw new Error("red-cap assertion id and caseId are required")
  return {
    id,
    caseId,
    ...classifyRedCapAssertion({ status, expectedOutcome, evidence, reason }),
    artifactPaths: [...artifactPaths],
  }
}

export function buildFunctionalEvidence({ runId, startedAt, completedAt, profile, stack, artifactRoot, caseResults } = {}) {
  const caseIds = caseResults.map((item) => item.caseId)
  const definitions = new Map(browserComputerFunctionalCases().map((item) => [item.id, item]))
  const cases = caseResults.map((item) => {
    const definition = definitions.get(item.caseId)
    if (!definition) throw new Error(`unknown functional case ${item.caseId}`)
    const assertions = definition.assertions.map((id) => {
      const result = item.assertions?.find((candidate) => candidate.id === id)
      if (!result) throw new Error(`functional case ${item.caseId} is missing assertion ${id}`)
      return { id, ok: result.status === "passed", evidence: result.evidence || result.reason || "recorded" }
    })
    return {
      caseId: item.caseId,
      status: assertions.every((assertion) => assertion.ok) ? "passed" : "failed",
      assertions,
    }
  })
  const report = {
    schema: BROWSER_COMPUTER_FUNCTIONAL_EVIDENCE_SCHEMA,
    runId,
    startedAt,
    completedAt,
    status: cases.every((item) => item.status === "passed") ? "passed" : "failed",
    ok: cases.every((item) => item.status === "passed"),
    profile: {
      id: profile.id,
      platform: profile.platform,
      topology: profile.topology,
    },
    stack: stack ?? {
      display: profile.display,
      displayVersion: "pre-cutover-noVNC",
      browserControl: profile.browserControl,
      browserControlVersion: "repository-one-shot-cdp",
      browser: "chromium",
      browserVersion: "captured-at-runtime",
    },
    artifactRoot,
    caseIds,
    coverage: JSON.stringify(caseIds) === JSON.stringify(CORE_CASE_IDS) ? "complete" : "subset",
    cases,
  }
  return validateBrowserComputerFunctionalEvidence(report, { requiredCaseIds: caseIds, repoRoots: [] })
}

export function validateRedCapEvidenceManifest(manifest, { repoRoot } = {}) {
  if (!manifest || typeof manifest !== "object") throw new Error("red-cap evidence manifest must be an object")
  if (manifest.schema !== BROWSER_COMPUTER_RED_CAP_EVIDENCE_SCHEMA) throw new Error("unsupported red-cap evidence schema")
  for (const field of ["runId", "testedSha", "artifactRoot", "status"]) {
    if (typeof manifest[field] !== "string" || manifest[field].trim() === "") throw new Error(`manifest.${field} is required`)
  }
  if (!/^[0-9a-f]{40}$/.test(manifest.testedSha)) throw new Error("manifest.testedSha must be a full git SHA")
  validateDrillTimestampOrder(manifest, "red-cap evidence")
  if (!Number.isSafeInteger(manifest.startedMonoNs) || !Number.isSafeInteger(manifest.completedMonoNs)) {
    throw new Error("red-cap evidence monotonic timestamps must be safe integers")
  }
  if (manifest.completedMonoNs < manifest.startedMonoNs) throw new Error("red-cap evidence monotonic time moved backwards")
  validateDrillDurationMatchesTimestamps(manifest, "red-cap evidence")
  assertSafeExternalPath(manifest.artifactRoot, { repoRoot, kind: "manifest.artifactRoot" })
  if (!manifest.profile?.id || !manifest.profile?.platform || !manifest.profile?.topology) throw new Error("manifest.profile is incomplete")
  if (!Array.isArray(manifest.caseIds) || manifest.caseIds.length === 0) throw new Error("manifest.caseIds must be non-empty")
  if (JSON.stringify(manifest.caseIds) !== JSON.stringify([...new Set(manifest.caseIds)].sort((a, b) => CORE_CASE_IDS.indexOf(a) - CORE_CASE_IDS.indexOf(b)))) {
    throw new Error("manifest.caseIds must be unique and follow the existing catalog order")
  }
  if (!Array.isArray(manifest.assertions) || manifest.assertions.length === 0) throw new Error("manifest.assertions must be non-empty")
  for (const assertion of manifest.assertions) {
    if (!assertion.id || !CASE_DEFINITIONS.has(assertion.caseId)) throw new Error("manifest assertion references an unknown case")
    if (!["passed", "failed", "blocked"].includes(assertion.status)) throw new Error(`invalid assertion status ${assertion.status}`)
    if (!["pass", "fail"].includes(assertion.expectedOutcome)) throw new Error("assertion expectedOutcome is invalid")
    if (!["green", "green-implementation", "red-expected", "red-unexpected", "blocked"].includes(assertion.classification)) {
      throw new Error(`invalid assertion classification ${assertion.classification}`)
    }
    const derivedClassification = classifyRedCapAssertion({
      status: assertion.status,
      expectedOutcome: assertion.expectedOutcome,
      evidence: assertion.evidence,
      reason: assertion.reason,
    }).classification
    if (assertion.classification !== derivedClassification) {
      throw new Error(`assertion classification does not match status and expected outcome: ${assertion.id}`)
    }
    if (typeof assertion.evidence !== "string" || assertion.evidence.trim() === "") throw new Error("assertion evidence is required")
    if (assertion.status === "failed" && assertion.expectedOutcome === "fail" && assertion.classification !== "red-expected") {
      throw new Error("known expected failures must be classified red-expected")
    }
  }
  if (!Array.isArray(manifest.commands)) throw new Error("manifest.commands must be an array")
  for (const command of manifest.commands) {
    if (!command.label || !command.command || !Array.isArray(command.args)) throw new Error("command evidence is incomplete")
    if (command.pid !== null && !Number.isSafeInteger(command.pid)) throw new Error("command PID must be exact or null")
    if (!Number.isSafeInteger(command.timeoutMs) || command.timeoutMs <= 0) throw new Error("command timeout is invalid")
  }
  if (!Array.isArray(manifest.resourceSamples) || manifest.resourceSamples.length < 1) throw new Error("resource samples are required")
  if (!Array.isArray(manifest.artifacts)) throw new Error("manifest.artifacts must be an array")
  for (const artifact of manifest.artifacts) {
    if (typeof artifact.path !== "string" || !path.isAbsolute(artifact.path)) throw new Error("artifact paths must be absolute")
    assertSafeExternalPath(artifact.path, { repoRoot, kind: "artifact path" })
  }
  if (!manifest.cleanup || typeof manifest.cleanup.ok !== "boolean") throw new Error("cleanup result is required")
  if (manifest.functionalEvidence) {
    validateBrowserComputerFunctionalEvidence(manifest.functionalEvidence, { requiredCaseIds: manifest.caseIds, repoRoots: repoRoot ? [repoRoot] : [] })
  }
  assertNoRedCapSecrets(manifest)
  return manifest
}

export function redCapManifestStatus(assertions, prerequisitesOk = true) {
  if (!prerequisitesOk || assertions.some((item) => item.classification === "blocked")) return "blocked"
  if (assertions.some((item) => item.classification === "red-unexpected")) return "failed"
  if (assertions.some((item) => item.classification === "red-expected")) return "expected-red"
  return "passed"
}

export async function cleanupRedCapRun({
  paths,
  plan,
  registry,
  runCommand = spawnBounded,
  createdContainer = false,
  createdVolume = false,
} = {}) {
  const commands = []
  const violations = []
  await stopOwnedProcesses(registry)
  if (createdContainer) {
    const result = await runCommand(plan.destroy.command, plan.destroy.args, { cwd: plan.destroy.cwd, timeoutMs: 30_000, label: "cleanup-container" })
    commands.push(result)
    if (result.code !== 0 && !/No such container/i.test(`${result.stdout}\n${result.stderr}`)) violations.push("owned container did not terminate")
  }
  if (createdVolume) {
    const result = await runCommand(plan.removeVolume.command, plan.removeVolume.args, { cwd: plan.removeVolume.cwd, timeoutMs: 30_000, label: "cleanup-volume" })
    commands.push(result)
    if (result.code !== 0 && !/No such volume/i.test(`${result.stdout}\n${result.stderr}`)) violations.push("owned volume did not terminate")
  }
  if (paths?.tempRoot) {
    assertOwnedTempRoot(paths.tempRoot, paths.repoRoot)
    try {
      const ownership = JSON.parse(await (await import("node:fs/promises")).readFile(path.join(paths.tempRoot, "ownership.json"), "utf8"))
      if (ownership.tempRoot !== path.resolve(paths.tempRoot) || (paths.runId && ownership.runId !== paths.runId)) {
        throw new Error("temporary root ownership marker does not match this run")
      }
      await rm(paths.tempRoot, { recursive: true, force: false })
    } catch (error) {
      violations.push(`owned temporary root remains: ${error.message}`)
    }
  }
  return { ok: violations.length === 0, violations, commands }
}

export async function artifactBytes(root) {
  let total = 0
  async function visit(current) {
    const entry = await stat(current)
    if (entry.isFile()) {
      total += entry.size
      return
    }
    if (!entry.isDirectory()) return
    const { readdir } = await import("node:fs/promises")
    for (const name of await readdir(current)) await visit(path.join(current, name))
  }
  await visit(root)
  return total
}

function assertNoRedCapSecrets(value, key = "manifest") {
  if (looksLikeDrillSecretValue(value)) throw new Error(`${key} contains a secret-looking value`)
  if (Array.isArray(value)) {
    for (const [index, child] of value.entries()) assertNoRedCapSecrets(child, `${key}[${index}]`)
    return
  }
  if (!value || typeof value !== "object") return
  for (const [childKey, child] of Object.entries(value)) {
    if (/password|token|secret|credential|authorization|api[-_]?key/i.test(childKey)) {
      if (child !== "<redacted>" && child !== undefined && child !== null) throw new Error(`${key}.${childKey} must be redacted`)
    }
    assertNoRedCapSecrets(child, `${key}.${childKey}`)
  }
}

export function sanitizeRedCapManifest(manifest) {
  return sanitizeDrillMetadata(manifest)
}

export function buildBlockedManifest({ paths, profile, testedSha, prerequisites, startedAt, startedMonoNs } = {}) {
  const completedAt = new Date().toISOString()
  const completedMonoNs = Number(process.hrtime.bigint())
  const assertions = redCapCaseIds().flatMap((caseId) => CASE_DEFINITIONS.get(caseId).assertions.map((id) => makeRedCapAssertion({
    id,
    caseId,
    status: "blocked",
    expectedOutcome: knownRedCapFailure(caseId) ? "fail" : "pass",
    reason: `live drill not run: ${prerequisites.missing.join("; ")}`,
  })))
  return {
    schema: BROWSER_COMPUTER_RED_CAP_EVIDENCE_SCHEMA,
    runId: paths.runId,
    testedSha: testedSha ?? "0000000000000000000000000000000000000000",
    originMainSha: prerequisites.originMainSha,
    startedAt,
    completedAt,
    startedMonoNs: Number(startedMonoNs),
    completedMonoNs,
    durationMs: Math.max(0, Date.parse(completedAt) - Date.parse(startedAt)),
    status: "blocked",
    ok: false,
    profile,
    stack: { display: "novnc", browserControl: "one-shot-cdp", mode: "pre-cutover" },
    artifactRoot: paths.artifactRoot,
    charioxHome: "<redacted-disposable-path>",
    caseIds: redCapCaseIds(),
    commands: [],
    assertions,
    resourceSamples: [],
    artifacts: [],
    prerequisites,
    cleanup: { ok: true, mode: "not-started", violations: [] },
  }
}

export function expectedFailureForCase(caseId) {
  return knownRedCapFailure(caseId)
}

export function profileRequiresDocker(profile) {
  return profile?.topology?.includes("docker") === true
}

export function repoRootFromModule() {
  return path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..")
}

async function requireFreshDisposableRoot(target, repoRoot) {
  const resolved = assertSafeExternalPath(target, { repoRoot, kind: "temporary root", disposable: true })
  try {
    await access(resolved)
    throw new Error(`temporary root already exists and is not disposable: ${resolved}`)
  } catch (error) {
    if (error.code === "ENOENT") return resolved
    throw error
  }
}
