#!/usr/bin/env node

import { spawn } from "node:child_process"
import { readFile, mkdir, writeFile } from "node:fs/promises"
import path from "node:path"
import process from "node:process"
import { fileURLToPath } from "node:url"

const scriptPath = fileURLToPath(import.meta.url)
const repositoryRoot = path.resolve(path.dirname(scriptPath), "../..")
const components = new Set(["github", "jira", "linear", "gitlab", "sentry", "slack"])

export function parseArgs(argv) {
  const options = {
    preflight: "",
    runId: "",
    component: "",
    aedsHost: "",
    aegsHost: "",
    relayHost: "",
    sshKey: "",
    aedsSshKey: "",
    aegsSshKey: "",
    aedsUrl: "",
    aegsUrl: "",
    evidenceDir: "",
    executeRestarts: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index]
    const next = () => {
      index += 1
      if (index >= argv.length) throw new Error(`${value} requires a value`)
      return argv[index]
    }
    if (value === "--preflight") options.preflight = next()
    else if (value === "--run-id") options.runId = next()
    else if (value === "--component") options.component = next()
    else if (value === "--aeds-host") options.aedsHost = next()
    else if (value === "--aegs-host") options.aegsHost = next()
    else if (value === "--relay-host") options.relayHost = next()
    else if (value === "--ssh-key") options.sshKey = next()
    else if (value === "--aeds-ssh-key") options.aedsSshKey = next()
    else if (value === "--aegs-ssh-key") options.aegsSshKey = next()
    else if (value === "--aeds-url") options.aedsUrl = next()
    else if (value === "--aegs-url") options.aegsUrl = next()
    else if (value === "--evidence-dir") options.evidenceDir = next()
    else if (value === "--execute-restarts") options.executeRestarts = true
    else if (value === "--help" || value === "-h") options.help = true
    else throw new Error(`unknown option: ${value}`)
  }
  return options
}

export function validateOptions(options) {
  for (const [name, value] of [
    ["--preflight", options.preflight],
    ["--run-id", options.runId],
    ["--component", options.component],
    ["--aeds-host", options.aedsHost],
    ["--aegs-host", options.aegsHost],
    ["--aeds-url", options.aedsUrl],
    ["--aegs-url", options.aegsUrl],
  ]) {
    if (!String(value ?? "").trim()) throw new Error(`${name} is required`)
  }
  if (!options.sshKey && (!options.aedsSshKey || !options.aegsSshKey)) {
    throw new Error("--ssh-key or both --aeds-ssh-key and --aegs-ssh-key are required")
  }
  if (!/^[a-z0-9][a-z0-9-]{2,48}$/.test(options.runId)) {
    throw new Error("--run-id must contain 3-49 lowercase letters, digits, or hyphens")
  }
  if (!components.has(options.component)) {
    throw new Error("--component must be one of github, jira, linear, gitlab, sentry, or slack")
  }
  if (options.aedsHost === options.aegsHost) {
    throw new Error("AEDS and AEGS hosts must be different")
  }
  if (options.relayHost && [options.aedsHost, options.aegsHost].includes(options.relayHost)) {
    throw new Error("the relay host must not be reused for AEDS or AEGS")
  }
  for (const [name, value] of [["--aeds-url", options.aedsUrl], ["--aegs-url", options.aegsUrl]]) {
    const url = new URL(value)
    if (
      url.protocol !== "https:"
      || url.username
      || url.password
      || url.pathname !== "/"
      || url.search
      || url.hash
    ) {
      throw new Error(`${name} must be a credential-free public HTTPS origin`)
    }
  }
}

