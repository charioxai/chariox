#!/usr/bin/env node

import { createHash } from "node:crypto"
import { execFile as execFileCallback } from "node:child_process"
import { lstat, readFile, realpath, readlink } from "node:fs/promises"
import path from "node:path"
import { pathToFileURL } from "node:url"
import { promisify } from "node:util"

export const PATH1_HOST_CAPTURE_SCHEMA = "chariox.path1-host-identity-capture/v1"

const PATHS = Object.freeze({
  bootId: "/proc/sys/kernel/random/boot_id",
  machineId: "/etc/machine-id",
  activation: "/usr/local/bin/chariox-kernel",
  activationDirectory: "/usr/local/bin",
  charioxRoot: "/usr/lib/chariox",
  currentRelease: "/usr/lib/chariox/current",
  releases: "/usr/lib/chariox/releases",
  varState: "/var/lib/chariox",
  homeState: "/home/chariox",
})

const PATH1_UNIT = "chariox-path1-managed-bootstrap.service"
const ALLOWED_UNITS = new Set([
  PATH1_UNIT,
  "chariox-managed-bootstrap.service",
  "chariox-disposable-worker-bootstrap.service",
  "chariox-rootless-docker.service",
  "chariox-slice-broker.service",
])
const EXPECTED_ACTIVATION_TARGET = "../../../usr/lib/chariox/current/usr/local/bin/chariox-kernel"
const SYSTEMCTL = "/usr/bin/systemctl"
const MAX_COMMAND_BYTES = 16 * 1024
const MAX_MANIFEST_BYTES = 64 * 1024

const execFile = promisify(execFileCallback)
const defaultFileSystem = Object.freeze({ lstat, readFile, realpath, readlink })

export class Path1HostIdentityCaptureError extends Error {
  constructor(code, message) {
    super(message)
    this.name = "Path1HostIdentityCaptureError"
    this.code = code
  }
}

function fail(code, message) {
  throw new Path1HostIdentityCaptureError(code, message)
}

function nonEmpty(value, label) {
  if (typeof value !== "string" || value.trim() !== value || value.length === 0) {
    fail("host_identity_missing", `${label} is missing`)
  }
  return value
}

function safeTimestamp(value) {
  const date = value instanceof Date ? value : new Date(value)
  if (!Number.isFinite(date.getTime())) fail("capture_time_invalid", "capture timestamp is invalid")
  return date.toISOString()
}

function decimal(value, label, { positive = false } = {}) {
  const text = String(value)
  if (!/^\d+$/.test(text) || (positive && BigInt(text) === 0n)) {
    fail("host_identity_invalid", `${label} is invalid`)
  }
  return text
}

function modeBits(metadata) {
  return Number(BigInt(metadata.mode) & 0o7777n)
}

function filesystemIdentity(metadata, label, { rootOwned = false, immutable = false } = {}) {
  if (rootOwned && String(metadata.uid) !== "0") {
    fail("unsafe_release_owner", `${label} is not root-owned`)
  }
  const mode = modeBits(metadata)
  if (immutable && (mode & 0o022) !== 0) {
    fail("unsafe_release_mode", `${label} is group- or world-writable`)
  }
  const device = decimal(metadata.dev, `${label} device`)
  const inode = decimal(metadata.ino, `${label} inode`, { positive: true })
  return {
    path: label,
    device,
    inode,
    uid: decimal(metadata.uid, `${label} uid`),
    gid: decimal(metadata.gid, `${label} gid`),
    mode: mode.toString(8).padStart(4, "0"),
    instanceId: `stat:${device}:${inode}`,
  }
}

async function exactStat(fileSystem, filePath, label, options = {}) {
  let metadata
  try {
    metadata = await fileSystem.lstat(filePath, { bigint: true })
  } catch (error) {
    fail("host_path_unavailable", `${label} cannot be inspected: ${error.message}`)
  }
  if (metadata.isSymbolicLink()) fail("host_path_symlink", `${label} must not be a symlink`)
  return { metadata, identity: filesystemIdentity(metadata, label, options) }
}

