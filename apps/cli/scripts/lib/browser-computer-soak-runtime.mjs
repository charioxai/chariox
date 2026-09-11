import { appendFile, chmod, lstat, mkdir, mkdtemp, open, readFile, readdir, readlink, rename, rm, stat, writeFile } from "node:fs/promises"
import { createWriteStream, readFileSync, readlinkSync } from "node:fs"
import { createHash } from "node:crypto"
import { execFile, spawn } from "node:child_process"
import http from "node:http"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { performance } from "node:perf_hooks"
import { Transform } from "node:stream"
import { promisify } from "node:util"

import {
  SELKIES_REQUIRED_PROTOCOL_VERSION,
  MINIMUM_FINAL_GATE_DURATION_SECONDS,
  buildGateReceiptPaths,
  buildSoakPaths,
  detachedLaunchSummary,
  gateFingerprint,
  validateCompletedSoakResult,
  validateGatePrerequisites,
} from "./browser-computer-soak.mjs"

const execFileAsync = promisify(execFile)
const schema = "chariox.browser_computer_soak.v1"
const minimumFreeBytes = 512 * 1024 * 1024
const maximumRetainedProcessLogBytes = 8 * 1024 * 1024
const signedRuntimeRoot = "/opt/chariox-slice"
const sensitiveEnvironmentName = /(authorization|cookie|credential|key|pass(word)?|secret|session|token)/i
const retainedSecretValues = [...new Set(Object.entries(process.env)
  .filter(([name, value]) => sensitiveEnvironmentName.test(name) && typeof value === "string" && value.length >= 4)
  .map(([, value]) => value))]
const viewport = {
  css_width: 800,
  css_height: 600,
  device_scale_factor: 1,
  desktop_pixel_width: 800,
  desktop_pixel_height: 600,
}

export function buildSanitizedChildEnvironment(base, additions = {}) {
  const allowed = ["PATH", "LANG", "LC_ALL", "LC_CTYPE", "TZ", "SSL_CERT_FILE", "SSL_CERT_DIR"]
  return Object.fromEntries([
    ...allowed.filter((name) => typeof base?.[name] === "string").map((name) => [name, base[name]]),
    ...Object.entries(additions).filter(([, value]) => typeof value === "string"),
  ])
}

export function verifiedRuntimeLayout() {
  return {
    root: signedRuntimeRoot,
    node: "/usr/local/bin/node",
    selkiesPython: "/opt/chariox-selkies/bin/python",
    selkiesBinary: "/opt/chariox-selkies/bin/selkies",
  }
}

export function runHostHelper(command, args, options = {}, {
  exec = execFileAsync,
  baseEnvironment = process.env,
} = {}) {
  return exec(command, args, { ...options, env: buildSanitizedChildEnvironment(baseEnvironment) })
}

export function redactEvidence(value, { secretValues = retainedSecretValues } = {}) {
  if (Array.isArray(value)) return value.map((entry) => redactEvidence(entry, { secretValues }))
  if (value && typeof value === "object") {
    if (value instanceof Error) return redactText(value.stack ?? value.message, secretValues)
    return Object.fromEntries(Object.entries(value).map(([key, entry]) => [key, redactEvidence(entry, { secretValues })]))
  }
  return typeof value === "string" ? redactText(value, secretValues) : value
}

function redactText(value, secretValues = retainedSecretValues) {
  let result = String(value ?? "").replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, "")
  for (const secret of [...secretValues].filter((entry) => typeof entry === "string" && entry.length >= 4).sort((a, b) => b.length - a.length)) {
    result = result.split(secret).join("[REDACTED]")
  }
  return result
    .replace(/\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]+/gi, "$1 [REDACTED]")
    .replace(/\b([A-Za-z0-9_]*(?:token|secret|password|credential|cookie|authorization|api[_-]?key)[A-Za-z0-9_]*)\s*[=:]\s*([^\s,;]+)/gi, "$1=[REDACTED]")
    .replace(/([a-z][a-z0-9+.-]*:\/\/[^\s/:@]+:)[^\s/@]+(@)/gi, "$1[REDACTED]$2")
}

export async function resolveVerifiedImage({ imageRef, signatureKey, engine = "docker" }, {
  exec = execFileAsync,
  readKey = readFile,
  baseEnvironment = process.env,
} = {}) {
  if (typeof imageRef !== "string" || imageRef.trim() === "") throw new Error("verified image requires --image-ref or CHARIOX_SLICE_IMAGE")
  if (/\s|:\/\//.test(imageRef) || imageRef.replace(/@sha256:[0-9a-f]{64}$/, "").includes("@")) {
    throw new Error("image reference must not contain whitespace, a URL scheme, or embedded credentials")
  }
  if (typeof signatureKey !== "string" || signatureKey.trim() === "") throw new Error("verified image requires a cosign signature key")
  if (!new Set(["docker", "podman"]).has(engine)) throw new Error("container engine must be docker or podman")
  let inspected
  try {
    const engineEnvironment = buildSanitizedChildEnvironment(baseEnvironment, Object.fromEntries(
      ["DOCKER_HOST", "CONTAINER_HOST", "XDG_RUNTIME_DIR"]
        .filter((name) => typeof baseEnvironment[name] === "string").map((name) => [name, baseEnvironment[name]]),
    ))
    const { stdout } = await exec(engine, ["image", "inspect", "--format", "{{json .}}", imageRef], { timeout: 20_000, env: engineEnvironment })
    inspected = JSON.parse(String(stdout).trim())
  } catch (error) {
    throw new Error("image engine inspection failed")
  }
  const candidates = Array.isArray(inspected?.RepoDigests)
    ? inspected.RepoDigests.filter((entry) => /^.+@sha256:[0-9a-f]{64}$/.test(entry)) : []
  if (!/^sha256:[0-9a-f]{64}$/.test(inspected?.Id ?? "")) throw new Error("image engine did not return an immutable local image ID")
  if (candidates.length === 0) throw new Error("image has no immutable engine RepoDigest")
  const repository = imageRef.replace(/@sha256:[0-9a-f]{64}$/, "").replace(/:[^/:]+$/, "")
  const configuredDigest = imageRef.match(/@(sha256:[0-9a-f]{64})$/)?.[1]
  const identity = configuredDigest
    ? candidates.find((entry) => entry === `${repository}@${configuredDigest}`)
    : candidates.find((entry) => entry.slice(0, entry.lastIndexOf("@")) === repository)
  if (!identity) throw new Error("image engine RepoDigest does not match the configured repository")
  const digest = identity.slice(identity.lastIndexOf("@") + 1)
  const keyBytes = await readKey(signatureKey).catch(() => { throw new Error("image signature key unavailable") })
  if (!Buffer.isBuffer(keyBytes) || keyBytes.length === 0 || keyBytes.length > 64 * 1024) throw new Error("image signature key is empty or unreasonably large")
  const verificationEnv = buildSanitizedChildEnvironment(baseEnvironment, { COSIGN_PUBLIC_KEY: keyBytes.toString("utf8") })
  const verify = async (kind, args) => {
    let stdout
    try { ({ stdout } = await exec("cosign", args, { timeout: 60_000, env: verificationEnv, maxBuffer: 1024 * 1024 })) } catch {
      throw new Error(`image ${kind} verification failed`)
    }
    const text = String(stdout).trim()
    if (!verificationOutputBindsDigest(text, digest)) throw new Error(`image ${kind} verification did not bind the engine digest`)
    let canonical
    try { canonical = canonicalVerificationOutput(text) } catch { throw new Error(`image ${kind} verification returned malformed evidence`) }
    return createHash("sha256").update(canonical).digest("hex")
  }
  const signatureBundleDigest = await verify("signature", ["verify", "--key", "env://COSIGN_PUBLIC_KEY", "--output", "json", identity])
  const attestationBundleDigest = await verify("attestation", ["verify-attestation", "--key", "env://COSIGN_PUBLIC_KEY", "--type", "slsaprovenance", "--output", "json", identity])
  return {
    identity,
    digest,
    engine,
    engineImageId: inspected.Id,
    signature: { verified: true, verifier: "cosign", keySha256: createHash("sha256").update(keyBytes).digest("hex"), bundleSha256: signatureBundleDigest },
    attestation: { verified: true, type: "slsaprovenance", bundleSha256: attestationBundleDigest },
  }
}

export async function inspectDigestBoundRuntime({ containerId, image, source }, {
  exec = execFileAsync,
  baseEnvironment = process.env,
} = {}) {
  if (!/^[0-9a-f]{12,64}$/.test(containerId ?? "")) {
    throw new Error("runtime container identity is required")
  }
  const environment = buildSanitizedChildEnvironment(baseEnvironment, Object.fromEntries(
    ["DOCKER_HOST", "CONTAINER_HOST", "XDG_RUNTIME_DIR"]
      .filter((name) => typeof baseEnvironment[name] === "string").map((name) => [name, baseEnvironment[name]]),
  ))
  let inspected
  try {
    const { stdout } = await exec(image.engine, ["container", "inspect", "--format", "{{json .}}", containerId], {
      timeout: 20_000, env: environment, maxBuffer: 1024 * 1024,
    })
    inspected = JSON.parse(String(stdout).trim())
  } catch {
    throw new Error("runtime container inspection failed")
  }
  if (!/^[0-9a-f]{64}$/.test(inspected?.Id ?? "")
    || !(inspected.Id === containerId || inspected.Id.startsWith(containerId))) {
    throw new Error("runtime container inspection returned a different container identity")
  }
  if (inspected?.State?.Running !== true) throw new Error("runtime container is not running")
  if (inspected?.Image !== image.engineImageId || inspected?.Config?.Image !== image.identity) {
    throw new Error("runtime image does not match the verified image digest and engine image ID")
  }
  const sourceRevision = inspected?.Config?.Labels?.["io.chariox.runtime-source-revision"]
  if (!/^[0-9a-f]{40}$/.test(sourceRevision ?? "") || sourceRevision !== source?.commit) {
    throw new Error("runtime source revision does not match the clean source commit")
  }
  return { containerId: inspected.Id, imageId: inspected.Image, identity: inspected.Config.Image, sourceRevision, running: true }
}

export function assertCurrentContainerIdentity(containerId, hostname = os.hostname()) {
  if (typeof hostname !== "string" || hostname.length < 12 || !containerId?.startsWith(hostname)) {
    throw new Error("inspected runtime container is not the container executing the soak")
  }
}