export function validatePreflightEvidence(evidence, options) {
  if (
    evidence?.schemaVersion !== 1
    || evidence?.kind !== "arroba-event-publication-hetzner-preflight"
  ) {
    throw new Error("preflight evidence has an unsupported contract")
  }
  if (evidence.runId !== options.runId) throw new Error("preflight run ID does not match")
  if (evidence.source?.dirty !== false) throw new Error("preflight source must be clean")
  if (evidence.separation?.aedsAndAegs !== true) {
    throw new Error("preflight does not prove AEDS/AEGS host separation")
  }
  if (evidence.hosts?.aeds?.sshTarget !== options.aedsHost) {
    throw new Error("AEDS host does not match preflight evidence")
  }
  if (evidence.hosts?.aegs?.sshTarget !== options.aegsHost) {
    throw new Error("AEGS host does not match preflight evidence")
  }
  if (options.relayHost) {
    if (
      evidence.separation?.eventServicesAndRelay !== true
      || evidence.hosts?.relay?.sshTarget !== options.relayHost
    ) {
      throw new Error("relay host does not match separated preflight evidence")
    }
  }
  const identities = [evidence.hosts.aeds.machineId, evidence.hosts.aegs.machineId]
  if (options.relayHost) identities.push(evidence.hosts.relay.machineId)
  if (identities.some((identity) => typeof identity !== "string" || !identity.trim())) {
    throw new Error("preflight evidence omits a machine identity")
  }
  if (new Set(identities).size !== identities.length) {
    throw new Error("preflight evidence resolves multiple roles to one machine")
  }
  return evidence
}

export function remoteAcceptanceCommand({ role, component, machineId, url, restart }) {
  if (!["aeds", "aegs"].includes(role)) throw new Error(`unsupported role: ${role}`)
  const unit = role === "aeds" ? "arroba-aeds.service" : `arroba-aegs-${component}.service`
  const markerRole = role
  const lines = [
    "set -eu",
    `expected_machine=${shellQuote(machineId)}`,
    `expected_role=${shellQuote(markerRole)}`,
    `unit=${shellQuote(unit)}`,
    `health_url=${shellQuote(`${url.replace(/\/+$/, "")}/readyz`)}`,
    "test \"$(cat /etc/machine-id)\" = \"$expected_machine\"",
    "marker=/etc/arroba/event-publication/host-role",
    "test -r \"$marker\"",
    "test \"$(tr -d '\\r\\n' < \"$marker\")\" = \"$expected_role\"",
  ]
  if (role === "aegs") {
    const units = [...components].map((name) => `arroba-aegs-${name}.service`)
    lines.push(
      "active_units=",
      `for candidate in ${units.map(shellQuote).join(" ")}; do`,
      "  if systemctl is-active --quiet \"$candidate\"; then active_units=\"${active_units}${candidate}\\n\"; fi",
      "done",
      "test \"$(printf '%b' \"$active_units\" | sed '/^$/d' | wc -l | tr -d ' ')\" -eq 1",
      "test \"$(printf '%b' \"$active_units\" | sed '/^$/d')\" = \"$unit\"",
    )
  }
  lines.push("systemctl is-active --quiet \"$unit\"")
  if (restart) {
    lines.push(
      "systemctl restart \"$unit\"",
      "ready=false",
      "attempt=0",
      "while test \"$attempt\" -lt 30; do",
      "  if curl --fail --silent --show-error --max-time 5 \"$health_url\" >/dev/null; then ready=true; break; fi",
      "  attempt=$((attempt + 1))",
      "  sleep 1",
      "done",
      "test \"$ready\" = true",
    )
  } else {
    lines.push("curl --fail --silent --show-error --max-time 5 \"$health_url\" >/dev/null")
  }
  lines.push(
    "printf 'machine_id=%s\\n' \"$expected_machine\"",
    "printf 'role=%s\\n' \"$expected_role\"",
    "printf 'unit=%s\\n' \"$unit\"",
    "printf 'restart=%s\\n' " + shellQuote(restart ? "passed" : "not-requested"),
    "awk '/MemAvailable:/ {printf \"memory_available_kib=%s\\n\", $2}' /proc/meminfo",
    "df -Pk /var/lib | awk 'NR == 2 {printf \"var_lib_available_kib=%s\\n\", $4}'",
    "df -Pk / | awk 'NR == 2 {gsub(/%/, \"\", $5); printf \"root_use_percent=%s\\n\", $5}'",
  )
  return lines.join("\n")
}

