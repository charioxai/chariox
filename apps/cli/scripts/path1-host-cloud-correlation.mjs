// MP-07/MP-10: correlate independently captured host and Cloud identities.
// This is campaign assembly validation, not signature, residue or parity proof.
import { createHash } from "node:crypto"
import { lstat, readFile, realpath } from "node:fs/promises"
import { resolve } from "node:path"
import { pathToFileURL } from "node:url"
import { validateCaptureOutput, writeCaptureOutput } from "./path1-provider-rebuild-capture.mjs"

const DIGEST = /^sha256:[a-f0-9]{64}$/
const SOURCE = /^[a-f0-9]{40}$/
const UUID = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/
const UNITS = new Set(["chariox-path1-managed-bootstrap.service", "chariox-managed-bootstrap.service",
  "chariox-disposable-worker-bootstrap.service", "chariox-rootless-docker.service", "chariox-slice-broker.service"])

function requireValue(condition, message) {
  if (!condition) throw new Error(message)
}

function timestamp(value) {
  requireValue(typeof value === "string" && Number.isFinite(Date.parse(value))
    && new Date(value).toISOString() === value, "capture time is invalid")
  return Date.parse(value)
}

function sameRelease(host, cloud) {
  requireValue(host && cloud && DIGEST.test(host.digest) && SOURCE.test(host.sourceCommit)
    && SOURCE.test(host.sourceTree)
    && ["digest", "sourceCommit", "sourceTree"].every(field => host[field] === cloud[field]),
  "host release does not match Cloud's captured release")
  return { digest: host.digest, sourceCommit: host.sourceCommit, sourceTree: host.sourceTree }
}

function hostIdentity(capture, expected, { path1 = false } = {}) {
  requireValue(capture?.schema === "chariox.path1-host-identity-capture/v1", "unsupported host capture")
  const identity = capture.identity
  requireValue(identity && UUID.test(identity.bootId) && /^[a-f0-9]{32}$/.test(identity.machineId)
    && identity.bootId === expected?.bootId && identity.machineId === expected?.machineId,
  "host boot or machine identity does not match Cloud")
  const release = sameRelease(identity.release, expected.release)
  const services = identity.serviceInstances
  requireValue(Array.isArray(services) && services.length > 0 && services.length <= 5
    && services.every(service => UNITS.has(service.unit) && /^[a-f0-9]{32}$/.test(service.invocationId))
    && new Set(services.map(service => service.unit)).size === services.length
    && new Set(services.map(service => service.invocationId)).size === services.length,
  "host capture has ambiguous service identities")
  if (path1) {
    requireValue(services.some(service => service.unit === "chariox-path1-managed-bootstrap.service")
      && !services.some(service => ["chariox-managed-bootstrap.service", "chariox-disposable-worker-bootstrap.service"].includes(service.unit)),
    "new host does not select the Path-1 service topology")
  }
  const states = identity.stateInstances
  requireValue(Array.isArray(states) && states.length === 2
    && ["/var/lib/chariox", "/home/chariox"].every(path => states.filter(state => state.path === path).length === 1)
    && states.every(state => /^stat:[0-9]+:[1-9][0-9]*$/.test(state.instanceId)),
  "host capture lacks exact state-root identities")
  const processes = identity.processes
  requireValue(Array.isArray(processes) && processes.length > 0 && processes.length <= services.length
    && new Set(processes).size === processes.length
    && processes.every(process => typeof process === "string"
      && process.startsWith(`${identity.bootId}:`) && /^[1-9][0-9]*:[1-9][0-9]*$/.test(process.slice(identity.bootId.length + 1))),
  "host process identities are not bound to this boot")
  return {
    bootId: identity.bootId, machineId: identity.machineId, release,
    serviceInstances: services.map(({ unit, invocationId }) => ({ unit, invocationId })),
    processes: [...processes], stateInstances: states.map(({ path, instanceId }) => ({ path, instanceId })),
  }
}