function verificationOutputBindsDigest(text, digest) {
  if (!text) return false
  const values = []
  try { values.push(JSON.parse(text)) } catch {
    for (const line of text.split("\n").filter(Boolean)) try { values.push(JSON.parse(line)) } catch {}
  }
  const digestHex = digest.slice("sha256:".length)
  const bound = (value) => {
    if (Array.isArray(value)) return value.some(bound)
    if (!value || typeof value !== "object") return false
    if (value.critical?.image?.["docker-manifest-digest"] === digest) return true
    if (Array.isArray(value.subject) && value.subject.some((subject) => subject?.digest?.sha256 === digestHex)) return true
    if (typeof value.payload === "string" && value.payload.length <= 2 * 1024 * 1024) {
      try { if (bound(JSON.parse(Buffer.from(value.payload, "base64").toString("utf8")))) return true } catch {}
    }
    return Object.values(value).some((entry) => typeof entry === "object" && bound(entry))
  }
  return values.some(bound)
}

function canonicalVerificationOutput(text) {
  let values
  try { values = [JSON.parse(text)] } catch {
    values = text.split("\n").filter(Boolean).map((line) => JSON.parse(line))
  }
  const canonical = (value) => {
    if (Array.isArray(value)) return `[${value.map(canonical).sort().join(",")}]`
    if (value && typeof value === "object") return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`).join(",")}}`
    return JSON.stringify(value)
  }
  return values.map(canonical).sort().join("\n")
}

export function assertFinalDetachPrerequisites(options, provenance) {
  if (options.mode !== "detach") return
  if (options.viewerBackend !== "selkies" || provenance?.viewer?.backend !== "selkies") throw new Error("final detach requires Selkies")
  if (provenance?.localDaemonProtocolVersion < SELKIES_REQUIRED_PROTOCOL_VERSION) throw new Error("final detach requires source protocol 322 or newer")
  if (options.durationSeconds < MINIMUM_FINAL_GATE_DURATION_SECONDS) throw new Error("final detach requires at least 28,800 seconds")
}

export function attributableNetworkDelta(baseline, current, attribution) {
  if (baseline?.attribution?.exclusive !== true || attribution?.exclusive !== true
    || baseline.attribution.namespace !== attribution.namespace) {
    throw new Error(`attributable network accounting unavailable${attribution?.foreignPids?.length ? `; foreign namespace PIDs: ${attribution.foreignPids.join(",")}` : ""}`)
  }
  const difference = current?.totalBytes - baseline?.totalBytes
  if (!Number.isFinite(difference) || difference < 0) throw new Error("owned network counters regressed or are invalid")
  return difference
}

export async function createRuntimeState(paths, {
  makeTemporary = mkdtemp,
  makeDirectory = mkdir,
  write = writeFile,
  remove = rm,
  pathExists = exists,
  temporaryRoot = os.tmpdir(),
} = {}) {
  let stateRoot = null
  try {
    stateRoot = await makeTemporary(path.join(temporaryRoot, "chariox-browser-computer-soak-"))
    const runtimeRoot = path.join(stateRoot, "runtime")
    const profileRoot = path.join(stateRoot, "chromium-profile")
    const logsRoot = path.join(paths.runDir, "process-logs")
    for (const candidate of [runtimeRoot, profileRoot, logsRoot]) {
      await makeDirectory(candidate, { recursive: true, mode: 0o700 })
    }
    await write(paths.pid, `${process.pid}\n`, { mode: 0o600 })
    return { stateRoot, runtimeRoot, profileRoot, logsRoot }
  } catch (error) {
    const failure = error instanceof Error ? error : new Error(bounded(error))
    let removeError = null
    if (stateRoot) try { await remove(stateRoot, { recursive: true, force: true }) } catch (cause) { removeError = cause }
    let verificationError = null
    let stateRemoved = stateRoot == null
    if (stateRoot) try { stateRemoved = !await pathExists(stateRoot) } catch (cause) { verificationError = cause }
    failure.terminalCleanup = {
      schema: "chariox.browser_computer_soak_cleanup.v1",
      at: new Date().toISOString(),
      phase: "partial-setup",
      actions: [{
        name: "partial-state-remove",
        ok: stateRemoved && removeError == null && verificationError == null,
        error: removeError || verificationError ? bounded((removeError ?? verificationError)?.message ?? (removeError ?? verificationError)) : undefined,
      }],
      stateRemoved,
      verificationCompleted: verificationError == null,
      clean: stateRemoved && removeError == null && verificationError == null,
    }
    throw failure
  }
}

export async function runBrowserComputerSoak({ options, repoRoot, scriptPath }) {
  const runId = new Date().toISOString().replace(/[:.]/g, "-")
  const paths = options.runDir
    ? buildSoakPaths(path.dirname(options.runDir), path.basename(options.runDir))
    : buildSoakPaths(options.evidenceRoot, runId)
  const startedAt = new Date().toISOString()
  let allocation = null
  let source = null
  let baseline = null
  try {
    if (options.launchGate) await awaitDetachedLaunchGate(options.launchGate)
    await mkdir(paths.runDir, { recursive: true, mode: 0o700 })
    if (options.mode === "run") {
      await writeFile(paths.pid, `${process.pid}\n`, { mode: 0o600 })
      await writeJson(paths.status, { schema, status: "starting", pid: process.pid, startedAt, runDir: paths.runDir })
    }
    source = await captureSourceIdentity(repoRoot)
    const provenance = await captureProvenance({ options, repoRoot, source })
    assertFinalDetachPrerequisites(options, provenance)
    allocation = await allocateRuntime(options)
    baseline = await baselineResourceSnapshot(paths)
    const preflight = await runPreflight({ options, allocation, paths, repoRoot, source, baseline, provenance })
    await assertSourceUnchanged(repoRoot, source)
    await writeJson(paths.preflight, preflight)
    if (options.mode === "preflight") {
      await writeGateReceipt(options.evidenceRoot, "preflight", preflight, provenance)
      console.log(JSON.stringify({ status: "passed", runDir: paths.runDir, preflight: paths.preflight, allocation }))
      return
    }
    await requireGateReceipts(options.evidenceRoot, provenance, { smoke: !options.smoke })
    if (options.mode === "detach") {
      await assertSourceUnchanged(repoRoot, source)
      const args = detachedArgs({ options, paths, allocation })
      const child = await launchDetachedRunner({
        command: process.execPath,
        args: [scriptPath, ...args],
        cwd: repoRoot,
        logPath: paths.log,
        gatePath: paths.launchGate,
        env: buildSanitizedChildEnvironment(process.env, {
          CHARIOX_SLICE_IMAGE: options.imageRef,
          CHARIOX_SLICE_IMAGE_SIGNATURE_KEY: options.imageSignatureKey,
          CHARIOX_CONTAINER_ENGINE: options.containerEngine,
        }),
      }, {
        writeStartupEvidence: async (startedChild) => {
          await writeFile(paths.pid, `${startedChild.pid}\n`, { mode: 0o600 })
          await writeJson(paths.status, {
            schema,
            status: "starting",
            pid: startedChild.pid,
            startedAt,
            runDir: paths.runDir,
          })
        },
        persistFailure: async (error) => {
          await persistStartupFailure({ error, options, allocation, paths, source, baseline, startedAt })
        },
      })
      console.log(JSON.stringify(detachedLaunchSummary(paths, child.pid)))
      return
    }
    const result = await executeSoak({ options, allocation, paths, repoRoot, source, baseline, provenance })
    if (options.smoke && result.status === "passed") {
      await writeGateReceipt(options.evidenceRoot, "smoke", result, provenance)
    }
  } catch (error) {
    if (await exists(paths.runDir) && !await exists(paths.result)) {
      await persistStartupFailure({ error, options, allocation, paths, source, baseline, startedAt })
    }
    throw error
  }
}

export async function launchDetachedRunner({ command, args, cwd, env, logPath, gatePath }, {
  openLog = (candidate) => open(candidate, "a", 0o600),
  prepareLaunchGate = (candidate) => rm(candidate, { force: true }),
  spawnChild = spawn,
  captureIdentity = (name, child) => captureSpawnedIdentity(name, child),
  writeStartupEvidence,
  releaseLaunchGate = releaseDetachedLaunchGate,
  terminate = terminateOwnedProcessGroup,
  persistFailure = async () => {},
} = {}) {
  let descriptor = null
  let child = null
  try {
    if (typeof gatePath !== "string" || gatePath === "") throw new Error("detached runner requires a launch gate")
    await prepareLaunchGate(gatePath)
    descriptor = await openLog(logPath)
    child = spawnChild(command, args, { cwd, env, detached: true, stdio: ["ignore", descriptor.fd, descriptor.fd] })
    child.ownedIdentity = await captureIdentity("detached runner", child)
    await descriptor.close()
    descriptor = null
    await writeStartupEvidence(child)
    child.unref()
    await releaseLaunchGate(gatePath)
    return child
  } catch (cause) {
    if (descriptor) await descriptor.close().catch(() => {})
    const action = child
      ? await terminate("detached-runner-rollback", child)
      : { name: "detached-runner-rollback", ok: true, notStarted: true, pidReuseSafe: true }
    const error = cause instanceof Error ? cause : new Error(bounded(cause))
    error.detachedChildPid = child?.pid ?? null
    error.terminalCleanup = {
      schema: "chariox.browser_computer_soak_cleanup.v1",
      at: new Date().toISOString(),
      phase: "startup",
      actions: [action],
      remainingPids: action.ok ? [] : [child?.pid].filter(Number.isSafeInteger),
      pidReuseSafe: action.pidReuseSafe === true,
      verificationCompleted: true,
      clean: action.ok === true && action.pidReuseSafe === true,
    }
    try {
      await persistFailure(error)
    } catch (persistenceError) {
      error.failurePersistenceError = bounded(persistenceError?.message ?? persistenceError)
    }
    throw error
  }
}

async function releaseDetachedLaunchGate(candidate, {
  write = writeFile,
  move = rename,
  remove = rm,
} = {}) {
  const temporary = `${candidate}.${process.pid}.tmp`
  try {
    await write(temporary, `${process.pid}\n`, { mode: 0o600 })
    await move(temporary, candidate)
  } catch (error) {
    await remove(temporary, { force: true }).catch(() => {})
    throw error
  }
}

async function awaitDetachedLaunchGate(gatePath, {
  pathExists = exists,
  remove = rm,
} = {}) {
  await retry(async () => {
    if (!await pathExists(gatePath)) throw new Error("launch gate not released")
  }, 30_000, "detached soak launch gate was not released")
  await remove(gatePath, { force: true })
}