async function exactRegularFile(fileSystem, filePath, label, maxBytes, { rootOwned = false } = {}) {
  const { metadata, identity } = await exactStat(fileSystem, filePath, label, { rootOwned, immutable: rootOwned })
  if (!metadata.isFile() || BigInt(metadata.size) > BigInt(maxBytes)) {
    fail("host_file_invalid", `${label} must be a bounded regular file`)
  }
  let bytes
  try {
    bytes = await fileSystem.readFile(filePath)
  } catch (error) {
    fail("host_path_unavailable", `${label} cannot be read: ${error.message}`)
  }
  const value = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes)
  if (value.length > maxBytes) fail("host_file_invalid", `${label} exceeds its size bound`)
  return { bytes: value, identity }
}

async function exactTextFile(fileSystem, filePath, label, maxBytes) {
  const { bytes } = await exactRegularFile(fileSystem, filePath, label, maxBytes)
  return bytes.toString("utf8").trim()
}

function validateRequestedUnits(units) {
  if (!Array.isArray(units) || units.length === 0 || units.length > ALLOWED_UNITS.size) {
    fail("unit_request_invalid", "request one or more exact managed systemd units")
  }
  const result = []
  const seen = new Set()
  for (const value of units) {
    const unit = nonEmpty(value, "systemd unit")
    if (!ALLOWED_UNITS.has(unit)) fail("unit_not_allowed", `unsupported systemd unit: ${unit}`)
    if (seen.has(unit)) fail("unit_request_invalid", `duplicate systemd unit: ${unit}`)
    seen.add(unit)
    result.push(unit)
  }
  return result
}

async function runReadOnlyCommand(command, args) {
  const result = await execFile(command, args, {
    encoding: "utf8",
    timeout: 5_000,
    maxBuffer: MAX_COMMAND_BYTES,
    windowsHide: true,
    env: { PATH: "/usr/bin:/bin", LC_ALL: "C", LANG: "C" },
  })
  return { stdout: result.stdout, stderr: result.stderr, exitCode: 0 }
}

function parseSystemdProperties(stdout, requestedUnit) {
  const values = new Map()
  for (const line of stdout.split(/\r?\n/).filter(Boolean)) {
    const separator = line.indexOf("=")
    if (separator <= 0) fail("systemd_output_invalid", `systemctl returned an invalid property for ${requestedUnit}`)
    const key = line.slice(0, separator)
    if (!new Set(["Id", "InvocationID", "MainPID", "ActiveState"]).has(key) || values.has(key)) {
      fail("systemd_output_invalid", `systemctl returned an unexpected property for ${requestedUnit}`)
    }
    values.set(key, line.slice(separator + 1))
  }
  for (const key of ["Id", "InvocationID", "MainPID", "ActiveState"]) {
    if (!values.has(key)) fail("systemd_output_invalid", `systemctl omitted ${key} for ${requestedUnit}`)
  }
  if (values.get("Id") !== requestedUnit) fail("systemd_unit_mismatch", `systemctl resolved a different unit than ${requestedUnit}`)
  if (values.get("ActiveState") !== "active") fail("systemd_unit_inactive", `${requestedUnit} is not active`)
  const invocationId = values.get("InvocationID")
  if (!/^[a-f0-9]{32}$/.test(invocationId)) fail("systemd_identity_missing", `${requestedUnit} has no valid InvocationID`)
  const mainPid = decimal(values.get("MainPID"), `${requestedUnit} MainPID`, { positive: true })
  if (!Number.isSafeInteger(Number(mainPid))) fail("systemd_identity_missing", `${requestedUnit} MainPID is outside the supported range`)
  return { unit: requestedUnit, invocationId, mainPid, activeState: values.get("ActiveState") }
}

