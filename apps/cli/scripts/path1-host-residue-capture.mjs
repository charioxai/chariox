#!/usr/bin/env node

import { execFile as execFileCallback } from "node:child_process"
import { lstat, readFile, realpath } from "node:fs/promises"
import { isAbsolute, resolve } from "node:path"
import { promisify } from "node:util"
import { pathToFileURL } from "node:url"
import { validateCaptureOutput, writeCaptureOutput } from "./path1-provider-rebuild-capture.mjs"

const execFile = promisify(execFileCallback)
const HOST_CAPTURE_SCHEMA = "chariox.path1-host-identity-capture/v1"
const EVIDENCE_SCHEMA = "chariox.path1-rebuild-evidence/v1"
const RESIDUE_SCHEMA = "chariox.path1-host-residue-capture/v1"
const SYSTEMD = "/usr/bin/systemctl"
const SYSTEMD_PROPERTIES = ["Id", "InvocationID", "MainPID", "ActiveState"]
const SYSTEMD_STATES = new Set(["active", "reloading", "inactive", "failed", "activating", "deactivating"])
const SERVICE_ALLOWLIST = new Set([
  "chariox-path1-managed-bootstrap.service",
  "chariox-managed-bootstrap.service",
  "chariox-disposable-worker-bootstrap.service",
  "chariox-rootless-docker.service",
  "chariox-slice-broker.service",
])
const STATE_PATHS = new Set(["/var/lib/chariox", "/home/chariox"])
const RELEASES_ROOT = "/usr/lib/chariox/releases"
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/
const MACHINE_ID = /^[0-9a-f]{32}$/
const INVOCATION_ID = /^[0-9a-f]{32}$/
const STAT_INSTANCE = /^stat:(0|[1-9][0-9]*):(0|[1-9][0-9]*)$/

export class Path1HostResidueCaptureError extends Error {
  constructor(code, message) {
    super(message)
    this.name = "Path1HostResidueCaptureError"
    this.code = code
  }
}

function fail(code, message) {
  throw new Path1HostResidueCaptureError(code, message)
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value)
}

function requireText(value, label, pattern) {
  if (typeof value !== "string" || !pattern.test(value)) {
    fail("before_identity_invalid", `invalid retained ${label}`)
  }
  return value
}

function requireInstanceId(value, label) {
  if (typeof value !== "string" || !STAT_INSTANCE.test(value)) {
    fail("before_identity_invalid", `invalid retained ${label} instance ID`)
  }
  return value
}

function normalizeBeforeIdentity(beforeCapture) {
  if (!isRecord(beforeCapture)) fail("before_identity_invalid", "retained before capture must be an object")
  let identity = beforeCapture
  if ("schema" in beforeCapture) {
    const rawCapture = beforeCapture.schema === HOST_CAPTURE_SCHEMA
    const receipt = beforeCapture.schema === EVIDENCE_SCHEMA && beforeCapture.kind === "before"
    if (!rawCapture && !receipt) fail("before_identity_invalid", "unsupported retained before capture schema")
    identity = beforeCapture.identity
  }
  if (!isRecord(identity)) fail("before_identity_invalid", "retained before identity is missing")
  const bootId = requireText(identity.bootId, "boot ID", UUID)
  const machineId = requireText(identity.machineId, "machine ID", MACHINE_ID)

  if (!isRecord(identity.release)) fail("before_identity_invalid", "retained release identity is missing")
  const digest = requireText(identity.release.digest, "release digest", /^sha256:[0-9a-f]{64}$/)
  const releaseInstanceId = requireInstanceId(identity.release.instanceId, "release")
  const releaseHex = digest.slice("sha256:".length)
  const releasePath = `${RELEASES_ROOT}/${releaseHex}`

  if (!Array.isArray(identity.serviceInstances) || identity.serviceInstances.length === 0 || identity.serviceInstances.length > SERVICE_ALLOWLIST.size) {
    fail("before_identity_invalid", "retained service identities are invalid")
  }
  const services = identity.serviceInstances.map((item) => {
    if (!isRecord(item) || !SERVICE_ALLOWLIST.has(item.unit)) fail("before_identity_invalid", "retained service is not allowlisted")
    return {
      unit: item.unit,
      invocationId: requireText(item.invocationId, "service invocation ID", INVOCATION_ID),
    }
  })
  if (new Set(services.map(({ unit }) => unit)).size !== services.length) fail("before_identity_invalid", "duplicate retained service unit")
  if (new Set(services.map(({ invocationId }) => invocationId)).size !== services.length) fail("before_identity_invalid", "duplicate retained service invocation ID")

  if (!Array.isArray(identity.processes) || identity.processes.length === 0 || identity.processes.length > 32) {
    fail("before_identity_invalid", "retained process identities are invalid")
  }
  const processes = identity.processes.map((value) => {
    if (typeof value !== "string") fail("before_identity_invalid", "retained process identity must be text")
    const match = /^([0-9a-f-]{36}):([1-9][0-9]*):(0|[1-9][0-9]*)$/.exec(value)
    if (!match || !UUID.test(match[1]) || match[1] !== bootId) fail("before_identity_invalid", "retained process identity is malformed")
    const pid = Number(match[2])
    if (!Number.isSafeInteger(pid) || pid <= 0) fail("before_identity_invalid", "retained process PID is invalid")
    return { identity: value, bootId: match[1], pid, startTimeTicks: match[3] }
  })
  if (new Set(processes.map(({ identity: value }) => value)).size !== processes.length) fail("before_identity_invalid", "duplicate retained process identity")

  if (!Array.isArray(identity.stateInstances) || identity.stateInstances.length !== STATE_PATHS.size) {
    fail("before_identity_invalid", "retained state directory identities are invalid")
  }
  const states = identity.stateInstances.map((item) => {
    if (!isRecord(item) || !STATE_PATHS.has(item.path)) fail("before_identity_invalid", "retained state path is not allowlisted")
    return { path: item.path, instanceId: requireInstanceId(item.instanceId, item.path) }
  })
  if (new Set(states.map(({ path }) => path)).size !== STATE_PATHS.size) fail("before_identity_invalid", "retained state paths are incomplete or duplicated")

  return { bootId, machineId, release: { digest, instanceId: releaseInstanceId, path: releasePath }, services, processes, states }
}