async function executeSoak({ options, allocation, paths, repoRoot, source, baseline, provenance }) {
  const startedAt = new Date().toISOString()
  const runtime = verifiedRuntimeLayout()
  const sourceRoot = runtime.root
  let stateRoot = null
  let runtimeRoot = null
  let profileRoot = null
  const logsRoot = path.join(paths.runDir, "process-logs")
  const owned = new Map()
  let fixture = null
  let controller = null
  let stream = null
  let selkiesPid = null
  let interrupted = null
  let sampleCount = 0
  let peakOwnedRssBytes = 0
  let peakOwnedCpuPercent = 0
  let peakOwnedProcessCount = 0
  let peakOwnedOpenFiles = 0
  let networkBytes = 0
  let diskGrowthBytes = 0
  let screenshotDigests = new Set()
  let iterations = 0
  let chromiumMutations = 0
  let structuredBrowserActions = 0
  let computerScreenshots = 0
  let computerInputs = 0
  let firstHealth = null
  let finalHealth = null
  let failure = null
  let status = "running"
  let monotonicStartedAt = null
  let wallStartedAt = null
  let lastCadenceAt = null
  let observedMaxCadenceGapMs = 0
  let lastActivityAt = null
  let ownedIdentities = []
  let initialStreamMetrics = null
  let finalHealthCheckedAt = null
  let finalStreamMetrics = null
  let activeMonotonicElapsedMs = 0
  let activeWallElapsedMs = 0
  let setupCleanup = null

  const signalHandler = (signal) => { interrupted ??= signal }
  process.once("SIGINT", signalHandler)
  process.once("SIGTERM", signalHandler)

  const display = `:${allocation.displayNumber}`
  let environment = null
  const environmentAdditions = {
    DISPLAY: display,
    CHARIOX_SLICE_DISPLAY: display,
    CHARIOX_SLICE_NOVNC_PORT: String(allocation.viewerPort),
    CHARIOX_SLICE_VIEWER_BACKEND: options.viewerBackend,
    CHARIOX_BROWSER_DEBUGGER_ENDPOINT: `http://127.0.0.1:${allocation.debugPort}`,
    CHARIOX_SLICE_ROOT: sourceRoot,
    OMP_NUM_THREADS: "1",
    PYTHONDONTWRITEBYTECODE: "1",
  }
  const command = commandEvidence({ options, paths, allocation })
  const result = {
    schema,
    status,
    pid: process.pid,
    startedAt,
    completedAt: null,
    durationSeconds: options.durationSeconds,
    smoke: options.smoke,
    command,
    source,
    provenance,
    allocation,
    paths,
    firstHealth: null,
    finalHealth: null,
    controller: null,
    viewer: { backend: options.viewerBackend },
    activity: {
      iterations: 0, controllerRequests: 0, chromiumMutations: 0, structuredBrowserActions: 0,
      computerScreenshots: 0, computerInputs: 0, screenshotDigests: 0, lastAt: null,
    },
    stream: { ready: false, binaryFrames: 0, binaryBytes: 0, changingFrameDigests: 0, textMarkers: [] },
    resources: { sampleCount: 0, peakOwnedRssBytes: 0, peakOwnedCpuPercent: 0, baseline },
    timing: null,
    gate: finalGateEligibility(options.viewerBackend, provenance.localDaemonProtocolVersion, options.durationSeconds),
    cleanup: null,
  }

  try {
    const setup = await createRuntimeState(paths)
    stateRoot = setup.stateRoot
    runtimeRoot = setup.runtimeRoot
    profileRoot = setup.profileRoot
    environment = buildSanitizedChildEnvironment(process.env, {
      ...environmentAdditions,
      HOME: stateRoot,
      TMPDIR: stateRoot,
      XDG_RUNTIME_DIR: runtimeRoot,
    })
    await assertRuntimeStillAvailable(allocation)
    fixture = await startFixtureServer()
    const xvfb = await spawnLogged("xvfb", "Xvfb", [display, "-screen", "0", "800x600x24", "-ac", "+extension", "RANDR", "+extension", "XTEST"], {
      env: environment, cwd: repoRoot, logsRoot,
    })
    owned.set("xvfb", xvfb)
    await waitForDisplay(display, environment)
    owned.set("openbox", await spawnLogged("openbox", "openbox", [], { env: environment, cwd: repoRoot, logsRoot }))
    owned.set("tint2", await spawnLogged("tint2", "tint2", ["-c", path.join(sourceRoot, "tint2rc")], { env: environment, cwd: repoRoot, logsRoot }))

    const chromiumArgs = [
      `--user-data-dir=${profileRoot}`,
      "--password-store=basic",
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-sync",
      "--disable-dev-shm-usage",
      "--disable-gpu",
      "--remote-debugging-address=127.0.0.1",
      `--remote-debugging-port=${allocation.debugPort}`,
      "--window-size=800,600",
      fixture.url,
    ]
    owned.set("chromium", await spawnLogged("chromium", "chromium", chromiumArgs, { env: environment, cwd: repoRoot, logsRoot }))
    await waitForHttp(`http://127.0.0.1:${allocation.debugPort}/json/version`, 20_000)

    const selkies = await execJson("/opt/chariox-selkies/bin/python", [path.join(sourceRoot, "slice-selkies.py"), "start"], {
      cwd: sourceRoot, env: environment, timeout: 30_000,
    })
    if (selkies.available !== true || !Number.isSafeInteger(selkies.pid)) throw new Error("Selkies did not report a healthy owned process")
    selkiesPid = selkies.pid
    ownedIdentities.push(...await captureOwnedIdentities([["viewer", selkiesPid]]))

    controller = new ControllerClient(await spawnProtocol("browser-controller", runtime.node, [path.join(sourceRoot, "browser-controller.mjs"), "stdio"], {
      env: environment, cwd: repoRoot, logsRoot,
    }))
    stream = new StreamClient(await spawnProtocol("selkies-stream", runtime.selkiesPython, [
      path.join(sourceRoot, "slice-selkies-stream.py"), "--lease-ms", "60000",
    ], { env: environment, cwd: sourceRoot, logsRoot }))
    await stream.waitReady(15_000)
    stream.startRenewal()
    stream.requestKeyframe()
    await stream.waitForFrames(2, 10_000)
    initialStreamMetrics = stream.metrics()
    monotonicStartedAt = performance.now()
    wallStartedAt = Date.now()
    lastCadenceAt = monotonicStartedAt

    ownedIdentities = await captureOwnedIdentities([
      ["runner", process.pid], ...[...owned].map(([name, child]) => [name, child.pid]),
      ["browser-controller", controller.child.pid], ["display-stream", stream.child.pid], ["viewer", selkiesPid],
    ])
    result.controller = ownedIdentities.find((entry) => entry.name === "browser-controller")

    await recordSample("initial", [process.pid, ...pids(owned), selkiesPid])
    const deadline = monotonicStartedAt + options.durationSeconds * 1_000
    let nextActivity = performance.now()
    let nextSample = performance.now() + options.sampleIntervalSeconds * 1_000
    while (performance.now() < deadline) {
      if (interrupted) throw new Error(`soak interrupted by ${interrupted}`)
      assertProcessHealth(owned, controller, stream)
      assertOwnedPidHealth(ownedIdentities.find((entry) => entry.name === "viewer"), "display viewer")
      const now = performance.now()
      observedMaxCadenceGapMs = Math.max(observedMaxCadenceGapMs, now - lastCadenceAt)
      if (now - lastCadenceAt > options.limits.maxCadenceGapMs) throw new Error("soak cadence exceeded its monotonic gap limit")
      lastCadenceAt = now
      if (now >= nextActivity) {
        const marker = `SOAK-${String(iterations + 1).padStart(8, "0")}`
        const health = await controller.request("health")
        firstHealth ??= health
        const reconciled = await controller.request("browser.reconcile", { viewport })
        const tab = reconciled.tabs.find((entry) => entry.url === fixture.url) ?? reconciled.tabs[0]
        if (!tab) throw new Error("Browser Controller returned no Chromium tab")
        const snapshot = await controller.request("browser.snapshot", {
          target_id: tab.target_id,
          document_id: tab.document_id,
        })
        const field = snapshot.accessibility_nodes.find((entry) =>
          !entry.ignored && entry.role === "textbox" && entry.name.trim() === "Soak marker")
        if (!field?.node_ref) throw new Error("Browser Controller did not discover the soak field")
        await controller.request("browser.action", {
          target_id: tab.target_id,
          document_id: tab.document_id,
          node_ref: field.node_ref,
          action: { kind: "fill", text: marker },
        })
        structuredBrowserActions += 1
        const observedMarker = await fetch(`${fixture.url}health`, { signal: AbortSignal.timeout(2_000) })
          .then((response) => response.text())
        if (observedMarker !== marker) throw new Error("Chromium mutation did not reach the active fixture")
        chromiumMutations += 1
        stream.requestKeyframe()
        const inputProof = await verifyComputerInputEffect({ iteration: iterations, cwd: repoRoot, env: environment })
        computerInputs += 1
        const screenshotPath = path.join(paths.runDir, "latest-screen.png")
        await execFileAsync("scrot", [screenshotPath], { cwd: repoRoot, env: environment, timeout: 10_000 })
        const screenshotDigest = createHash("sha256").update(await readFile(screenshotPath)).digest("hex")
        screenshotDigests.add(screenshotDigest)
        computerScreenshots += 1
        iterations += 1
        lastActivityAt = new Date().toISOString()
        await appendFile(paths.activity, `${evidenceJson({
          at: lastActivityAt,
          monotonicElapsedMs: performance.now() - monotonicStartedAt,
          iteration: iterations,
          controllerRequests: controller.requestCount,
          chromiumMutations,
          structuredBrowserActions,
          computerScreenshots,
          computerInputs,
          inputProof,
          screenshotDigest,
          stream: stream.metrics(),
        })}\n`, { mode: 0o600 })
        await chmod(paths.activity, 0o600)
        await writeJson(paths.status, {
          schema,
          status: "healthy",
          pid: process.pid,
          startedAt,
          updatedAt: new Date().toISOString(),
          firstHealth,
          activity: { iterations, controllerRequests: controller.requestCount, chromiumMutations, structuredBrowserActions, computerScreenshots, computerInputs, lastAt: lastActivityAt },
          stream: stream.metrics(),
          resources: { sampleCount, peakOwnedRssBytes, peakOwnedCpuPercent, peakOwnedProcessCount, peakOwnedOpenFiles, diskGrowthBytes, networkBytes },
          runDir: paths.runDir,
        })
        nextActivity += options.activityIntervalSeconds * 1_000
      }
      if (now >= nextSample) {
        await recordSample("during", [process.pid, ...pids(owned), selkiesPid])
        nextSample += options.sampleIntervalSeconds * 1_000
      }
      await sleep(Math.min(250, Math.max(1, Math.min(nextActivity, nextSample, deadline) - performance.now())))
    }
    assertProcessHealth(owned, controller, stream)
    assertOwnedPidHealth(ownedIdentities.find((entry) => entry.name === "viewer"), "display viewer")
    finalHealth = await controller.request("health")
    finalHealthCheckedAt = new Date().toISOString()
    const framesBeforeFinalCheck = stream.metrics().binaryFrames
    stream.requestKeyframe()
    await stream.waitForFrames(framesBeforeFinalCheck + 1, 10_000)
    assertProcessHealth(owned, controller, stream)
    await recordSample("final", [process.pid, ...pids(owned), selkiesPid])
    const finalCadenceAt = performance.now()
    observedMaxCadenceGapMs = Math.max(observedMaxCadenceGapMs, finalCadenceAt - lastCadenceAt)
    if (finalCadenceAt - lastCadenceAt > options.limits.maxCadenceGapMs) throw new Error("final checks exceeded the monotonic cadence limit")
    activeMonotonicElapsedMs = finalCadenceAt - monotonicStartedAt
    activeWallElapsedMs = Date.now() - wallStartedAt
    finalStreamMetrics = stream.metrics()
    await assertSourceUnchanged(repoRoot, source)
    status = "passed"
  } catch (error) {
    setupCleanup = error?.terminalCleanup ?? null
    status = interrupted ? "interrupted" : "failed"
    failure = bounded(error?.stack ?? error)
  } finally {
    const functionalCompletedAt = new Date().toISOString()
    stream?.stopRenewal()
    stream?.stopObserving()
    let cleanup
    try {
      cleanup = await cleanupRuntime({ controller, stream, owned, ownedIdentities, selkiesPid, environment, sourceRoot, fixture, stateRoot, allocation })
    } catch (error) {
      status = "failed"
      failure = bounded(`cleanup failed: ${error?.stack ?? error}`)
      cleanup = failedCleanupEvidence(error)
    }
    if (setupCleanup) {
      cleanup.actions.unshift(...setupCleanup.actions)
      cleanup.stateRemoved = cleanup.stateRemoved && setupCleanup.stateRemoved
      cleanup.clean = cleanup.clean && setupCleanup.clean
      cleanup.partialSetupVerificationCompleted = setupCleanup.verificationCompleted
    }
    await writeJson(paths.cleanup, cleanup)
    const streamMetrics = finalStreamMetrics ?? stream?.metrics() ?? result.stream
    result.status = status
    result.completedAt = functionalCompletedAt
    result.finalizedAt = new Date().toISOString()
    result.firstHealth = firstHealth
    result.finalHealth = finalHealth ? { ...finalHealth, checkedAt: finalHealthCheckedAt } : null
    result.activity = {
      iterations,
      controllerRequests: controller?.requestCount ?? 0,
      chromiumMutations,
      structuredBrowserActions,
      computerScreenshots,
      computerInputs,
      screenshotDigests: screenshotDigests.size,
      lastAt: lastActivityAt,
    }
    result.stream = {
      ...streamMetrics,
      ready: stream?.ready === true,
      initialBinaryFrames: initialStreamMetrics?.binaryFrames ?? 0,
      finalBinaryFrames: streamMetrics.binaryFrames,
      initialActivityCounter: initialStreamMetrics?.activityCounter ?? 0,
      finalActivityCounter: streamMetrics.activityCounter ?? 0,
      initialChangingFrameDigests: initialStreamMetrics?.changingFrameDigests ?? 0,
      finalChangingFrameDigests: streamMetrics.changingFrameDigests ?? 0,
    }
    result.timing = {
      expectedDurationMs: options.durationSeconds * 1_000,
      monotonicElapsedMs: activeMonotonicElapsedMs,
      wallElapsedMs: activeWallElapsedMs,
      maxCadenceGapMs: options.limits.maxCadenceGapMs,
      observedMaxCadenceGapMs,
      wallMonotonicSkewMs: activeWallElapsedMs - activeMonotonicElapsedMs,
    }
    result.resources = {
      sampleCount, peakOwnedRssBytes, peakOwnedCpuPercent, peakOwnedProcessCount, peakOwnedOpenFiles,
      diskGrowthBytes, networkBytes, limits: options.limits, baseline,
      withinBounds: resourceBoundsPass({ peakOwnedRssBytes, peakOwnedCpuPercent, peakOwnedProcessCount, peakOwnedOpenFiles, diskGrowthBytes, networkBytes }, options.limits),
    }
    result.cleanup = cleanup
    if (failure) result.failure = failure
    if (status === "passed") {
      try { validateCompletedSoakResult(result) } catch (error) {
        result.status = "failed"
        result.failure = bounded(error?.message ?? error)
      }
    }
    if (result.status !== "passed") {
      await writeJson(paths.failure, {
        schema: "chariox.browser_computer_soak_failure.v1",
        status: result.status,
        at: result.completedAt,
        marker: result.failure ?? "soak did not pass",
      })
    }
    await writeJson(paths.result, result)
    await writeJson(paths.status, {
      schema,
      status: result.status,
      pid: process.pid,
      startedAt,
      completedAt: result.completedAt,
      result: paths.result,
      cleanup: paths.cleanup,
    })
    process.removeListener("SIGINT", signalHandler)
    process.removeListener("SIGTERM", signalHandler)
    console.log(JSON.stringify({ status: result.status, pid: process.pid, runDir: paths.runDir, result: paths.result }))
    if (result.status !== "passed") process.exitCode = 1
    return result
  }

  async function recordSample(label, roots) {
    const sample = await resourceSnapshot(label, roots.filter(Number.isSafeInteger), paths.runDir)
    sample.monotonicElapsedMs = monotonicStartedAt == null ? null : performance.now() - monotonicStartedAt
    ownedIdentities = await mergeSampledIdentities(ownedIdentities, sample.owned.processes)
    sample.owned.diskBytes = await ownedDiskBytes([paths.runDir, stateRoot])
    sample.owned.networkBytes = attributableNetworkDelta(baseline.network, sample.network, sample.network.attribution)
    sampleCount += 1
    peakOwnedRssBytes = Math.max(peakOwnedRssBytes, sample.owned.rssBytes)
    peakOwnedCpuPercent = Math.max(peakOwnedCpuPercent, sample.owned.cpuPercent)
    peakOwnedProcessCount = Math.max(peakOwnedProcessCount, sample.owned.processCount)
    peakOwnedOpenFiles = Math.max(peakOwnedOpenFiles, sample.owned.openFiles)
    diskGrowthBytes = Math.max(diskGrowthBytes, sample.owned.diskBytes)
    networkBytes = Math.max(networkBytes, sample.owned.networkBytes)
    if (!resourceBoundsPass({ peakOwnedRssBytes, peakOwnedCpuPercent, peakOwnedProcessCount, peakOwnedOpenFiles, diskGrowthBytes, networkBytes }, options.limits)) {
      throw new Error("soak resource evidence exceeded configured bounds")
    }
    await appendFile(paths.samples, `${evidenceJson(sample)}\n`, { mode: 0o600 })
    await chmod(paths.samples, 0o600)
  }
}