export async function runAcceptance(options, dependencies = {}) {
  validateOptions(options)
  const evidence = validatePreflightEvidence(
    JSON.parse(await (dependencies.readFile ?? readFile)(options.preflight, "utf8")),
    options,
  )
  const currentRevision = dependencies.revision ?? await gitOutput(["rev-parse", "HEAD"])
  const currentDirty = dependencies.dirty
    ?? Boolean((await gitOutput(["status", "--porcelain"])).trim())
  if (currentDirty) throw new Error("acceptance requires a clean OSS worktree")
  if (evidence.source.revision !== currentRevision) {
    throw new Error("preflight revision does not match the acceptance revision")
  }
  const runSsh = dependencies.runSsh ?? sshCommand
  const results = {}
  for (const role of ["aeds", "aegs"]) {
    const host = options[`${role}Host`]
    const hostEvidence = evidence.hosts[role]
    const url = options[`${role}Url`]
    const command = remoteAcceptanceCommand({
      role,
      component: options.component,
      machineId: hostEvidence.machineId,
      url,
      restart: options.executeRestarts,
    })
    results[role] = await runSsh(
      host,
      options[`${role}SshKey`] || options.sshKey,
      command,
    )
  }
  const record = {
    schemaVersion: 1,
    kind: "arroba-event-publication-hetzner-acceptance",
    runId: options.runId,
    capturedAt: new Date().toISOString(),
    component: options.component,
    preflight: path.resolve(options.preflight),
    restartMode: options.executeRestarts ? "executed" : "read-only",
    hosts: {
      aeds: { sshTarget: options.aedsHost, machineId: evidence.hosts.aeds.machineId },
      aegs: { sshTarget: options.aegsHost, machineId: evidence.hosts.aegs.machineId },
    },
    results,
  }
  const evidenceDir = path.resolve(
    options.evidenceDir
      || path.join(repositoryRoot, ".artifacts/event-publication-hetzner", options.runId),
  )
  await (dependencies.mkdir ?? mkdir)(evidenceDir, { recursive: true, mode: 0o700 })
  const evidencePath = path.join(evidenceDir, `${options.component}-acceptance.json`)
  await (dependencies.writeFile ?? writeFile)(
    evidencePath,
    `${JSON.stringify(record, null, 2)}\n`,
    { mode: 0o600 },
  )
  return { record, evidencePath }
}

async function sshCommand(host, key, command) {
  return spawnCapture("ssh", [
    "-i", key,
    "-o", "BatchMode=yes",
    "-o", "ConnectTimeout=10",
    "-o", "StrictHostKeyChecking=yes",
    host,
    command,
  ])
}

function gitOutput(args) {
  return spawnCapture("git", ["-C", repositoryRoot, ...args])
}

function spawnCapture(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"] })
    let stdout = ""
    let stderr = ""
    child.stdout.setEncoding("utf8")
    child.stderr.setEncoding("utf8")
    child.stdout.on("data", (chunk) => {
      stdout += chunk
      if (stdout.length > 16 * 1024) child.kill()
    })
    child.stderr.on("data", (chunk) => {
      stderr += chunk
      if (stderr.length > 16 * 1024) child.kill()
    })
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
    "  node deploy/event-publication/hetzner-acceptance.mjs \\",
    "    --preflight PATH --run-id RUN_ID --component COMPONENT \\",
    "    --aeds-host USER@HOST --aegs-host USER@HOST [--relay-host USER@HOST] \\",
    "    [--ssh-key PATH | --aeds-ssh-key PATH --aegs-ssh-key PATH] \\",
    "    --aeds-url https://HOST --aegs-url https://HOST \\",
    "    [--evidence-dir PATH] [--execute-restarts]",
    "",
    "The default run is read-only. --execute-restarts restarts only the exact AEDS",
    "and selected AEGS systemd units after machine IDs and role markers are verified.",
  ].join("\n")
}

if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  try {
    const options = parseArgs(process.argv.slice(2))
    if (options.help) console.log(usage())
    else console.log((await runAcceptance(options)).evidencePath)
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error))
    process.exitCode = 1
  }
}