async function observeSystemdUnit(runCommand, unit) {
  const args = [
    "show",
    "--no-pager",
    "--property=Id",
    "--property=InvocationID",
    "--property=MainPID",
    "--property=ActiveState",
    unit,
  ]
  let result
  try {
    result = await runCommand(SYSTEMCTL, args, { timeout: 5_000, maxBuffer: MAX_COMMAND_BYTES })
  } catch (error) {
    fail("systemd_query_failed", `systemd unit ${unit} could not be queried: ${error.message}`)
  }
  if (result.exitCode !== 0 || typeof result.stdout !== "string"
    || Buffer.byteLength(result.stdout) > MAX_COMMAND_BYTES
    || typeof result.stderr !== "string" || Buffer.byteLength(result.stderr) > MAX_COMMAND_BYTES) {
    fail("systemd_query_failed", `systemd unit ${unit} returned an invalid bounded command result`)
  }
  return {
    ...parseSystemdProperties(result.stdout, unit),
    command: SYSTEMCTL,
    arguments: args,
    stdout: result.stdout,
    stderr: result.stderr,
    exitCode: result.exitCode,
  }
}

function parseProcessStat(pid, value, bootId) {
  const text = Buffer.isBuffer(value) ? value.toString("utf8") : String(value)
  if (Buffer.byteLength(text) > 4096) fail("process_stat_invalid", `process ${pid} stat exceeds its bound`)
  const open = text.indexOf(" (")
  const close = text.lastIndexOf(")")
  if (open <= 0 || close <= open || text.slice(0, open) !== pid.toString()) {
    fail("process_stat_invalid", `process ${pid} stat does not match the requested PID`)
  }
  const fields = text.slice(close + 1).trim().split(/\s+/)
  if (fields.length <= 19 || !/^[A-Z]$/.test(fields[0])) {
    fail("process_stat_invalid", `process ${pid} stat is incomplete`)
  }
  const parentPid = decimal(fields[1], `process ${pid} parent PID`)
  const startTimeTicks = decimal(fields[19], `process ${pid} start time`, { positive: true })
  return {
    bootId,
    pid: pid.toString(),
    parentPid,
    state: fields[0],
    startTimeTicks,
    identity: `${bootId}:${pid}:${startTimeTicks}`,
  }
}