async function runPreflight({ options, allocation, paths, repoRoot, source, baseline, provenance }) {
  assertSandboxCapableChromiumIdentity()
  if (source.dirty) throw new Error("preflight requires the exact clean source tree")
  if (options.viewerBackend !== "selkies") {
    throw new Error("noVNC may provide diagnostic evidence but cannot run the active final-gate stream contract")
  }
  const runtime = verifiedRuntimeLayout()
  const sourceRoot = runtime.root
  const commands = ["Xvfb", "openbox", "tint2", "chromium", "scrot", "xdpyinfo", "xdotool", "ps"]
  const commandPaths = {}
  for (const command of commands) commandPaths[command] = (await runHostHelper("which", [command], { timeout: 5_000 })).stdout.trim()
  const files = [
    "browser-controller.mjs",
    "browser-controller-cdp.mjs",
    "slice-selkies.py",
    "slice-selkies-stream.py",
    "selkies_viewers.py",
    "tint2rc",
  ]
  for (const file of files) await stat(path.join(sourceRoot, file))
  await Promise.all([
    stat(runtime.node),
    stat(runtime.selkiesPython),
    stat(runtime.selkiesBinary),
  ])
  if (baseline.host.freeMemoryBytes < minimumFreeBytes) throw new Error("preflight requires at least 512 MiB free RAM")
  if (baseline.disk.availableBytes < minimumFreeBytes) throw new Error("preflight requires at least 512 MiB free disk")
  if (baseline.network?.attribution?.exclusive !== true) {
    throw new Error("preflight requires exclusive network-namespace accounting for an owned network bound")
  }
  await assertRuntimeStillAvailable(allocation)
  return {
    schema: "chariox.browser_computer_soak_preflight.v1",
    status: "passed",
    at: new Date().toISOString(),
    durationSeconds: options.durationSeconds,
    smoke: options.smoke,
    source,
    provenance,
    fingerprint: gateFingerprint(provenance),
    allocation,
    commandPaths,
    sourceFiles: files,
    baseline,
    paths,
  }
}

export function assertSandboxCapableChromiumIdentity({
  uid = process.getuid?.(),
  managedProviderIsolationActive = process.env.CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE,
} = {}) {
  if (uid !== 0) return
  const launchContext = managedProviderIsolationActive === "1"
    ? "launch the soak from the slice-owner desktop/runtime instead of the provider sandbox"
    : "launch the soak as the non-root slice user"
  throw new Error(`preflight refuses root Chromium because the renderer sandbox would be disabled; ${launchContext}`)
}

async function allocateRuntime(options) {
  const debugPort = options.debugPort ?? await availablePort(52_000, 55_000)
  const viewerPort = options.viewerPort ?? await availablePort(55_000, 58_000, new Set([debugPort]))
  if (debugPort === viewerPort) throw new Error("debug-port and viewer-port must differ")
  const displayNumber = options.displayNumber ?? await availableDisplay(70, 120)
  return { debugPort, viewerPort, displayNumber }
}

async function assertRuntimeStillAvailable({ debugPort, viewerPort, displayNumber }) {
  for (const port of [debugPort, viewerPort]) {
    if (!await portAvailable(port)) throw new Error(`preflight port ${port} is already in use`)
  }
  for (const candidate of [`/tmp/.X11-unix/X${displayNumber}`, `/tmp/.X${displayNumber}-lock`]) {
    if (await exists(candidate)) throw new Error(`preflight display :${displayNumber} is already in use`)
  }
}