function errorCode(error) {
  return typeof error?.code === "string" ? error.code : "unknown"
}

async function readSmallText(fileSystem, path, label, maxBytes) {
  let contents
  try {
    contents = await fileSystem.readFile(path)
  } catch (error) {
    fail("probe_read_failed", `could not read ${label} (${errorCode(error)})`)
  }
  const bytes = Buffer.isBuffer(contents) ? contents : Buffer.from(String(contents))
  if (bytes.length > maxBytes) fail("probe_read_failed", `${label} exceeded its read bound`)
  return bytes.toString("utf8").trim()
}

async function readHostGeneration(fileSystem) {
  const bootId = await readSmallText(fileSystem, "/proc/sys/kernel/random/boot_id", "boot ID", 128)
  const machineId = await readSmallText(fileSystem, "/etc/machine-id", "machine ID", 128)
  if (!UUID.test(bootId) || !MACHINE_ID.test(machineId)) fail("probe_read_failed", "host identity files returned invalid values")
  return { bootId, machineId }
}

function parseSystemdOutput(stdout, unit) {
  if (typeof stdout !== "string" || Buffer.byteLength(stdout, "utf8") > 8192) fail("systemd_read_failed", "systemd response exceeded its read bound")
  const values = new Map()
  for (const line of stdout.split(/\r?\n/).filter(Boolean)) {
    const separator = line.indexOf("=")
    if (separator <= 0) fail("systemd_read_failed", "systemd response was malformed")
    const key = line.slice(0, separator)
    if (!SYSTEMD_PROPERTIES.includes(key) || values.has(key)) fail("systemd_read_failed", "systemd response had unexpected properties")
    values.set(key, line.slice(separator + 1))
  }
  if (SYSTEMD_PROPERTIES.some((key) => !values.has(key))) fail("systemd_read_failed", "systemd response omitted identity properties")
  const invocationId = values.get("InvocationID")
  const pidText = values.get("MainPID")
  const activeState = values.get("ActiveState")
  if (invocationId !== "" && !INVOCATION_ID.test(invocationId)) fail("systemd_read_failed", "systemd returned an invalid invocation ID")
  if (!/^(0|[1-9][0-9]*)$/.test(pidText) || !Number.isSafeInteger(Number(pidText))) fail("systemd_read_failed", "systemd returned an invalid main PID")
  if (!SYSTEMD_STATES.has(activeState)) fail("systemd_read_failed", "systemd returned an invalid active state")
  return { unit, id: values.get("Id"), invocationId, mainPid: Number(pidText), activeState }
}

async function readService(runCommand, retained) {
  const args = ["show", "--no-pager", ...SYSTEMD_PROPERTIES.map((property) => `--property=${property}`), retained.unit]
  let result
  try {
    result = await runCommand(SYSTEMD, args)
  } catch (error) {
    fail("systemd_read_failed", `systemd identity read failed (${errorCode(error)})`)
  }
  if (!isRecord(result) || result.exitCode !== 0 || typeof result.stdout !== "string") fail("systemd_read_failed", "systemd identity read failed")
  return parseSystemdOutput(result.stdout, retained.unit)
}