function sha256(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`
}

async function symlinkIdentity(fileSystem, filePath, label) {
  let metadata
  try {
    metadata = await fileSystem.lstat(filePath, { bigint: true })
  } catch (error) {
    fail("release_activation_missing", `${label} cannot be inspected: ${error.message}`)
  }
  if (!metadata.isSymbolicLink() || String(metadata.uid) !== "0") {
    fail("release_activation_unsafe", `${label} must be a root-owned symlink`)
  }
  return filesystemIdentity(metadata, filePath)
}

async function canonicalRootDirectory(fileSystem, directory) {
  const result = await exactStat(fileSystem, directory, directory, { rootOwned: true, immutable: true })
  if (!result.metadata.isDirectory()) fail("release_activation_unsafe", `${directory} is not a directory`)
  let canonical
  try {
    canonical = await fileSystem.realpath(directory)
  } catch (error) {
    fail("release_activation_unsafe", `${directory} cannot be resolved: ${error.message}`)
  }
  if (canonical !== directory) fail("release_activation_unsafe", `${directory} is not canonical`)
  return result.identity
}

async function captureActivatedRelease(fileSystem) {
  const activationIdentity = await symlinkIdentity(fileSystem, PATHS.activation, "chariox-kernel activation")
  const currentIdentity = await symlinkIdentity(fileSystem, PATHS.currentRelease, "managed current release")
  let activationTarget
  let currentTarget
  try {
    [activationTarget, currentTarget] = await Promise.all([
      fileSystem.readlink(PATHS.activation),
      fileSystem.readlink(PATHS.currentRelease),
    ])
  } catch (error) {
    fail("release_activation_unsafe", `managed release symlink cannot be read: ${error.message}`)
  }
  if (activationTarget !== EXPECTED_ACTIVATION_TARGET) {
    fail("release_activation_unsafe", "chariox-kernel activation symlink has an unexpected destination")
  }
  const match = /^releases\/([a-f0-9]{64})$/.exec(currentTarget)
  if (!match) fail("release_activation_unsafe", "current release must select one digest-named release")

  const selectedDigest = `sha256:${match[1]}`
  const releaseRoot = path.join(PATHS.releases, match[1])
  const expectedKernel = path.join(releaseRoot, "usr/local/bin/chariox-kernel")
  const manifestPath = path.join(releaseRoot, "usr/lib/chariox/release-manifest.json")
  const rootDirectories = []
  const releaseDirectories = [
    PATHS.activationDirectory,
    PATHS.charioxRoot,
    PATHS.releases,
    `${releaseRoot}/usr`,
    `${releaseRoot}/usr/local`,
    `${releaseRoot}/usr/local/bin`,
    `${releaseRoot}/usr/lib`,
    `${releaseRoot}/usr/lib/chariox`,
  ]
  for (const directory of releaseDirectories) {
    rootDirectories.push(await canonicalRootDirectory(fileSystem, directory))
  }

  let resolvedActivation
  let resolvedCurrent
  let resolvedRelease
  let resolvedKernel
  let resolvedManifest
  try {
    [resolvedActivation, resolvedCurrent, resolvedRelease, resolvedKernel, resolvedManifest] = await Promise.all([
      fileSystem.realpath(PATHS.activation),
      fileSystem.realpath(PATHS.currentRelease),
      fileSystem.realpath(releaseRoot),
      fileSystem.realpath(expectedKernel),
      fileSystem.realpath(manifestPath),
    ])
  } catch (error) {
    fail("release_activation_unsafe", `managed release destination cannot be resolved: ${error.message}`)
  }
  if (resolvedRelease !== releaseRoot || resolvedCurrent !== releaseRoot
    || resolvedActivation !== expectedKernel || resolvedKernel !== expectedKernel
    || resolvedManifest !== manifestPath) {
    fail("release_activation_unsafe", "managed kernel activation does not resolve inside the canonical digest-named release")
  }

  const releaseRootStat = await exactStat(fileSystem, releaseRoot, releaseRoot, { rootOwned: true, immutable: true })
  if (!releaseRootStat.metadata.isDirectory()) fail("release_root_invalid", "active release root is not a directory")
  const kernelStat = await exactStat(fileSystem, expectedKernel, expectedKernel, { rootOwned: true, immutable: true })
  if (!kernelStat.metadata.isFile()) fail("release_kernel_invalid", "active kernel is not a regular file")
  const manifestFile = await exactRegularFile(fileSystem, manifestPath, manifestPath, MAX_MANIFEST_BYTES, { rootOwned: true })
  const manifestText = manifestFile.bytes.toString("utf8")
  let manifest
  try {
    manifest = JSON.parse(manifestText)
  } catch {
    fail("release_manifest_invalid", "active release manifest is not valid JSON")
  }
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)
    || manifest.schemaVersion !== 2
    || !/^[a-f0-9]{40}$/.test(manifest.sourceCommit ?? "")
    || !/^[a-f0-9]{40}$/.test(manifest.sourceTree ?? "")) {
    fail("release_manifest_invalid", "active release manifest has no supported source identity")
  }
  const manifestDigest = sha256(manifestFile.bytes)
  if (manifestDigest !== selectedDigest) {
    fail("release_digest_mismatch", "active release manifest digest does not match its directory name")
  }

  return {
    identity: {
      digest: manifestDigest,
      sourceCommit: manifest.sourceCommit,
      sourceTree: manifest.sourceTree,
      instanceId: releaseRootStat.identity.instanceId,
    },
    observed: {
      activation: {
        path: PATHS.activation,
        linkTarget: activationTarget,
        resolvedPath: resolvedActivation,
        identity: activationIdentity,
        currentPath: PATHS.currentRelease,
        currentLinkTarget: currentTarget,
        currentResolvedPath: resolvedCurrent,
        currentIdentity,
        releasePath: resolvedRelease,
      },
      releaseRoot: releaseRootStat.identity,
      rootDirectories,
      kernel: kernelStat.identity,
      manifestDigest,
      manifest: {
        ...manifestFile.identity,
        size: manifestFile.bytes.length,
        sha256: manifestDigest,
        sourceCommit: manifest.sourceCommit,
        sourceTree: manifest.sourceTree,
        text: manifestText,
      },
    },
  }
}

async function readHostIdentity(fileSystem) {
  const bootId = await exactTextFile(fileSystem, PATHS.bootId, PATHS.bootId, 128)
  if (!/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(bootId)) {
    fail("host_identity_invalid", "Linux boot ID is malformed")
  }
  const machineId = await exactTextFile(fileSystem, PATHS.machineId, PATHS.machineId, 128)
  if (!/^[a-f0-9]{32}$/.test(machineId)) fail("host_identity_invalid", "Linux machine ID is malformed")
  return { bootId, machineId }
}

async function readProcessIdentity(fileSystem, pid, bootId, unit) {
  let processStat
  try {
    processStat = await fileSystem.readFile(`/proc/${pid}/stat`)
  } catch (error) {
    fail("process_identity_missing", `MainPID ${pid} for ${unit} has no readable process stat: ${error.message}`)
  }
  return parseProcessStat(pid, processStat, bootId)
}

async function captureStateDirectories(fileSystem) {
  const stateStats = []
  for (const statePath of [PATHS.varState, PATHS.homeState]) {
    const result = await exactStat(fileSystem, statePath, statePath)
    if (!result.metadata.isDirectory()) fail("state_identity_invalid", `${statePath} is not a directory`)
    let canonical
    try {
      canonical = await fileSystem.realpath(statePath)
    } catch (error) {
      fail("state_identity_invalid", `${statePath} cannot be resolved: ${error.message}`)
    }
    if (canonical !== statePath) fail("state_identity_invalid", `${statePath} is not its canonical directory`)
    stateStats.push(result.identity)
  }
  return stateStats
}

function equalServiceIdentity(first, second) {
  return first.unit === second.unit
    && first.invocationId === second.invocationId
    && first.mainPid === second.mainPid
    && first.activeState === second.activeState
}

function releaseIdentitySnapshot(release) {
  return JSON.stringify({
    identity: release.identity,
    activation: release.observed.activation,
    releaseRoot: release.observed.releaseRoot,
    kernel: release.observed.kernel,
    manifest: {
      path: release.observed.manifest.path,
      instanceId: release.observed.manifest.instanceId,
      sha256: release.observed.manifest.sha256,
      size: release.observed.manifest.size,
    },
    rootDirectories: release.observed.rootDirectories,
  })
}

export async function capturePath1HostIdentity({
  units = [PATH1_UNIT],
  platform = process.platform,
  fileSystem = defaultFileSystem,
  runCommand = runReadOnlyCommand,
  now = () => new Date(),
} = {}) {
  if (platform !== "linux") fail("platform_unsupported", "Path-1 host identity capture requires Linux")
  const requestedUnits = validateRequestedUnits(units)
  const startedAt = safeTimestamp(now())
  const hostIdentity = await readHostIdentity(fileSystem)
  const serviceObservations = []
  for (const unit of requestedUnits) {
    serviceObservations.push(await observeSystemdUnit(runCommand, unit))
  }
  const invocationOwners = new Map()
  for (const service of serviceObservations) {
    const owner = invocationOwners.get(service.invocationId)
    if (owner && owner !== service.unit) {
      fail("systemd_identity_duplicate", `${service.unit} shares an InvocationID with ${owner}`)
    }
    invocationOwners.set(service.invocationId, service.unit)
  }

  const processObservations = []
  for (const service of serviceObservations) {
    processObservations.push({
      unit: service.unit,
      ...await readProcessIdentity(fileSystem, Number(service.mainPid), hostIdentity.bootId, service.unit),
    })
  }
  const stateStats = await captureStateDirectories(fileSystem)
  const release = await captureActivatedRelease(fileSystem)

  const recheckedServices = []
  const recheckedProcesses = []
  for (let index = 0; index < serviceObservations.length; index += 1) {
    const firstService = serviceObservations[index]
    const lastService = await observeSystemdUnit(runCommand, firstService.unit)
    if (!equalServiceIdentity(firstService, lastService)) {
      fail("systemd_identity_changed", `${firstService.unit} identity changed during capture`)
    }
    recheckedServices.push(lastService)
    const firstProcess = processObservations[index]
    const lastProcess = await readProcessIdentity(fileSystem, Number(lastService.mainPid), hostIdentity.bootId, lastService.unit)
    if (lastProcess.startTimeTicks !== firstProcess.startTimeTicks) {
      fail("process_identity_changed", `MainPID ${lastService.mainPid} for ${lastService.unit} changed during capture`)
    }
    recheckedProcesses.push({ unit: lastService.unit, ...lastProcess })
  }

  const recheckedStateStats = await captureStateDirectories(fileSystem)
  if (JSON.stringify(stateStats) !== JSON.stringify(recheckedStateStats)) {
    fail("state_identity_changed", "managed state root identity changed during capture")
  }
  const recheckedRelease = await captureActivatedRelease(fileSystem)
  if (releaseIdentitySnapshot(release) !== releaseIdentitySnapshot(recheckedRelease)) {
    fail("release_identity_changed", "active release identity changed during capture")
  }
  const recheckedHostIdentity = await readHostIdentity(fileSystem)
  if (recheckedHostIdentity.bootId !== hostIdentity.bootId || recheckedHostIdentity.machineId !== hostIdentity.machineId) {
    fail("host_identity_changed", "boot ID or machine ID changed during capture")
  }

  const processes = [...new Map(processObservations.map((process) => [process.identity, process])).values()]
    .sort((left, right) => Number(left.pid) - Number(right.pid))
  return {
    schema: PATH1_HOST_CAPTURE_SCHEMA,
    capturedAt: safeTimestamp(now()),
    identity: {
      bootId: hostIdentity.bootId,
      machineId: hostIdentity.machineId,
      release: release.identity,
      serviceInstances: serviceObservations.map(({ unit, invocationId }) => ({ unit, invocationId })),
      processes: processes.map(({ identity }) => identity),
      stateInstances: stateStats.map(({ path: statePath, instanceId }) => ({ path: statePath, instanceId })),
    },
    observations: {
      startedAt,
      systemdUnits: serviceObservations,
      processIdentities: processes,
      stateStats,
      release: release.observed,
      identityRecheck: {
        bootId: recheckedHostIdentity.bootId,
        machineId: recheckedHostIdentity.machineId,
        systemdUnits: recheckedServices,
        processIdentities: recheckedProcesses,
        stateStats: recheckedStateStats,
        release: recheckedRelease.observed,
      },
    },
  }
}

function parseArguments(argv) {
  const units = []
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] !== "--unit" || !argv[index + 1] || argv[index + 1].startsWith("--")) {
      fail("usage", "usage: path1-host-identity-capture.mjs --unit <allowlisted-systemd-unit> [--unit <allowlisted-systemd-unit> ...]")
    }
    units.push(argv[index + 1])
    index += 1
  }
  return validateRequestedUnits(units)
}

async function main() {
  try {
    const capture = await capturePath1HostIdentity({ units: parseArguments(process.argv.slice(2)) })
    process.stdout.write(`${JSON.stringify(capture, null, 2)}\n`)
  } catch (error) {
    const code = error instanceof Path1HostIdentityCaptureError ? error.code : "capture_failed"
    const message = error instanceof Error ? error.message : String(error)
    process.stderr.write(`FAIL ${code}: ${message}\n`)
    process.exitCode = 1
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main()
}