async function cleanupRuntime({ controller, stream, owned, ownedIdentities, selkiesPid, environment, sourceRoot, fixture, stateRoot, allocation }) {
  const actions = []
  try { await controller?.shutdown(); actions.push({ name: "controller-shutdown", ok: true }) } catch (error) {
    actions.push({ name: "controller-shutdown", ok: false, error: bounded(error?.message ?? error) })
  }
  try { await stream?.close(); actions.push({ name: "stream-close", ok: true }) } catch (error) {
    actions.push({ name: "stream-close", ok: false, error: bounded(error?.message ?? error) })
  }
  if (stream?.child) actions.push(await terminateGroup("selkies-stream-process", stream.child))
  if (controller?.child) actions.push(await terminateGroup("browser-controller-process", controller.child))
  if (Number.isSafeInteger(selkiesPid)) try {
    const expected = ownedIdentities.find((entry) => entry.name === "viewer")
    actions.push(await terminateCapturedProcessGroup("selkies-process", expected))
    const stopped = await execJson("/opt/chariox-selkies/bin/python", [path.join(sourceRoot, "slice-selkies.py"), "stop", "--allow-forced"], {
      cwd: sourceRoot, env: environment, timeout: 20_000,
    })
    actions.push({ name: "selkies-state-clear", ok: stopped.stopped === true && stopped.forced !== true })
  } catch (error) {
    actions.push({ name: "selkies-state-clear", ok: false, error: bounded(error?.message ?? error) })
  }
  for (const [name, child] of [...owned.entries()].reverse()) actions.push(await terminateGroup(name, child))
  try { await fixture?.close(); actions.push({ name: "fixture-close", ok: true }) } catch (error) {
    actions.push({ name: "fixture-close", ok: false, error: bounded(error?.message ?? error) })
  }
  if (stateRoot) try { await rm(stateRoot, { recursive: true, force: true }); actions.push({ name: "state-remove", ok: true }) } catch (error) {
    actions.push({ name: "state-remove", ok: false, error: bounded(error?.message ?? error) })
  } else actions.push({ name: "state-remove", ok: true, notCreated: true })
  await sleep(250)
  const identityChecks = await Promise.all(ownedIdentities.filter((entry) => entry.name !== "runner").map(async (expected) => {
    const observed = await processIdentity(expected.pid)
    return { expected, observed, matches: processIdentityMatches(expected, observed) }
  }))
  const remainingPids = identityChecks.filter((entry) => entry.matches).map((entry) => entry.expected.pid)
  const reusedPids = identityChecks.filter((entry) => entry.observed && !entry.matches).map((entry) => entry.expected.pid)
  const ownedPorts = [allocation.debugPort, allocation.viewerPort, fixture?.port].filter(Number.isSafeInteger)
  const portsReleased = await Promise.all(ownedPorts.map(portAvailable))
  const remainingListeners = ownedPorts
    .filter((_port, index) => !portsReleased[index]).map((port) => `127.0.0.1:${port}`)
  const displayReleased = !await exists(`/tmp/.X11-unix/X${allocation.displayNumber}`) && !await exists(`/tmp/.X${allocation.displayNumber}-lock`)
  const stateRemoved = stateRoot == null || !await exists(stateRoot)
  const leakScan = await scanRuntimeLeaks({ stateRoot, allocation })
  const signalActions = actions.filter((entry) => entry.name === "selkies-process" || entry.name === "selkies-stream-process"
    || entry.name === "browser-controller-process" || owned.has(entry.name))
  const pidReuseSafe = signalActions.length > 0 && signalActions.every((entry) => entry.pidReuseSafe === true)
  return {
    schema: "chariox.browser_computer_soak_cleanup.v1",
    at: new Date().toISOString(),
    actions,
    remainingPids,
    reusedPids,
    pidReuseSafe,
    remainingListeners,
    leakScan,
    portsReleased: portsReleased.every(Boolean),
    displayReleased,
    stateRemoved,
    clean: pidReuseSafe && remainingPids.length === 0 && remainingListeners.length === 0 && leakScan.clean
      && displayReleased && stateRemoved && actions.every((entry) => entry.ok),
  }
}

function failedCleanupEvidence(error) {
  return {
    schema: "chariox.browser_computer_soak_cleanup.v1",
    at: new Date().toISOString(),
    actions: [{ name: "cleanup-ledger", ok: false, error: bounded(error?.message ?? error) }],
    remainingPids: [],
    reusedPids: [],
    pidReuseSafe: false,
    remainingListeners: [],
    leakScan: { clean: false, matches: ["cleanup scan did not complete"] },
    portsReleased: false,
    displayReleased: false,
    stateRemoved: false,
    clean: false,
  }
}

async function captureOwnedIdentities(entries) {
  const identities = []
  for (const [name, pid] of entries) {
    if (!Number.isSafeInteger(pid)) throw new Error(`owned ${name} process did not expose a PID`)
    const identity = await processIdentity(pid)
    if (!identity) throw new Error(`owned ${name} process ${pid} disappeared before identity capture`)
    identities.push({ name, ...identity })
  }
  return identities
}

async function mergeSampledIdentities(existing, processes) {
  const identities = [...existing]
  for (const entry of processes) {
    const identity = await processIdentity(entry.pid)
    if (identity && !identities.some((candidate) => processIdentityMatches(candidate, identity))) {
      identities.push({ name: `descendant:${entry.command}`, ...identity })
    }
  }
  return identities
}

async function processIdentity(pid) {
  try {
    const statText = await readFile(`/proc/${pid}/stat`, "utf8")
    const close = statText.lastIndexOf(")")
    const fields = statText.slice(close + 2).split(" ")
    const startedAtTicks = fields[19]
    const processGroupId = Number(fields[2])
    const executable = await readlink(`/proc/${pid}/exe`)
    if (!/^\d+$/.test(startedAtTicks ?? "") || !Number.isSafeInteger(processGroupId) || typeof executable !== "string") return null
    return { pid, startedAtTicks, executable, processGroupId }
  } catch { return null }
}

export function processIdentityMatches(expected, observed) {
  return Boolean(expected && observed && expected.pid === observed.pid
    && expected.startedAtTicks === observed.startedAtTicks && expected.executable === observed.executable
    && expected.processGroupId === observed.processGroupId)
}

function assertOwnedPidHealth(expected, label) {
  if (!processIdentityMatches(expected, processIdentitySync(expected?.pid))) {
    throw new Error(`${label} exited or changed identity during soak`)
  }
}

function processIdentitySync(pid) {
  if (!Number.isSafeInteger(pid)) return null
  try {
    const statText = readFileSync(`/proc/${pid}/stat`, "utf8")
    const close = statText.lastIndexOf(")")
    const fields = statText.slice(close + 2).split(" ")
    const processGroupId = Number(fields[2])
    const executable = readlinkSync(`/proc/${pid}/exe`)
    if (!/^\d+$/.test(fields[19] ?? "") || !Number.isSafeInteger(processGroupId)) return null
    return { pid, startedAtTicks: fields[19], executable, processGroupId }
  } catch { return null }
}

export function createLifecycleGuard() {
  let generation = 0
  let active = true
  return {
    begin() { if (!active) throw new Error("lifecycle is aborted"); generation += 1; return generation },
    accept(candidate) { return active && candidate === generation },
    abort() { if (!active) return false; active = false; generation += 1; return true },
  }
}

async function scanRuntimeLeaks({ stateRoot, allocation }) {
  const { stdout } = await runHostHelper("ps", ["-eo", "pid=,comm=,args="], { timeout: 10_000 })
  const markers = [stateRoot, `Xvfb :${allocation.displayNumber}`, `--display :${allocation.displayNumber}`].filter(Boolean)
  const matches = findRuntimeLeakMatches(stdout, { markers, currentPid: process.pid })
  return { clean: matches.length === 0, matches }
}

export function findRuntimeLeakMatches(processListing, { markers, currentPid, maximumMatches = 100 }) {
  return processListing.split("\n").map((line) => line.trim()).filter(Boolean).flatMap((line) => {
    const parsed = line.match(/^(\d+)\s+(\S+)\s+(.*)$/)
    const pid = Number(parsed?.[1])
    if (!parsed || pid === currentPid || !markers.some((marker) => parsed[3].includes(marker))) return []
    return [{ pid, command: parsed[2] }]
  }).slice(0, maximumMatches)
}

class ControllerClient {
  constructor(child) {
    this.child = child
    this.requestCount = 0
    this.nextId = 0
    this.pending = new Map()
    readJsonLines(child.stdout, (value) => {
      const pending = this.pending.get(value.id)
      if (!pending) return
      this.pending.delete(value.id)
      if (value.ok) pending.resolve(value.result)
      else pending.reject(new Error(`Browser Controller ${value.error?.code ?? "error"}: ${value.error?.message ?? "request failed"}`))
    })
    child.once("exit", () => {
      for (const pending of this.pending.values()) pending.reject(new Error("Browser Controller exited with pending requests"))
      this.pending.clear()
    })
  }

  request(method, params) {
    const id = ++this.nextId
    this.requestCount += 1
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`Browser Controller ${method} timed out`))
      }, 15_000)
      this.pending.set(id, {
        resolve: (value) => { clearTimeout(timer); resolve(value) },
        reject: (error) => { clearTimeout(timer); reject(error) },
      })
      this.child.stdin.write(`${JSON.stringify({ id, method, params })}\n`)
    })
  }

  async shutdown() {
    if (childExited(this.child)) return
    await this.request("shutdown")
    this.child.stdin.end()
    await waitForExit(this.child, 5_000)
  }
}

class StreamClient {
  constructor(child) {
    this.child = child
    this.ready = false
    this.readyAt = null
    this.binaryFrames = 0
    this.binaryBytes = 0
    this.digests = new Set()
    this.textMarkers = new Set()
    this.activityCounter = 0
    this.lastBinaryFrameAt = null
    this.waiters = new Set()
    this.renewal = null
    this.lifecycle = createLifecycleGuard()
    const generation = this.lifecycle.begin()
    readJsonLines(child.stdout, (value) => {
      if (!this.lifecycle.accept(generation)) return
      this.activityCounter += 1
      if (value.kind === "ready") {
        this.ready = true
        this.readyAt ??= Date.now()
      }
      if (value.kind === "binary" && typeof value.data_base64 === "string") {
        this.binaryFrames += 1
        this.binaryBytes += Buffer.byteLength(value.data_base64, "base64")
        this.digests.add(createHash("sha256").update(value.data_base64).digest("hex"))
        this.lastBinaryFrameAt = Date.now()
      }
      if (value.kind === "text" && typeof value.text === "string") this.textMarkers.add(value.text)
      for (const wake of this.waiters) wake()
    })
  }

  async waitReady(timeoutMs) {
    await waitUntil(() => this.ready, timeoutMs, this.waiters, "Selkies stream did not become ready")
  }

  async waitForFrames(count, timeoutMs) {
    await waitUntil(() => this.binaryFrames >= count && this.digests.size >= 2, timeoutMs, this.waiters, "Selkies stream did not deliver changing video frames")
  }

  startRenewal() {
    this.renewal = setInterval(() => {
      if (!childExited(this.child)) this.child.stdin.write('{"kind":"renew"}\n')
    }, 20_000)
    this.renewal.unref()
  }

