#!/usr/bin/env node

import { spawn } from "node:child_process"
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"
import process from "node:process"
import { fileURLToPath } from "node:url"

const scriptPath = fileURLToPath(import.meta.url)
const repositoryRoot = path.resolve(path.dirname(scriptPath), "../..")

export function parseArgs(argv) {
  const options = {
    aedsHost: "",
    aegsHost: "",
    relayHost: "",
    sshKey: "",
    aedsSshKey: "",
    aegsSshKey: "",
    relaySshKey: "",
    runId: "",
    evidenceDir: "",
  }
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index]
    const next = () => {
      index += 1
      if (index >= argv.length) throw new Error(`${value} requires a value`)
      return argv[index]
    }
    if (value === "--aeds-host") options.aedsHost = next()
    else if (value === "--aegs-host") options.aegsHost = next()
    else if (value === "--relay-host") options.relayHost = next()
    else if (value === "--ssh-key") options.sshKey = next()
    else if (value === "--aeds-ssh-key") options.aedsSshKey = next()
    else if (value === "--aegs-ssh-key") options.aegsSshKey = next()
    else if (value === "--relay-ssh-key") options.relaySshKey = next()
    else if (value === "--run-id") options.runId = next()
    else if (value === "--evidence-dir") options.evidenceDir = next()
    else if (value === "--help" || value === "-h") options.help = true
    else throw new Error(`unknown option: ${value}`)
  }
  return options
}

export function validateOptions(options) {
  for (const [name, value] of [
    ["--aeds-host", options.aedsHost],
    ["--aegs-host", options.aegsHost],
    ["--run-id", options.runId],
  ]) {
    if (!String(value ?? "").trim()) throw new Error(`${name} is required`)
  }
  if (!options.sshKey && !options.aedsSshKey) {
    throw new Error("--aeds-ssh-key or --ssh-key is required")
  }
  if (!options.sshKey && !options.aegsSshKey) {
    throw new Error("--aegs-ssh-key or --ssh-key is required")
  }
  if (options.relayHost && !options.sshKey && !options.relaySshKey) {
    throw new Error("--relay-ssh-key or --ssh-key is required when --relay-host is set")
  }
  if (!/^[a-z0-9][a-z0-9-]{2,48}$/.test(options.runId)) {
    throw new Error("--run-id must contain 3-49 lowercase letters, digits, or hyphens")
  }
  if (options.aedsHost === options.aegsHost) {
    throw new Error("AEDS and AEGS hosts must be different")
  }
  if (options.relayHost && [options.aedsHost, options.aegsHost].includes(options.relayHost)) {
    throw new Error("the relay host must not be reused for AEDS or AEGS")
  }
}

export function remoteProbeCommand(role) {
  if (!["aeds", "aegs", "relay"].includes(role)) throw new Error(`unsupported role: ${role}`)
  const requiresDocker = role !== "relay"
  return [
    "set -eu",
    `expected_role=${shellQuote(role)}`,
    "machine_id=$(cat /etc/machine-id)",
    "host_name=$(hostname)",
    "cpu_count=$(getconf _NPROCESSORS_ONLN)",
    "memory_kib=$(awk '/MemTotal:/ {print $2}' /proc/meminfo)",
    "available_kib=$(df -Pk /var/lib | awk 'NR == 2 {print $4}')",
    "root_use_percent=$(df -Pk / | awk 'NR == 2 {gsub(/%/, \"\", $5); print $5}')",
    "role_marker=unassigned",
    "marker=/etc/arroba/event-publication/host-role",
    "if test -e \"$marker\"; then role_marker=$(tr -d '\\r\\n' < \"$marker\"); fi",
    "if test \"$expected_role\" != relay && test \"$role_marker\" != unassigned && test \"$role_marker\" != \"$expected_role\"; then",
    "  printf 'host role marker is %s, refusing requested role %s\\n' \"$role_marker\" \"$expected_role\" >&2",
    "  exit 21",
    "fi",
    "if test \"$expected_role\" != relay && test \"$memory_kib\" -lt 1048576; then",
    "  printf 'host has %s KiB RAM; at least 1048576 KiB is required\\n' \"$memory_kib\" >&2",
    "  exit 22",
    "fi",
    "if test \"$expected_role\" != relay && test \"$available_kib\" -lt 5242880; then",
    "  printf 'host has %s KiB free under /var/lib; at least 5242880 KiB is required\\n' \"$available_kib\" >&2",
    "  exit 23",
    "fi",
    ...(requiresDocker
      ? [
          "command -v docker >/dev/null",
          "docker info >/dev/null",
          "docker compose version >/dev/null",
          "docker_version=$(docker version --format '{{.Server.Version}}')",
          "compose_version=$(docker compose version --short)",
        ]
      : [
          "docker_version=not-required",
          "compose_version=not-required",
        ]),
    "printf 'machine_id=%s\\n' \"$machine_id\"",
    "printf 'hostname=%s\\n' \"$host_name\"",
    "printf 'role_marker=%s\\n' \"$role_marker\"",
    "printf 'cpu_count=%s\\n' \"$cpu_count\"",
    "printf 'memory_kib=%s\\n' \"$memory_kib\"",
    "printf 'available_kib=%s\\n' \"$available_kib\"",
    "printf 'root_use_percent=%s\\n' \"$root_use_percent\"",
    "printf 'docker_version=%s\\n' \"$docker_version\"",
    "printf 'compose_version=%s\\n' \"$compose_version\"",
  ].join("\n")
}

