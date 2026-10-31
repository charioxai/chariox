import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  assertPhysicalSeparation,
  parseArgs,
  parseProbeOutput,
  remoteProbeCommand,
  runPreflight,
  validateOptions,
} from "../deploy/event-publication/host-separation-preflight.mjs"

const baseOptions = {
  aedsHost: "root@aeds.example",
  aegsHost: "root@aegs.example",
  relayHost: "root@relay.example",
  sshKey: "/tmp/event-services-key",
  aedsSshKey: "",
  aegsSshKey: "",
  relaySshKey: "",
  runId: "event-preflight-001",
  evidenceDir: "",
}

function hostEvidence(role, sshTarget, machineId) {
  return {
    role,
    sshTarget,
    machineId,
    hostname: `${role}-host`,
    roleMarker: "unassigned",
    cpuCount: 2,
    memoryKiB: 4 * 1024 * 1024,
    availableKiB: 20 * 1024 * 1024,
    rootUsePercent: 40,
    dockerVersion: role === "relay" ? "not-required" : "28.0.0",
    composeVersion: role === "relay" ? "not-required" : "2.35.0",
  }
}

test("preflight requires exact distinct hosts, key, and run identity", () => {
  assert.deepEqual(
    parseArgs([
      "--aeds-host", "root@aeds.example",
      "--aegs-host", "root@aegs.example",
      "--relay-host", "root@relay.example",
      "--ssh-key", "/tmp/key",
      "--run-id", "event-run-001",
    ]),
    {
      aedsHost: "root@aeds.example",
      aegsHost: "root@aegs.example",
      relayHost: "root@relay.example",
      sshKey: "/tmp/key",
      aedsSshKey: "",
      aegsSshKey: "",
      relaySshKey: "",
      runId: "event-run-001",
      evidenceDir: "",
    },
  )
  assert.throws(
    () => validateOptions({ ...baseOptions, aegsHost: baseOptions.aedsHost }),
    /must be different/,
  )
  assert.throws(
    () => validateOptions({ ...baseOptions, aedsHost: baseOptions.relayHost }),
    /relay host must not be reused/,
  )
  assert.throws(
    () => validateOptions({ ...baseOptions, runId: "../../unsafe" }),
    /run-id/,
  )
  assert.doesNotThrow(() => validateOptions({
    ...baseOptions,
    sshKey: "",
    aedsSshKey: "/tmp/aeds-key",
    aegsSshKey: "/tmp/aegs-key",
    relaySshKey: "/tmp/relay-key",
  }))
  assert.throws(
    () => validateOptions({ ...baseOptions, sshKey: "", aedsSshKey: "/tmp/aeds-key" }),
    /--aegs-ssh-key/,
  )
})

test("remote probe is read-only and enforces capacity, Docker, and role fencing", () => {
  const command = remoteProbeCommand("aeds")
  assert.match(command, /cat \/etc\/machine-id/)
  assert.match(command, /at least 1048576 KiB/)
  assert.match(command, /at least 5242880 KiB/)
  assert.match(command, /docker compose version/)
  assert.match(command, /host role marker/)
  assert.doesNotMatch(command, /\brm\b|\btee\b|\bmkdir\b|\bdocker compose up\b/)
})

test("probe parsing is bounded to non-secret resource evidence", () => {
  const parsed = parseProbeOutput([
    "machine_id=machine-a",
    "hostname=aeds-host",
    "role_marker=aeds",
    "cpu_count=2",
    "memory_kib=4194304",
    "available_kib=20971520",
    "root_use_percent=42",
    "docker_version=28.0.0",
    "compose_version=2.35.0",
  ].join("\n"), "aeds", "root@aeds.example")
  assert.equal(parsed.machineId, "machine-a")
  assert.equal(parsed.memoryKiB, 4194304)
  assert.equal(parsed.availableKiB, 20971520)
  assert.deepEqual(Object.keys(parsed).sort(), [
    "availableKiB",
    "composeVersion",
    "cpuCount",
    "dockerVersion",
    "hostname",
    "machineId",
    "memoryKiB",
    "role",
    "roleMarker",
    "rootUsePercent",
    "sshTarget",
  ].sort())
})

test("machine identity prevents alias-based co-location and relay reuse", () => {
  const aeds = hostEvidence("aeds", "root@aeds.example", "machine-a")
  const aegs = hostEvidence("aegs", "root@aegs.example", "machine-b")
  const relay = hostEvidence("relay", "root@relay.example", "machine-c")
  assert.doesNotThrow(() => assertPhysicalSeparation(aeds, aegs, relay))
  assert.throws(
    () => assertPhysicalSeparation(aeds, { ...aegs, machineId: "machine-a" }, relay),
    /same machine/,
  )
  assert.throws(
    () => assertPhysicalSeparation(aeds, aegs, { ...relay, machineId: "machine-b" }),
    /relay machine/,
  )
})

test("successful preflight retains revision, resource, and separation evidence", async (context) => {
  const evidenceDir = await mkdtemp(path.join(os.tmpdir(), "chariox-event-preflight-"))
  context.after(() => rm(evidenceDir, { recursive: true, force: true }))
  const options = { ...baseOptions, evidenceDir }
  const evidenceByRole = {
    aeds: hostEvidence("aeds", options.aedsHost, "machine-a"),
    aegs: hostEvidence("aegs", options.aegsHost, "machine-b"),
    relay: hostEvidence("relay", options.relayHost, "machine-c"),
  }
  const { evidencePath } = await runPreflight(options, {
    revision: "a".repeat(40),
    dirty: false,
    runSsh: async (_host, _key, role) => evidenceByRole[role],
  })
  const evidence = JSON.parse(await readFile(evidencePath, "utf8"))
  assert.equal(evidence.runId, options.runId)
  assert.equal(evidence.source.revision, "a".repeat(40))
  assert.equal(evidence.source.dirty, false)
  assert.equal(evidence.separation.aedsAndAegs, true)
  assert.equal(evidence.separation.eventServicesAndRelay, true)
  assert.equal(evidence.hosts.aeds.machineId, "machine-a")
  assert.equal(evidence.hosts.aegs.machineId, "machine-b")
  assert.equal(evidence.hosts.relay.machineId, "machine-c")
})

test("preflight routes host-specific SSH keys without persisting them", async (context) => {
  const evidenceDir = await mkdtemp(path.join(os.tmpdir(), "chariox-event-preflight-keys-"))
  context.after(() => rm(evidenceDir, { recursive: true, force: true }))
  const options = {
    ...baseOptions,
    sshKey: "",
    aedsSshKey: "/tmp/aeds-key",
    aegsSshKey: "/tmp/aegs-key",
    relaySshKey: "/tmp/relay-key",
    evidenceDir,
  }
  const observed = []
  await runPreflight(options, {
    revision: "b".repeat(40),
    dirty: false,
    runSsh: async (host, key, role) => {
      observed.push({ host, key, role })
      return hostEvidence(role, host, `machine-${role}`)
    },
  })
  assert.deepEqual(observed, [
    { host: options.aedsHost, key: options.aedsSshKey, role: "aeds" },
    { host: options.aegsHost, key: options.aegsSshKey, role: "aegs" },
    { host: options.relayHost, key: options.relaySshKey, role: "relay" },
  ])
})