  stopRenewal() { if (this.renewal) clearInterval(this.renewal) }
  stopObserving() { this.lifecycle.abort() }
  requestKeyframe() { if (!childExited(this.child)) this.child.stdin.write('{"kind":"control","text":"REQUEST_KEYFRAME"}\n') }
  metrics() {
    return {
      binaryFrames: this.binaryFrames,
      binaryBytes: this.binaryBytes,
      changingFrameDigests: this.digests.size,
      textMarkers: [...this.textMarkers].sort(),
      activityCounter: this.activityCounter,
      lastBinaryFrameAt: this.lastBinaryFrameAt ? new Date(this.lastBinaryFrameAt).toISOString() : null,
    }
  }

  async close() {
    if (childExited(this.child)) return
    this.child.stdin.write('{"kind":"close"}\n')
    this.child.stdin.end()
    await waitForExit(this.child, 5_000)
  }
}

async function spawnLogged(name, command, args, { env, cwd, logsRoot }) {
  const log = createWriteStream(path.join(logsRoot, `${name}.log`), { flags: "a", mode: 0o600 })
  const child = spawn(command, args, { cwd, env, detached: true, stdio: ["ignore", "pipe", "pipe"] })
  child.ownedIdentity = await captureSpawnedIdentity(name, child)
  log.once("error", (error) => { child.retainedLogError = bounded(error?.message ?? error) })
  child.retainedLogFinished = new Promise((resolve) => log.once("close", resolve))
  const output = createRedactingTransform()
  output.once("error", (error) => { child.retainedLogError = bounded(error?.message ?? error) })
  child.stdout.pipe(output, { end: false })
  child.stderr.pipe(output, { end: false })
  output.pipe(log)
  let streams = 2
  const endOutput = () => { streams -= 1; if (streams === 0) output.end() }
  child.stdout.once("end", endOutput)
  child.stderr.once("end", endOutput)
  return child
}

async function spawnProtocol(name, command, args, { env, cwd, logsRoot }) {
  const log = createWriteStream(path.join(logsRoot, `${name}.stderr.log`), { flags: "a", mode: 0o600 })
  const child = spawn(command, args, { cwd, env, detached: true, stdio: ["pipe", "pipe", "pipe"] })
  child.ownedIdentity = await captureSpawnedIdentity(name, child)
  log.once("error", (error) => { child.retainedLogError = bounded(error?.message ?? error) })
  child.retainedLogFinished = new Promise((resolve) => log.once("close", resolve))
  const output = createRedactingTransform()
  output.once("error", (error) => { child.retainedLogError = bounded(error?.message ?? error) })
  child.stderr.pipe(output).pipe(log)
  return child
}

async function captureSpawnedIdentity(name, child) {
  if (!Number.isSafeInteger(child.pid)) throw new Error(`${name} did not expose a PID after spawn`)
  await new Promise((resolve, reject) => {
    if (child.spawnfile) return resolve()
    child.once("spawn", resolve)
    child.once("error", reject)
  })
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const identity = await processIdentity(child.pid)
    if (identity?.processGroupId === child.pid) return identity
    await sleep(10)
  }
  try { child.kill("SIGKILL") } catch {}
  throw new Error(`${name} detached process identity could not be captured`)
}

export function createRedactingTransform({ secretValues = retainedSecretValues } = {}) {
  const overlap = Math.max(512, Math.min(65_536, secretValues.reduce((maximum, value) => Math.max(maximum, value.length + 64), 0)))
  let buffered = ""
  let retained = 0
  let truncated = false
  const marker = Buffer.from("\n[REDACTED LOG TRUNCATED]\n")
  const contentLimit = maximumRetainedProcessLogBytes - marker.length
  const emit = function (stream, sanitized) {
    const encoded = Buffer.from(sanitized)
    const available = Math.max(0, contentLimit - retained)
    const emitted = encoded.subarray(0, available)
    retained += emitted.length
    if (emitted.length > 0) stream.push(emitted)
    if (emitted.length < encoded.length) truncated = true
  }
  return new Transform({
    transform(chunk, _encoding, callback) {
      buffered += chunk.toString("utf8")
      const safeLength = Math.max(0, buffered.length - overlap)
      if (safeLength > 0) {
        const sanitized = redactText(buffered.slice(0, safeLength), secretValues)
        buffered = buffered.slice(safeLength)
        emit(this, sanitized)
      }
      callback()
    },
    flush(callback) {
      emit(this, redactText(buffered, secretValues))
      if (truncated) this.push(marker)
      callback()
    },
  })
}

export async function terminateOwnedProcessGroup(name, child, {
  identity = processIdentity,
  signal = process.kill,
  wait = waitForExit,
  waitForLog = waitForRetainedLog,
} = {}) {
  if (!child) return { name, ok: true, notStarted: true }
  if (childExited(child)) {
    await waitForLog(child)
    return { name, ok: !child.retainedLogError, alreadyExited: true, pidReuseSafe: Boolean(child.ownedIdentity), error: child.retainedLogError }
  }
  const expected = child.ownedIdentity
  const beforeTerm = await identity(child.pid)
  if (!processIdentityMatches(expected, beforeTerm) || beforeTerm.processGroupId !== expected.pid) {
    return { name, ok: false, pidReuseSafe: false, error: "owned process identity changed before SIGTERM" }
  }
  let forced = false
  try { signal(-expected.processGroupId, "SIGTERM") } catch (error) {
    if (error?.code !== "ESRCH") return { name, ok: false, pidReuseSafe: true, error: bounded(error?.message ?? error) }
  }
  if (!await wait(child, 5_000, false)) {
    forced = true
    const beforeKill = await identity(child.pid)
    if (!processIdentityMatches(expected, beforeKill) || beforeKill.processGroupId !== expected.pid) {
      return { name, ok: false, forced, pidReuseSafe: false, error: "owned process identity changed before SIGKILL" }
    }
    try { signal(-expected.processGroupId, "SIGKILL") } catch (error) {
      if (error?.code !== "ESRCH") return { name, ok: false, forced, pidReuseSafe: true, error: bounded(error?.message ?? error) }
    }
    await wait(child, 2_000, false)
  }
  await waitForLog(child)
  return { name, ok: childExited(child) && !child.retainedLogError, forced, pidReuseSafe: true, error: child.retainedLogError }
}

const terminateGroup = terminateOwnedProcessGroup

export async function terminateCapturedProcessGroup(name, expected, {
  identity = processIdentity,
  signal = process.kill,
} = {}) {
  if (!expected) return { name, ok: false, pidReuseSafe: false, error: "owned process identity was not captured" }
  const observed = await identity(expected.pid)
  if (observed == null) return { name, ok: true, alreadyExited: true, pidReuseSafe: true }
  if (!processIdentityMatches(expected, observed) || observed.processGroupId !== expected.pid) {
    return { name, ok: false, pidReuseSafe: false, error: "owned process identity changed before SIGTERM" }
  }
  try { signal(-expected.processGroupId, "SIGTERM") } catch (error) {
    if (error?.code !== "ESRCH") return { name, ok: false, pidReuseSafe: true, error: bounded(error?.message ?? error) }
  }
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (!processIdentityMatches(expected, await identity(expected.pid))) return { name, ok: true, forced: false, pidReuseSafe: true }
    await sleep(50)
  }
  const beforeKill = await identity(expected.pid)
  if (!processIdentityMatches(expected, beforeKill) || beforeKill.processGroupId !== expected.pid) {
    return { name, ok: false, forced: true, pidReuseSafe: false, error: "owned process identity changed before SIGKILL" }
  }
  try { signal(-expected.processGroupId, "SIGKILL") } catch (error) {
    if (error?.code !== "ESRCH") return { name, ok: false, forced: true, pidReuseSafe: true, error: bounded(error?.message ?? error) }
  }
  for (let attempt = 0; attempt < 40; attempt += 1) {
    if (!processIdentityMatches(expected, await identity(expected.pid))) return { name, ok: true, forced: true, pidReuseSafe: true }
    await sleep(50)
  }
  return { name, ok: false, forced: true, pidReuseSafe: true, error: "owned process remained after SIGKILL" }
}

async function waitForRetainedLog(child) {
  if (!child?.retainedLogFinished) return
  const closed = await Promise.race([
    child.retainedLogFinished.then(() => true),
    sleep(2_000).then(() => false),
  ])
  if (!closed) child.retainedLogError ??= "retained log did not close within 2000ms"
}

async function resourceSnapshot(label, rootPids, diskPath) {
  const { stdout } = await runHostHelper("ps", ["-eo", "pid=,ppid=,rss=,%cpu=,comm="], { timeout: 10_000 })
  const rows = stdout.split("\n").map((line) => line.trim().split(/\s+/, 5)).filter((parts) => parts.length === 5).map(([pid, ppid, rss, cpu, command]) => ({
    pid: Number(pid), ppid: Number(ppid), rssKb: Number(rss), cpuPercent: Number(cpu), command,
  })).filter((row) => Number.isSafeInteger(row.pid) && Number.isSafeInteger(row.ppid))
  const ownedIds = descendantIds(rows, rootPids)
  const ownedRows = rows.filter((row) => ownedIds.has(row.pid))
  const disk = await import("node:fs/promises").then(({ statfs }) => statfs(diskPath))
  const openFiles = (await Promise.all([...ownedIds].map(async (pid) => {
    try { return (await readdir(`/proc/${pid}/fd`)).length } catch { return 0 }
  }))).reduce((sum, count) => sum + count, 0)
  const network = await networkSnapshot(ownedIds)
  const memory = await linuxMemorySnapshot()
  return {
    label,
    at: new Date().toISOString(),
    host: {
      totalMemoryBytes: os.totalmem(),
      freeMemoryBytes: os.freemem(),
      loadAverage: os.loadavg(),
      cpuCount: os.cpus().length,
      swapTotalBytes: memory.swapTotalBytes,
      swapFreeBytes: memory.swapFreeBytes,
    },
    disk: {
      path: diskPath,
      availableBytes: Number(disk.bavail) * Number(disk.bsize),
      totalBytes: Number(disk.blocks) * Number(disk.bsize),
    },
    network,
    owned: {
      rootPids,
      processCount: ownedRows.length,
      rssBytes: ownedRows.reduce((sum, row) => sum + row.rssKb * 1024, 0),
      cpuPercent: ownedRows.reduce((sum, row) => sum + row.cpuPercent, 0),
      openFiles,
      processes: ownedRows,
    },
  }
}

async function networkSnapshot(ownedIds) {
  const text = await readFile("/proc/net/dev", "utf8")
  let receivedBytes = 0
  let transmittedBytes = 0
  for (const line of text.split("\n").slice(2)) {
    const match = line.match(/^\s*([^:]+):\s*(\d+)(?:\s+\d+){7}\s+(\d+)/)
    if (!match) continue
    receivedBytes += Number(match[2])
    transmittedBytes += Number(match[3])
  }
  const attribution = await networkNamespaceAttribution(ownedIds)
  return { receivedBytes, transmittedBytes, totalBytes: receivedBytes + transmittedBytes, attribution }
}