export function parseProbeOutput(output, role, sshTarget) {
  const values = Object.fromEntries(
    String(output)
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        const separator = line.indexOf("=")
        if (separator <= 0) throw new Error(`invalid ${role} preflight output: ${line}`)
        return [line.slice(0, separator), line.slice(separator + 1)]
      }),
  )
  for (const key of [
    "machine_id",
    "hostname",
    "role_marker",
    "cpu_count",
    "memory_kib",
    "available_kib",
    "root_use_percent",
    "docker_version",
    "compose_version",
  ]) {
    if (!values[key]) throw new Error(`${role} preflight omitted ${key}`)
  }
  return {
    role,
    sshTarget,
    machineId: values.machine_id,
    hostname: values.hostname,
    roleMarker: values.role_marker,
    cpuCount: Number(values.cpu_count),
    memoryKiB: Number(values.memory_kib),
    availableKiB: Number(values.available_kib),
    rootUsePercent: Number(values.root_use_percent),
    dockerVersion: values.docker_version,
    composeVersion: values.compose_version,
  }
}

export function assertPhysicalSeparation(aeds, aegs, relay = null) {
  if (aeds.machineId === aegs.machineId) {
    throw new Error("AEDS and AEGS SSH targets resolve to the same machine")
  }
  if (relay && [aeds.machineId, aegs.machineId].includes(relay.machineId)) {
    throw new Error("an event-service SSH target resolves to the existing relay machine")
  }
}

export async function runPreflight(options, dependencies = {}) {
  validateOptions(options)
  const runSsh = dependencies.runSsh ?? sshProbe
  const revision = dependencies.revision ?? await gitOutput(["rev-parse", "HEAD"])
  const dirty = dependencies.dirty ?? Boolean((await gitOutput(["status", "--porcelain"])).trim())
  const [aeds, aegs, relay] = await Promise.all([
    runSsh(options.aedsHost, options.aedsSshKey || options.sshKey, "aeds"),
    runSsh(options.aegsHost, options.aegsSshKey || options.sshKey, "aegs"),
    options.relayHost
      ? runSsh(options.relayHost, options.relaySshKey || options.sshKey, "relay")
      : null,
  ])
  assertPhysicalSeparation(aeds, aegs, relay)
  const evidence = {
    schemaVersion: 1,
    kind: "arroba-event-publication-hetzner-preflight",
    runId: options.runId,
    capturedAt: new Date().toISOString(),
    source: { repository: "arroba", revision, dirty },
    separation: {
      aedsAndAegs: true,
      eventServicesAndRelay: relay ? true : null,
    },
    hosts: { aeds, aegs, ...(relay ? { relay } : {}) },
  }
  const evidenceDir = path.resolve(
    options.evidenceDir
      || path.join(repositoryRoot, ".artifacts/event-publication-hetzner", options.runId),
  )
  await mkdir(evidenceDir, { recursive: true, mode: 0o700 })
  const evidencePath = path.join(evidenceDir, "preflight.json")
  await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 })
  return { evidence, evidencePath }
}

async function sshProbe(host, key, role) {
  const output = await spawnCapture("ssh", [
    "-i",
    key,
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=10",
    "-o",
    "StrictHostKeyChecking=accept-new",
    host,
    remoteProbeCommand(role),
  ])
  return parseProbeOutput(output, role, host)
}

async function gitOutput(args) {
  return spawnCapture("git", ["-C", repositoryRoot, ...args])
}

function spawnCapture(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"] })
    let stdout = ""
    let stderr = ""
    child.stdout.setEncoding("utf8")
    child.stderr.setEncoding("utf8")
    child.stdout.on("data", (chunk) => { stdout += chunk })
    child.stderr.on("data", (chunk) => { stderr += chunk })
    child.on("error", reject)
    child.on("close", (code) => {
      if (code === 0) resolve(stdout.trim())
      else reject(new Error(`${command} exited ${code}: ${stderr.trim() || stdout.trim()}`))
    })
  })
}

function shellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\''")}'`
}

function usage() {
  return [
    "Usage:",
    "  node deploy/event-publication/host-separation-preflight.mjs \\",
    "    --aeds-host USER@HOST --aegs-host USER@HOST --run-id RUN_ID \\",
    "    [--ssh-key PATH | --aeds-ssh-key PATH --aegs-ssh-key PATH] \\",
    "    [--relay-host USER@HOST] [--relay-ssh-key PATH] [--evidence-dir PATH]",
    "",
    "The command is read-only. It verifies capacity, Docker, role markers, and physical",
    "host separation before any event-service deployment is allowed.",
  ].join("\n")
}

if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  try {
    const options = parseArgs(process.argv.slice(2))
    if (options.help) {
      console.log(usage())
    } else {
      const result = await runPreflight(options)
      console.log(result.evidencePath)
    }
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error))
    process.exitCode = 1
  }
}