function serviceStatus(retained, observed) {
  const stopped = observed.invocationId === "" && observed.mainPid === 0 && ["inactive", "failed"].includes(observed.activeState)
  if (observed.id === "" && stopped) return "missing"
  if (observed.id !== retained.unit) return "changed"
  if (stopped) return "missing"
  if (observed.invocationId !== retained.invocationId) return "changed"
  if (observed.mainPid === 0 || observed.activeState !== "active") return "changed"
  return "present"
}

function parseProcStat(contents, expectedPid) {
  const text = Buffer.isBuffer(contents) ? contents.toString("utf8") : String(contents)
  if (Buffer.byteLength(text, "utf8") > 4096) fail("process_stat_invalid", "process stat exceeded its read bound")
  const open = text.indexOf("(")
  const close = text.lastIndexOf(")")
  if (open < 1 || close <= open || !/^\d+$/.test(text.slice(0, open).trim())) fail("process_stat_invalid", "process stat was malformed")
  const pid = Number(text.slice(0, open).trim())
  const fields = text.slice(close + 1).trim().split(/\s+/)
  if (pid !== expectedPid || fields.length < 20 || fields[0].length !== 1 || !/^\d+$/.test(fields[1]) || !/^\d+$/.test(fields[19])) {
    fail("process_stat_invalid", "process stat identity fields were malformed")
  }
  return { pid, state: fields[0], parentPid: Number(fields[1]), startTimeTicks: fields[19] }
}

async function readProcess(fileSystem, retained) {
  const path = `/proc/${retained.pid}/stat`
  let contents
  try {
    contents = await fileSystem.readFile(path)
  } catch (error) {
    if (errorCode(error) === "ENOENT") return { kind: "missing", observed: null }
    fail("probe_read_failed", `process stat read failed (${errorCode(error)})`)
  }
  return { kind: "present", observed: parseProcStat(contents, retained.pid) }
}

function processObservationStatus(retained, currentBootId, read) {
  if (read.kind === "missing") return "missing"
  if (retained.bootId !== currentBootId) return "changed"
  return read.observed.startTimeTicks === retained.startTimeTicks ? "present" : "changed"
}

function processIdentitySnapshot(read) {
  return read.kind === "missing"
    ? { kind: "missing" }
    : { kind: "present", pid: read.observed.pid, startTimeTicks: read.observed.startTimeTicks }
}

function statFacts(stats) {
  const dev = String(stats.dev)
  const ino = String(stats.ino)
  const mode = Number(stats.mode)
  const uid = Number(stats.uid)
  const gid = Number(stats.gid)
  if (![dev, ino].every((part) => /^(0|[1-9][0-9]*)$/.test(part)) || ![mode, uid, gid].every(Number.isSafeInteger)) {
    fail("probe_read_failed", "filesystem identity returned invalid stat fields")
  }
  const kind = stats.isSymbolicLink() ? "symlink" : stats.isDirectory() ? "directory" : stats.isFile() ? "file" : "other"
  return { kind, dev, ino, mode, uid, gid, instanceId: `stat:${dev}:${ino}` }
}

async function readDirectoryIdentity(fileSystem, retained) {
  let stats
  try {
    stats = await fileSystem.lstat(retained.path, { bigint: true })
  } catch (error) {
    if (errorCode(error) === "ENOENT") return { kind: "missing", observed: null }
    fail("probe_read_failed", `filesystem identity read failed (${errorCode(error)})`)
  }
  const facts = statFacts(stats)
  if (facts.kind === "symlink") return { kind: "changed", observed: facts }
  if (facts.kind !== "directory") return { kind: "changed", observed: facts }
  let canonicalPath
  try {
    canonicalPath = await fileSystem.realpath(retained.path)
  } catch (error) {
    fail("probe_read_failed", `canonical path read failed (${errorCode(error)})`)
  }
  const observed = { ...facts, canonicalPath }
  return { kind: "observed", observed }
}

function directoryStatus(retained, read, expectedPath) {
  if (read.kind === "missing") return "missing"
  const observed = read.observed
  if (observed.kind !== "directory" || observed.canonicalPath !== expectedPath) return "changed"
  return observed.instanceId === retained.instanceId ? "present" : "changed"
}

function snapshotStatus(snapshot) {
  return JSON.stringify(snapshot)
}