export function correlatePath1HostCloud({ before, after, cloud, now = () => new Date() }) {
  requireValue(cloud?.schema === "chariox.path1-cloud-reimage-capture/v1", "unsupported Cloud capture")
  requireValue(["environmentId", "operationId", "receiptId", "providerServerId"].every(field =>
    typeof cloud[field] === "string" && /^[a-zA-Z0-9][a-zA-Z0-9._:/ -]{0,255}$/.test(cloud[field]))
    && /^[1-9][0-9]{0,17}$/.test(cloud.rebuildActionId)
    && Number.isSafeInteger(cloud.generation) && cloud.generation > 1
    && cloud.previousGeneration === cloud.generation - 1, "Cloud capture lacks the operation binding")
  const capturedAt = now().toISOString()
  const current = timestamp(capturedAt)
  const requested = timestamp(cloud.requestedAt)
  const completed = timestamp(cloud.completedAt)
  const cloudTime = timestamp(cloud.capturedAt)
  const beforeTime = timestamp(before?.capturedAt)
  const afterTime = timestamp(after?.capturedAt)
  requireValue(timestamp(cloud.before?.observedAt) <= requested
    && beforeTime <= requested && requested <= completed && completed <= afterTime
    && completed <= cloudTime && cloudTime <= current && afterTime <= current,
  "host and Cloud captures do not bracket the rebuild")
  requireValue(cloud.before?.bootId !== cloud.after?.bootId
    && cloud.before?.machineId !== cloud.after?.machineId, "rebuild did not rotate host identity")
  const oldIdentity = hostIdentity(before, cloud.before)
  const newIdentity = hostIdentity(after, { ...cloud.after, release: cloud.release }, { path1: true })
  requireValue(newIdentity.serviceInstances.every(service =>
    !oldIdentity.serviceInstances.some(old => old.invocationId === service.invocationId)),
  "new host reuses an old service invocation")
  return {
    schema: "chariox.path1-host-cloud-correlation/v1", capturedAt,
    environmentId: cloud.environmentId, operationId: cloud.operationId,
    receiptId: cloud.receiptId, generation: cloud.generation,
    providerServerId: cloud.providerServerId, rebuildActionId: cloud.rebuildActionId,
    before: oldIdentity, after: newIdentity,
  }
}

export async function readCorrelationCapture(file) {
  const canonical = await realpath(resolve(file))
  const metadata = await lstat(resolve(file))
  requireValue(metadata.isFile() && !metadata.isSymbolicLink() && metadata.size <= 1024 * 1024,
    "capture input must be a bounded regular file")
  const bytes = await readFile(canonical)
  requireValue(bytes.length <= 1024 * 1024, "capture input exceeds the size bound")
  return {
    capture: JSON.parse(bytes.toString("utf8")),
    evidence: { path: canonical, sha256: `sha256:${createHash("sha256").update(bytes).digest("hex")}` },
  }
}

async function main() {
  const args = process.argv.slice(2)
  const allowed = new Set(["--before", "--after", "--cloud", "--output"])
  const flags = new Map()
  requireValue(args.length === 8, "supply --before --after --cloud --output once each")
  for (let index = 0; index < args.length; index += 2) {
    requireValue(allowed.has(args[index]) && !flags.has(args[index]) && args[index + 1], "invalid correlation arguments")
    flags.set(args[index], args[index + 1])
  }
  const output = await validateCaptureOutput(flags.get("--output"))
  const inputs = {}
  for (const kind of ["before", "after", "cloud"]) inputs[kind] = await readCorrelationCapture(flags.get(`--${kind}`))
  const paths = Object.values(inputs).map(input => input.evidence.path)
  requireValue(new Set([...paths, output]).size === 4, "capture inputs and output must be distinct")
  const correlation = correlatePath1HostCloud(Object.fromEntries(Object.entries(inputs).map(([kind, input]) => [kind, input.capture])))
  await writeCaptureOutput(output, { ...correlation,
    evidence: Object.fromEntries(Object.entries(inputs).map(([kind, input]) => [kind, input.evidence])),
  })
  process.stdout.write("Correlated host and Cloud identities. Full MP-10 evidence remains required.\n")
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(() => { process.stderr.write("Path-1 capture correlation failed; no acceptance verdict.\n"); process.exitCode = 1 })
}