export async function networkNamespaceAttribution(ownedIds, maximumForeignPids = 32, {
  currentPid = process.pid,
  listProc = () => readdir("/proc"),
  readNamespace = (candidate) => readlink(candidate),
} = {}) {
  let namespace
  try { namespace = await readNamespace(`/proc/${currentPid}/ns/net`) } catch {
    return { exclusive: false, foreignPids: [], reason: "network namespace identity unavailable" }
  }
  const foreignPids = []
  const unreadablePids = []
  const mismatchedOwnedPids = []
  let names
  try { names = await listProc() } catch {
    return { exclusive: false, namespace, foreignPids, unreadablePids, reason: "network namespace inventory unavailable" }
  }
  for (const name of names) {
    if (!/^\d+$/.test(name)) continue
    const pid = Number(name)
    if (pid === currentPid) continue
    let observed
    try { observed = await readNamespace(`/proc/${pid}/ns/net`) } catch {
      unreadablePids.push(pid)
      continue
    }
    if (ownedIds.has(pid)) {
      if (observed !== namespace) mismatchedOwnedPids.push(pid)
    } else if (observed === namespace) {
      foreignPids.push(pid)
      if (foreignPids.length >= maximumForeignPids) break
    }
  }
  return {
    exclusive: foreignPids.length === 0 && unreadablePids.length === 0 && mismatchedOwnedPids.length === 0,
    namespace,
    foreignPids,
    unreadablePids,
    mismatchedOwnedPids,
    reason: unreadablePids.length > 0 || mismatchedOwnedPids.length > 0 ? "network namespace inventory incomplete" : undefined,
  }
}

async function ownedDiskBytes(roots, maximumEntries = 100_000) {
  let bytes = 0
  let entries = 0
  const pending = [...roots]
  while (pending.length > 0) {
    const candidate = pending.pop()
    let metadata
    try { metadata = await lstat(candidate) } catch { continue }
    entries += 1
    if (entries > maximumEntries) throw new Error(`owned disk inventory exceeded ${maximumEntries} entries`)
    if (metadata.isSymbolicLink()) continue
    bytes += metadata.size
    if (!metadata.isDirectory()) continue
    for (const name of await readdir(candidate)) pending.push(path.join(candidate, name))
  }
  return bytes
}

async function linuxMemorySnapshot() {
  try {
    const text = await readFile("/proc/meminfo", "utf8")
    const kib = (name) => Number(text.match(new RegExp(`^${name}:\\s+(\\d+) kB$`, "m"))?.[1] ?? 0)
    return { swapTotalBytes: kib("SwapTotal") * 1024, swapFreeBytes: kib("SwapFree") * 1024 }
  } catch { return { swapTotalBytes: 0, swapFreeBytes: 0 } }
}

function resourceBoundsPass(metrics, limits) {
  return metrics.peakOwnedRssBytes <= limits.maxRssBytes
    && metrics.peakOwnedCpuPercent <= limits.maxCpuPercent
    && metrics.peakOwnedProcessCount <= limits.maxProcesses
    && metrics.diskGrowthBytes <= limits.maxDiskGrowthBytes
    && metrics.peakOwnedOpenFiles <= limits.maxOpenFiles
    && metrics.networkBytes <= limits.maxNetworkBytes
}

function descendantIds(rows, roots) {
  const ids = new Set(roots.filter(Number.isSafeInteger))
  let changed = true
  while (changed) {
    changed = false
    for (const row of rows) if (ids.has(row.ppid) && !ids.has(row.pid)) { ids.add(row.pid); changed = true }
  }
  return ids
}

export async function captureSourceIdentity(repoRoot, {
  exec = execFileAsync,
  baseEnvironment = process.env,
} = {}) {
  const run = (command, args, options) => runHostHelper(command, args, options, { exec, baseEnvironment })
  const [{ stdout: commit }, { stdout: tree }, { stdout: branch }, { stdout: status }] = await Promise.all([
    run("git", ["-c", `safe.directory=${repoRoot}`, "rev-parse", "HEAD"], { cwd: repoRoot, timeout: 10_000 }),
    run("git", ["-c", `safe.directory=${repoRoot}`, "rev-parse", "HEAD^{tree}"], { cwd: repoRoot, timeout: 10_000 }),
    run("git", ["-c", `safe.directory=${repoRoot}`, "branch", "--show-current"], { cwd: repoRoot, timeout: 10_000 }),
    run("git", ["-c", `safe.directory=${repoRoot}`, "status", "--short"], { cwd: repoRoot, timeout: 10_000 }),
  ])
  return { commit: commit.trim(), tree: tree.trim(), branch: branch.trim(), dirty: status.trim() !== "" }
}

async function assertSourceUnchanged(repoRoot, expected) {
  const observed = await captureSourceIdentity(repoRoot)
  if (observed.dirty || observed.commit !== expected?.commit || observed.tree !== expected?.tree || observed.branch !== expected?.branch) {
    throw new Error("source changed after provenance capture")
  }
}

async function captureProvenance({ options, repoRoot, source }) {
  const image = await resolveVerifiedImage({
    imageRef: options.imageRef,
    signatureKey: options.imageSignatureKey,
    engine: options.containerEngine,
  })
  const protocolSource = await readFile(path.join(repoRoot, "apps", "kernel", "src", "local", "api", "types.rs"), "utf8")
  const protocol = Number(protocolSource.match(/LOCAL_DAEMON_PROTOCOL_VERSION:\s*u32\s*=\s*(\d+)/)?.[1])
  if (!Number.isSafeInteger(protocol)) throw new Error("could not identify the local daemon protocol version")
  assertCurrentContainerIdentity(options.runtimeContainerId)
  const runtimeImage = await inspectDigestBoundRuntime({ containerId: options.runtimeContainerId, image, source })
  return {
    schema: "chariox.browser_computer_soak_provenance.v1",
    capturedAt: new Date().toISOString(),
    source,
    image,
    runtimeImage,
    limits: options.limits,
    viewer: { backend: options.viewerBackend },
    localDaemonProtocolVersion: protocol,
  }
}

async function requireGateReceipts(evidenceRoot, provenance, { smoke }) {
  const paths = buildGateReceiptPaths(evidenceRoot)
  const preflight = await readJson(paths.preflight, "preflight receipt")
  const smokeReceipt = smoke ? await readJson(paths.smoke, "smoke receipt") : {
    schema: "chariox.browser_computer_soak_gate_receipt.v1",
    phase: "smoke",
    status: "passed",
    completedAt: new Date().toISOString(),
    fingerprint: gateFingerprint(provenance),
    cleanup: { clean: true },
  }
  validateGatePrerequisites({ preflight, smoke: smokeReceipt, provenance })
}

async function writeGateReceipt(evidenceRoot, phase, report, provenance) {
  const paths = buildGateReceiptPaths(evidenceRoot)
  await mkdir(evidenceRoot, { recursive: true, mode: 0o700 })
  await writeJson(paths[phase], {
    schema: "chariox.browser_computer_soak_gate_receipt.v1",
    phase,
    status: "passed",
    completedAt: report.completedAt ?? report.at,
    fingerprint: gateFingerprint(provenance),
    runDir: report.paths?.runDir ?? null,
    cleanup: phase === "smoke" ? report.cleanup : undefined,
  })
}

async function readJson(candidate, label) {
  try { return JSON.parse(await readFile(candidate, "utf8")) } catch (error) {
    throw new Error(`${label} is missing or invalid: ${bounded(error?.message ?? error)}`)
  }
}

export function finalGateEligibility(backend, protocol, durationSeconds = MINIMUM_FINAL_GATE_DURATION_SECONDS) {
  if (backend === "novnc") return { eligible: false, reason: "novnc_not_final_gate" }
  if (protocol < SELKIES_REQUIRED_PROTOCOL_VERSION) return { eligible: false, reason: `protocol_${SELKIES_REQUIRED_PROTOCOL_VERSION}_not_integrated` }
  if (durationSeconds < MINIMUM_FINAL_GATE_DURATION_SECONDS) return { eligible: false, reason: `duration_below_${MINIMUM_FINAL_GATE_DURATION_SECONDS}_seconds` }
  return { eligible: true, reason: null }
}

function detachedArgs({ options, paths, allocation }) {
  return [
    "--internal-run",
    ...(options.smoke ? ["--smoke"] : [
      "--duration-seconds", String(options.durationSeconds),
      "--activity-interval-seconds", String(options.activityIntervalSeconds),
      "--sample-interval-seconds", String(options.sampleIntervalSeconds),
    ]),
    "--evidence-root", options.evidenceRoot,
    "--run-dir", paths.runDir,
    "--display-number", String(allocation.displayNumber),
    "--debug-port", String(allocation.debugPort),
    "--viewer-port", String(allocation.viewerPort),
    "--viewer-backend", options.viewerBackend,
    "--runtime-container-id", options.runtimeContainerId,
    "--launch-gate", paths.launchGate,
    "--max-cadence-gap-seconds", String(options.limits.maxCadenceGapMs / 1_000),
    "--max-rss-mib", String(options.limits.maxRssBytes / 1024 / 1024),
    "--max-cpu-percent", String(options.limits.maxCpuPercent),
    "--max-processes", String(options.limits.maxProcesses),
    "--max-disk-growth-mib", String(options.limits.maxDiskGrowthBytes / 1024 / 1024),
    "--max-open-files", String(options.limits.maxOpenFiles),
    "--max-network-mib", String(options.limits.maxNetworkBytes / 1024 / 1024),
  ]
}

function commandEvidence({ options, paths, allocation }) {
  return [
    "node apps/cli/scripts/live-browser-computer-soak.mjs",
    options.smoke ? "--smoke" : `--duration-seconds ${options.durationSeconds}`,
    `--activity-interval-seconds ${options.activityIntervalSeconds}`,
    `--sample-interval-seconds ${options.sampleIntervalSeconds}`,
    `--evidence-root ${shellQuote(options.evidenceRoot)}`,
    `--run-dir ${shellQuote(paths.runDir)}`,
    `--display-number ${allocation.displayNumber}`,
    `--debug-port ${allocation.debugPort}`,
    `--viewer-port ${allocation.viewerPort}`,
    `--viewer-backend ${options.viewerBackend}`,
    "--image-ref [VERIFIED_IN_PROVENANCE]",
    "--image-signature-key [REDACTED_PATH]",
    `--container-engine ${options.containerEngine}`,
    `--max-cadence-gap-seconds ${options.limits.maxCadenceGapMs / 1_000}`,
    `--max-rss-mib ${options.limits.maxRssBytes / 1024 / 1024}`,
    `--max-cpu-percent ${options.limits.maxCpuPercent}`,
    `--max-processes ${options.limits.maxProcesses}`,
    `--max-disk-growth-mib ${options.limits.maxDiskGrowthBytes / 1024 / 1024}`,
    `--max-open-files ${options.limits.maxOpenFiles}`,
    `--max-network-mib ${options.limits.maxNetworkBytes / 1024 / 1024}`,
  ].join(" ")
}