async function defaultRunCommand(executable, args) {
  const result = await execFile(executable, args, {
    encoding: "utf8",
    timeout: 5000,
    maxBuffer: 16 * 1024,
    windowsHide: true,
  })
  return { stdout: result.stdout, stderr: result.stderr, exitCode: 0 }
}

export async function capturePath1HostResidue({
  beforeCapture,
  platform = process.platform,
  fileSystem = { readFile, lstat, realpath },
  runCommand = defaultRunCommand,
  now = () => new Date().toISOString(),
} = {}) {
  if (platform !== "linux") fail("unsupported_platform", "Path-1 host residue capture requires Linux")
  const retained = normalizeBeforeIdentity(beforeCapture)
  const hostAtStart = await readHostGeneration(fileSystem)

  const serviceReads = []
  for (const service of retained.services) serviceReads.push(await readService(runCommand, service))
  const processReads = []
  for (const proc of retained.processes) processReads.push(await readProcess(fileSystem, proc))
  const directoryReads = []
  for (const state of retained.states) directoryReads.push(await readDirectoryIdentity(fileSystem, state))
  const releaseRetained = { path: retained.release.path, instanceId: retained.release.instanceId }
  const releaseRead = await readDirectoryIdentity(fileSystem, releaseRetained)

  const hostAtEnd = await readHostGeneration(fileSystem)
  if (snapshotStatus(hostAtEnd) !== snapshotStatus(hostAtStart)) fail("capture_generation_changed", "host boot or machine identity changed during capture")
  const serviceRechecks = []
  for (let index = 0; index < retained.services.length; index += 1) {
    const finalRead = await readService(runCommand, retained.services[index])
    if (snapshotStatus(finalRead) !== snapshotStatus(serviceReads[index])) fail("capture_generation_changed", `systemd identity changed during capture: ${retained.services[index].unit}`)
    serviceRechecks.push(finalRead)
  }
  const processRechecks = []
  for (let index = 0; index < retained.processes.length; index += 1) {
    const finalRead = await readProcess(fileSystem, retained.processes[index])
    if (snapshotStatus(processIdentitySnapshot(finalRead)) !== snapshotStatus(processIdentitySnapshot(processReads[index]))) fail("capture_generation_changed", `process identity changed during capture: ${retained.processes[index].pid}`)
    processRechecks.push(finalRead)
  }
  const stateRechecks = []
  for (let index = 0; index < retained.states.length; index += 1) {
    const finalRead = await readDirectoryIdentity(fileSystem, retained.states[index])
    if (snapshotStatus(finalRead) !== snapshotStatus(directoryReads[index])) fail("capture_generation_changed", `filesystem identity changed during capture: ${retained.states[index].path}`)
    stateRechecks.push(finalRead)
  }
  const releaseRecheck = await readDirectoryIdentity(fileSystem, releaseRetained)
  if (snapshotStatus(releaseRecheck) !== snapshotStatus(releaseRead)) fail("capture_generation_changed", "release root identity changed during capture")
  const finalHost = await readHostGeneration(fileSystem)
  if (snapshotStatus(finalHost) !== snapshotStatus(hostAtStart)) fail("capture_generation_changed", "host boot or machine identity changed during capture")

  return {
    schema: RESIDUE_SCHEMA,
    capturedAt: now(),
    observations: {
      host: {
        bootId: { retained: retained.bootId, observed: hostAtStart.bootId, status: hostAtStart.bootId === retained.bootId ? "present" : "changed" },
        machineId: { retained: retained.machineId, observed: hostAtStart.machineId, status: hostAtStart.machineId === retained.machineId ? "present" : "changed" },
      },
      services: retained.services.map((service, index) => ({
        retained: service,
        observed: serviceReads[index],
        status: serviceStatus(service, serviceReads[index]),
      })),
      processes: retained.processes.map((proc, index) => {
        const read = processReads[index]
        return {
          retained: { bootId: proc.bootId, pid: proc.pid, startTimeTicks: proc.startTimeTicks },
          observed: read.kind === "missing" ? null : { ...read.observed, bootId: hostAtStart.bootId },
          status: processObservationStatus(proc, hostAtStart.bootId, read),
          limitation: "A reused PID with a different start time is not evidence that the retained process remains present.",
        }
      }),
      stateDirectories: retained.states.map((state, index) => ({
        retained: state,
        observed: directoryReads[index].observed,
        status: directoryStatus(state, directoryReads[index], state.path),
      })),
      releaseRoot: {
        retained: { digest: retained.release.digest, instanceId: retained.release.instanceId },
        path: retained.release.path,
        observed: releaseRead.observed,
        status: directoryStatus(releaseRetained, releaseRead, retained.release.path),
      },
      recheck: {
        bootId: hostAtEnd.bootId,
        machineId: hostAtEnd.machineId,
        services: serviceRechecks,
        processes: processRechecks.map((read) => read.kind === "missing" ? null : {
          bootId: hostAtStart.bootId,
          pid: read.observed.pid,
          startTimeTicks: read.observed.startTimeTicks,
        }),
        stateDirectories: stateRechecks.map(({ observed }) => observed),
        releaseRoot: releaseRecheck.observed,
      },
    },
    limitations: [
      "These exact-path probes cannot prove there are no copies or identities elsewhere on the host.",
      "A PID with a different start time is a changed or reused PID; PID reuse is not proof that the retained process remains present.",
      "Permission and read errors fail capture as unknown; they are never classified as absence.",
      "This raw observation is not an MP verdict or a legacy acceptance receipt.",
    ],
  }
}

export function parsePath1HostResidueArguments(argv) {
  if (!Array.isArray(argv) || argv.length !== 4) fail("arguments_invalid", "supply --before and --output once each")
  const allowed = new Set(["--before", "--output"])
  const flags = new Map()
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index]
    const value = argv[index + 1]
    if (!allowed.has(flag) || flags.has(flag) || typeof value !== "string" || value.length === 0 || value.startsWith("--")) {
      fail("arguments_invalid", "invalid Path-1 host residue arguments")
    }
    flags.set(flag, value)
  }
  if (!flags.has("--before") || !flags.has("--output")) fail("arguments_invalid", "both --before and --output are required")
  const output = flags.get("--output")
  if (!isAbsolute(output)) fail("arguments_invalid", "--output must be an absolute external evidence path")
  return { before: flags.get("--before"), output }
}

export async function readPath1HostResidueInput(file, readCapture) {
  const requestedPath = resolve(file)
  let canonicalPath
  try {
    canonicalPath = await realpath(requestedPath)
  } catch (error) {
    fail("before_input_unavailable", `retained before capture cannot be resolved (${errorCode(error)})`)
  }
  if (canonicalPath !== requestedPath) fail("before_input_symlink", "retained before capture path must be canonical and contain no symlinks")
  if (!readCapture) {
    const correlation = await import("./path1-host-cloud-correlation.mjs")
    readCapture = correlation.readCorrelationCapture
  }
  const input = await readCapture(requestedPath)
  if (!isRecord(input?.capture) || input.capture.schema !== HOST_CAPTURE_SCHEMA) {
    fail("before_input_invalid", "--before must contain a raw Path-1 host identity capture")
  }
  if (!isRecord(input.evidence) || input.evidence.path !== canonicalPath
    || !/^sha256:[0-9a-f]{64}$/.test(input.evidence.sha256 ?? "")) {
    fail("before_input_invalid", "retained before capture evidence binding is invalid")
  }
  return input
}

export function bindPath1HostResidueEvidence(capture, inputEvidence, outputPath) {
  if (!isRecord(capture) || capture.schema !== RESIDUE_SCHEMA) fail("capture_invalid", "raw residue capture schema is invalid")
  if (!isRecord(inputEvidence) || typeof inputEvidence.path !== "string"
    || !/^sha256:[0-9a-f]{64}$/.test(inputEvidence.sha256 ?? "")) {
    fail("before_input_invalid", "retained before capture evidence binding is invalid")
  }
  if (resolve(outputPath) === inputEvidence.path) fail("input_output_overlap", "before input and output paths must be distinct")
  return {
    ...capture,
    retainedBeforeEvidence: { path: inputEvidence.path, sha256: inputEvidence.sha256 },
  }
}

export async function runPath1HostResidueCaptureCli(argv, {
  readInput = readPath1HostResidueInput,
  validateOutput = validateCaptureOutput,
  capture = capturePath1HostResidue,
  writeOutput = writeCaptureOutput,
} = {}) {
  const args = parsePath1HostResidueArguments(argv)
  const input = await readInput(args.before)
  const outputPath = await validateOutput(args.output)
  if (outputPath === input.evidence.path) fail("input_output_overlap", "before input and output paths must be distinct")
  const residue = await capture({ beforeCapture: input.capture })
  const output = bindPath1HostResidueEvidence(residue, input.evidence, outputPath)
  await writeOutput(args.output, output)
  return output
}

async function main() {
  await runPath1HostResidueCaptureCli(process.argv.slice(2))
  process.stdout.write("Captured raw Path-1 host residue observations; no acceptance verdict.\n")
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => {
    process.stderr.write(`Path-1 host residue capture failed (${error.code ?? "error"}): ${error.message}\n`)
    process.exitCode = 1
  })
}