async function startFixtureServer() {
  let marker = "SOAK-00000000"
  let bytes = 0
  const server = http.createServer((request, response) => {
    bytes += Buffer.byteLength(`${request.method ?? ""} ${request.url ?? ""}`)
    if (request.url === "/health") {
      response.writeHead(200, { "content-type": "text/plain", "cache-control": "no-store" })
      bytes += Buffer.byteLength(marker)
      return response.end(marker)
    }
    if (request.url?.startsWith("/mark?")) {
      marker = new URL(request.url, "http://127.0.0.1").searchParams.get("value") ?? marker
      response.writeHead(204, { "cache-control": "no-store" })
      return response.end()
    }
    response.writeHead(200, { "content-type": "text/html", "cache-control": "no-store" })
    const body = `<!doctype html><title>Chariox active soak</title><style>body{font:24px sans-serif;background:#14213d;color:#fff}main{padding:60px}input{font-size:28px;width:520px}.pulse{width:120px;height:120px;background:#fca311;animation:pulse 1s infinite alternate}@keyframes pulse{to{transform:translateX(320px);background:#2ec4b6}}</style><main><label>Soak marker <input id="marker" value="${marker}"></label><p id="echo">${marker}</p><div class="pulse"></div></main><script>const field=document.querySelector('#marker');field.addEventListener('input',()=>{document.querySelector('#echo').textContent=field.value;document.title=field.value;fetch('/mark?value='+encodeURIComponent(field.value)).catch(()=>{})})</script>`
    bytes += Buffer.byteLength(body)
    response.end(body)
  })
  await new Promise((resolve, reject) => server.once("error", reject).listen(0, "127.0.0.1", resolve))
  const port = server.address().port
  const url = `http://127.0.0.1:${port}/`
  return {
    url,
    port,
    metrics: () => ({ bytes }),
    close: () => new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve())),
  }
}

async function waitForDisplay(display, env) {
  await retry(async () => {
    await execFileAsync("xdpyinfo", ["-display", display], { env, timeout: 2_000 })
  }, 10_000, `X display ${display} did not become ready`)
}

async function waitForHttp(url, timeoutMs) {
  await retry(async () => {
    const response = await fetch(url, { signal: AbortSignal.timeout(1_000) })
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
  }, timeoutMs, `${url} did not become ready`)
}

async function retry(operation, timeoutMs, message) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    try { return await operation() } catch (error) { last = error }
    await sleep(100)
  }
  throw new Error(`${message}: ${bounded(last?.message ?? last)}`)
}

export function assertProcessHealth(owned, controller, stream, now = Date.now()) {
  for (const [name, child] of owned) {
    if (childExited(child)) throw new Error(`${name} exited during soak with ${childExitDescription(child)}`)
    if (child.retainedLogError) throw new Error(`${name} retained log failed: ${child.retainedLogError}`)
  }
  if (childExited(controller.child)) throw new Error(`Browser Controller exited during soak with ${childExitDescription(controller.child)}`)
  if (controller.child.retainedLogError) throw new Error(`Browser Controller retained log failed: ${controller.child.retainedLogError}`)
  if (childExited(stream.child)) throw new Error(`Selkies stream exited during soak with ${childExitDescription(stream.child)}`)
  if (stream.child.retainedLogError) throw new Error(`Selkies stream retained log failed: ${stream.child.retainedLogError}`)
  if (stream.readyAt && !stream.lastBinaryFrameAt && now - stream.readyAt > 30_000) {
    throw new Error("Selkies stream did not deliver its first video frame")
  }
  if (stream.lastBinaryFrameAt && now - stream.lastBinaryFrameAt > 30_000) {
    throw new Error("Selkies stream stopped delivering video frames")
  }
}

export async function verifyComputerInputEffect({ iteration, env, cwd }, { exec = execFileAsync } = {}) {
  const readPointer = async () => {
    const { stdout } = await exec("xdotool", ["getmouselocation", "--shell"], { cwd, env, timeout: 10_000 })
    const x = Number(String(stdout).match(/^X=(\d+)$/m)?.[1])
    const y = Number(String(stdout).match(/^Y=(\d+)$/m)?.[1])
    if (!Number.isSafeInteger(x) || !Number.isSafeInteger(y)) throw new Error("Computer input could not read the physical pointer position")
    return { x, y }
  }
  const before = await readPointer()
  let target = { x: 25 + iteration % 700, y: 30 + iteration % 500 }
  if (target.x === before.x && target.y === before.y) target = { x: (target.x + 37) % 780, y: (target.y + 41) % 580 }
  await exec("xdotool", ["mousemove", "--sync", String(target.x), String(target.y)], { cwd, env, timeout: 10_000 })
  const after = await readPointer()
  if (after.x !== target.x || after.y !== target.y || (after.x === before.x && after.y === before.y)) {
    throw new Error("Computer physical pointer did not move to the requested coordinates")
  }
  return { before, requested: target, after }
}

export async function baselineResourceSnapshot(paths, {
  rootPids = [process.pid],
  snapshot = resourceSnapshot,
} = {}) {
  return snapshot("preflight", rootPids, paths.runDir)
}

function readJsonLines(stream, consume) {
  let buffer = ""
  stream.setEncoding("utf8")
  stream.on("data", (chunk) => {
    buffer += chunk
    for (;;) {
      const index = buffer.indexOf("\n")
      if (index < 0) break
      const line = buffer.slice(0, index)
      buffer = buffer.slice(index + 1)
      if (!line.trim()) continue
      try { consume(JSON.parse(line)) } catch {}
    }
  })
}

async function waitUntil(condition, timeoutMs, waiters, message) {
  const deadline = Date.now() + timeoutMs
  while (!condition()) {
    const remaining = deadline - Date.now()
    if (remaining <= 0) throw new Error(message)
    await new Promise((resolve) => {
      const timer = setTimeout(done, remaining)
      function done() { clearTimeout(timer); waiters.delete(done); resolve() }
      waiters.add(done)
    })
  }
}

async function execJson(command, args, options) {
  const { stdout } = await execFileAsync(command, args, options)
  return JSON.parse(stdout.trim().split("\n").at(-1))
}

async function writeJson(target, value) {
  const temporary = `${target}.${process.pid}.tmp`
  try {
    await writeFile(temporary, `${JSON.stringify(redactEvidence(value), null, 2)}\n`, { mode: 0o600 })
    await chmod(temporary, 0o600)
    await rename(temporary, target)
  } catch (error) {
    await rm(temporary, { force: true }).catch(() => {})
    throw error
  }
}

async function availablePort(start, end, excluded = new Set()) {
  for (let port = start; port < end; port += 1) if (!excluded.has(port) && await portAvailable(port)) return port
  throw new Error(`no available port from ${start} to ${end - 1}`)
}

function portAvailable(port) {
  return new Promise((resolve) => {
    const server = net.createServer()
    server.unref()
    server.once("error", () => resolve(false))
    server.listen(port, "127.0.0.1", () => server.close(() => resolve(true)))
  })
}

async function availableDisplay(start, end) {
  for (let display = start; display < end; display += 1) {
    if (!await exists(`/tmp/.X11-unix/X${display}`) && !await exists(`/tmp/.X${display}-lock`)) return display
  }
  throw new Error(`no available X display from :${start} to :${end - 1}`)
}

async function waitForExit(child, timeoutMs, reject = true) {
  if (childExited(child)) return true
  const exited = await Promise.race([
    new Promise((resolve) => child.once("exit", () => resolve(true))),
    sleep(timeoutMs).then(() => false),
  ])
  if (!exited && reject) throw new Error(`process ${child.pid} did not exit within ${timeoutMs}ms`)
  return exited
}

async function persistStartupFailure({ error, options, allocation, paths, source, baseline, startedAt }) {
  const completedAt = new Date().toISOString()
  const marker = bounded(error?.stack ?? error)
  const failedPid = Number.isSafeInteger(error?.detachedChildPid) ? error.detachedChildPid : process.pid
  const cleanup = error?.terminalCleanup ?? {
    schema: "chariox.browser_computer_soak_cleanup.v1",
    at: completedAt,
    phase: "startup",
    actions: [{ name: "terminal-lifecycle-boundary", ok: true, ownedRuntimeStarted: false }],
    remainingPids: [],
    portsReleased: null,
    displayReleased: null,
    stateRemoved: null,
    verificationCompleted: true,
    clean: true,
  }
  const result = {
    schema,
    status: "failed",
    phase: "startup",
    pid: failedPid,
    startedAt,
    completedAt,
    durationSeconds: options.durationSeconds,
    smoke: options.smoke,
    source,
    allocation,
    paths,
    resources: { baseline },
    cleanup,
    failure: marker,
  }
  await writeFile(paths.pid, `${failedPid}\n`, { mode: 0o600 })
  await writeJson(paths.cleanup, cleanup)
  await writeJson(paths.failure, {
    schema: "chariox.browser_computer_soak_failure.v1",
    status: "failed",
    phase: "startup",
    at: completedAt,
    marker,
  })
  await writeJson(paths.result, result)
  await writeJson(paths.status, {
    schema,
    status: "failed",
    phase: "startup",
    pid: failedPid,
    startedAt,
    completedAt,
    result: paths.result,
    cleanup: paths.cleanup,
  })
}

function childExited(child) { return child?.exitCode != null || child?.signalCode != null }
function childExitDescription(child) {
  return child.signalCode ? `signal ${child.signalCode}` : `exit code ${child.exitCode}`
}

function pids(owned) { return [...owned.values()].map((child) => child?.pid).filter(Number.isSafeInteger) }
export function processIsRunning(pid, {
  probe = (candidate, signal) => process.kill(candidate, signal),
  readStat = (candidate) => readFileSync(candidate, "utf8"),
} = {}) {
  try { probe(pid, 0) } catch { return false }
  try {
    const statLine = readStat(`/proc/${pid}/stat`)
    const commandEnd = statLine.lastIndexOf(")")
    const state = commandEnd === -1 ? "" : statLine.slice(commandEnd + 1).trimStart().charAt(0)
    return state !== "Z" && state !== "X"
  } catch {
    return false
  }
}

function running(pid) { return processIsRunning(pid) }
async function exists(candidate) { try { await stat(candidate); return true } catch { return false } }
const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds))
const bounded = (value, limit = 2_000) => redactText(value).slice(-limit)
const evidenceJson = (value) => JSON.stringify(redactEvidence(value))
const shellQuote = (value) => `'${String(value).replaceAll("'", "'\\''")}'`
