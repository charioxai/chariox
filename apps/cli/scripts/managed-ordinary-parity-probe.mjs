#!/usr/bin/env node

import { execFile } from "node:child_process"
import { createHash } from "node:crypto"
import dns from "node:dns/promises"
import {
  access,
  constants,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
  stat,
  writeFile,
} from "node:fs/promises"
import net from "node:net"
import { basename, dirname, isAbsolute, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { ROW_DEFINITIONS, SHUTDOWN_EXPECTATIONS } from "./managed-ordinary-parity-matrix.mjs"

const execFileAsync = promisify(execFile)
const PROBE_RELATIVE_PATH = "apps/cli/scripts/managed-ordinary-parity-probe.mjs"
const REVIEWED_COMMIT = /^[0-9a-f]{40}$/i
const DIGEST = /^sha256:[0-9a-f]{64}$/i
const TOPOLOGIES = new Set(["ordinary", "path1"])
const OFFICIAL_PROVIDERS = new Set(["claude", "codex", "opencode"])
const CHECKS = new Map(ROW_DEFINITIONS.map(({ id, checks }) => [id, new Set(checks)]))

class ProbeError extends Error {
  constructor(message, details = {}) {
    super(message)
    this.name = "ManagedOrdinaryParityProbeError"
    Object.assign(this, details)
  }
}

function stable(value) {
  if (Array.isArray(value)) return value.map(stable)
  if (!value || typeof value !== "object") return value
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, stable(value[key])]))
}

function canonicalJson(value) {
  return JSON.stringify(stable(value))
}

function fingerprint(value) {
  if (Buffer.isBuffer(value) || value instanceof Uint8Array) {
    return `sha256:${createHash("sha256").update(value).digest("hex")}`
  }
  return `sha256:${createHash("sha256").update(typeof value === "string" ? value : canonicalJson(value)).digest("hex")}`
}

function parseBoolean(value) {
  return value === true || value === "1" || value === "true"
}

function safeIdentifier(value, label) {
  if (typeof value !== "string" || !/^[A-Za-z0-9][A-Za-z0-9_.:/-]{0,127}$/.test(value)) {
    throw new ProbeError(`${label} must be a safe identifier`)
  }
  return value
}

function parseArgs(argv) {
  const values = {}
  const allowed = new Set([
    "source-root", "reviewed-commit", "parity-row", "parity-check", "topology",
    "home-path", "tmp-path", "nested-path", "new-directory", "json",
    "provider", "provider-command", "expected-cwd", "workspace-id",
  ])
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (!argument.startsWith("--") || !allowed.has(argument.slice(2))) {
      throw new ProbeError(`unsupported argument: ${argument}`)
    }
    const key = argument.slice(2).replaceAll("-", "_")
    if (key === "json") {
      values.json = true
      continue
    }
    const value = argv[index + 1]
    if (!value || value.startsWith("--")) throw new ProbeError(`missing value for ${argument}`)
    values[key] = value
    index += 1
  }
  for (const key of ["source_root", "reviewed_commit", "parity_row", "parity_check", "topology", "home_path", "tmp_path", "nested_path", "new_directory"]) {
    if (typeof values[key] !== "string" || values[key].trim() === "") throw new ProbeError(`missing required argument: --${key.replaceAll("_", "-")}`)
  }
  if (values.expected_cwd !== undefined && !isAbsolute(values.expected_cwd)) {
    throw new ProbeError("expected cwd must be absolute")
  }
  if (!REVIEWED_COMMIT.test(values.reviewed_commit)) throw new ProbeError("reviewed commit must be a full Git commit ID")
  if (!TOPOLOGIES.has(values.topology)) throw new ProbeError("topology must be ordinary or path1")
  if (!CHECKS.has(values.parity_row) || !CHECKS.get(values.parity_row).has(values.parity_check)) {
    throw new ProbeError("parity row/check is not in the reviewed inventory")
  }
  if (!isAbsolute(values.source_root)) throw new ProbeError("source root must be absolute")
  return values
}

async function runCommand(command, args, options = {}) {
  try {
    const env = command === "git"
      ? {
          ...process.env,
          ...(options.env ?? {}),
          GIT_CONFIG_NOSYSTEM: "1",
          GIT_CONFIG_GLOBAL: "/dev/null",
          GIT_NO_REPLACE_OBJECTS: "1",
          LC_ALL: "C",
        }
      : options.env
    const result = await execFileAsync(command, args, {
      cwd: options.cwd,
      env,
      encoding: "utf8",
      maxBuffer: 4 * 1024 * 1024,
      timeout: options.timeout ?? 15_000,
    })
    return { code: 0, signal: null, stdout: String(result.stdout ?? ""), stderr: String(result.stderr ?? "") }
  } catch (error) {
    return {
      code: Number.isInteger(error?.code) ? error.code : null,
      signal: error?.signal ?? null,
      timedOut: error?.code === "ETIMEDOUT" || error?.timedOut === true,
      stdout: String(error?.stdout ?? ""),
      stderr: String(error?.stderr ?? error?.message ?? ""),
    }
  }
}

async function runGit(sourceRoot, args) {
  const result = await runCommand("git", args, { cwd: sourceRoot })
  if (result.code !== 0 || result.signal) throw new ProbeError(`git ${args.join(" ")} failed`, { cause: result })
  return result.stdout.trim()
}

function parseTreeBlob(text) {
  const records = String(text).trim().split("\n").filter(Boolean)
  if (records.length !== 1) throw new ProbeError("repo-owned probe is not uniquely tracked")
  const match = /^(100644|100755) blob ([0-9a-f]{40})\t(.+)$/.exec(records[0])
  if (!match || match[3] !== PROBE_RELATIVE_PATH) throw new ProbeError("repo-owned probe path is not tracked at the reviewed commit")
  return match[2]
}

async function verifyProbeIdentity(values) {
  const sourceRoot = resolve(values.source_root)
  const ownPath = resolve(fileURLToPath(import.meta.url))
  const expectedPath = resolve(sourceRoot, PROBE_RELATIVE_PATH)
  if (ownPath !== expectedPath) throw new ProbeError("probe was not executed from the repo-owned path")
  const actualCommit = await runGit(sourceRoot, ["rev-parse", "HEAD"])
  if (actualCommit !== values.reviewed_commit) throw new ProbeError("probe source commit does not match reviewed commit")
  const status = await runGit(sourceRoot, ["status", "--porcelain=1", "--untracked-files=all"])
  if (status) throw new ProbeError("probe source worktree is dirty")
  const reviewedBlob = parseTreeBlob(await runGit(sourceRoot, ["ls-tree", "-r", "--full-tree", values.reviewed_commit, "--", PROBE_RELATIVE_PATH]))
  const actualBlob = await runGit(sourceRoot, ["hash-object", "--", PROBE_RELATIVE_PATH])
  if (actualBlob !== reviewedBlob) throw new ProbeError("repo-owned probe does not match the reviewed commit")
  const sourceTree = await runGit(sourceRoot, ["rev-parse", `${values.reviewed_commit}^{tree}`])
  const fileBytes = await readFile(ownPath)
  return {
    probe_identity_verified: true,
    probe_source_commit: actualCommit,
    probe_source_tree: sourceTree,
    probe_file: PROBE_RELATIVE_PATH,
    probe_file_git_blob: actualBlob,
    probe_file_sha256: fingerprint(fileBytes),
  }
}

function parseJsonEnv(name) {
  const raw = process.env[name]
  if (!raw) throw new ProbeError(`missing observation context: ${name}`, { code: "observation_context_missing" })
  try {
    const value = JSON.parse(raw)
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("not an object")
    return value
  } catch (error) {
    throw new ProbeError(`invalid observation context: ${name}`, { code: "observation_context_invalid", cause: error })
  }
}

function requireObservedEvidence(value, name) {
  if (!value || value.observed !== true) throw new ProbeError(`${name} is not an observed product result`, { code: "observation_not_verified" })
  return value
}

async function accessible(path, mode = constants.R_OK | constants.X_OK) {
  await access(path, mode)
  return true
}

async function pathFingerprint(path, label) {
  try {
    const resolvedPath = await realpath(path)
    return { path_fingerprint: fingerprint(resolvedPath), resolved_path: resolvedPath }
  } catch (error) {
    throw new ProbeError(`${label} is not readable`, { cause: error })
  }
}

function safeProbePath(path, label) {
  const resolvedPath = resolve(path)
  if (!isAbsolute(path) || !resolvedPath.startsWith("/tmp/chariox-parity-")) {
    throw new ProbeError(`${label} must be a dedicated /tmp/chariox-parity path`)
  }
  return resolvedPath
}

function decodeMountField(value) {
  return value.replaceAll("\\040", " ").replaceAll("\\011", "\t").replaceAll("\\012", "\n").replaceAll("\\134", "\\")
}

export function normalizeMountInfo(text) {
  return String(text).split("\n").filter(Boolean).map((line) => {
    const [leftText, rightText] = line.split(" - ")
    const left = leftText.trim().split(/\s+/)
    const right = (rightText ?? "").trim().split(/\s+/)
    if (left.length < 6 || right.length < 3) throw new ProbeError("mountinfo record is malformed")
    return {
      mount_point: decodeMountField(left[4]),
      options: left[5],
      optional: left.slice(6).sort(),
      filesystem: right[0],
      source: decodeMountField(right[1]),
      super_options: right[2],
    }
  }).sort((left, right) => canonicalJson(left).localeCompare(canonicalJson(right)))
}

async function processChain() {
  const chain = []
  let pid = process.pid
  const seen = new Set()
  while (Number.isInteger(pid) && pid > 0 && !seen.has(pid) && chain.length < 64) {
    seen.add(pid)
    let command = ""
    let parentPid = null
    try {
      command = (await readFile(`/proc/${pid}/cmdline`, "utf8")).replaceAll("\0", " ").trim()
      const statLine = await readFile(`/proc/${pid}/stat`, "utf8")
      const close = statLine.lastIndexOf(")")
      const after = close >= 0 ? statLine.slice(close + 2).trim().split(/\s+/) : []
      parentPid = Number(after[1])
    } catch {
      break
    }
    chain.push({ pid, command })
    if (!Number.isSafeInteger(parentPid) || parentPid <= 0 || parentPid === pid) break
    pid = parentPid
  }
  return chain
}

async function environmentMarkers(chain) {
  const found = new Set()
  const forbidden = ["CHARIOX_MANAGED_PROVIDER_ISOLATION", "CHARIOX_MANAGED_PROVIDER_BWRAP", "CHARIOX_MANAGED_PROVIDER_BWRAP_ARGS"]
  for (const entry of chain) {
    try {
      const bytes = await readFile(`/proc/${entry.pid}/environ`)
      for (const item of bytes.toString("utf8").split("\0")) {
        const key = item.split("=", 1)[0]
        if (forbidden.includes(key)) found.add(key)
      }
    } catch {}
  }
  for (const key of forbidden) if (Object.hasOwn(process.env, key)) found.add(key)
  return found
}

function identityResult(identity, result) {
  return { ...identity, observed: true, ...result }
}

async function observeFreshWorker(identity) {
  const evidence = requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_WORKER_EVIDENCE_JSON"), "worker evidence")
  if (evidence.fresh_worker !== true || typeof evidence.worker_id !== "string" || evidence.worker_id.length === 0) {
    throw new ProbeError("worker evidence does not prove a fresh worker")
  }
  return identityResult(identity, {
    fresh_worker: true,
    worker_id: fingerprint(evidence.worker_id),
    worker_start_fingerprint: fingerprint({ pid: process.pid, start_time: evidence.start_time ?? null }),
  })
}

async function observeProviderIdentity(identity, values) {
  const evidence = requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_PROVIDER_IDENTITY_JSON"), "provider identity")
  const provider = evidence.name ?? values.provider ?? process.env.CHARIOX_PARITY_PROVIDER
  const executable = evidence.executable ?? values.provider_command ?? process.env.CHARIOX_PARITY_PROVIDER_COMMAND
  const version = evidence.version
  if (!OFFICIAL_PROVIDERS.has(provider) || typeof version !== "string" || version.length === 0 || evidence.official !== true) {
    throw new ProbeError("provider identity is not an official observed identity")
  }
  if (executable && basename(executable) !== provider) throw new ProbeError("provider executable does not match observed provider")
  const chain = await processChain()
  const providerObserved = chain.some(({ command }) => command.toLowerCase().includes(provider.toLowerCase()))
    || evidence.process_observed === true
  if (!providerObserved) throw new ProbeError("provider process was not observed")
  return identityResult(identity, {
    official: true,
    provider_name: provider,
    executable_matches: true,
    executable_basename: provider,
    provider_version_fingerprint: fingerprint(version),
  })
}

async function observeCaptureBoundary(identity) {
  const evidence = requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON"), "capture boundary")
  const boundary = evidence.boundary ?? process.env.CHARIOX_PARITY_BOUNDARY
  if (!new Set(["official-provider-turn", "remote-command"]).has(boundary)
    || evidence.inside_provider_turn !== true
    || evidence.independent !== true) {
    throw new ProbeError("capture boundary is not an independent approved product boundary")
  }
  return identityResult(identity, { boundary, inside_provider_turn: true, independent: true })
}

async function observeDirectoryCheck(identity, values, checkId) {
  if (checkId === "home_access") return identityResult(identity, { accessible: await accessible(values.home_path) })
  if (checkId === "tmp_access") return identityResult(identity, { accessible: await accessible(values.tmp_path) })
  if (checkId === "directory_discovery") {
    const exactPathAccessible = await accessible(values.home_path)
    let childEnumerationDenied = false
    try {
      await readdir(values.home_path)
    } catch (error) {
      if (error?.code !== "EACCES" && error?.code !== "EPERM") throw error
      childEnumerationDenied = true
    }
    return identityResult(identity, { exact_path_accessible: exactPathAccessible, child_enumeration_denied: childEnumerationDenied })
  }
  if (checkId === "exact_path_entry") {
    const expectedCwd = values.expected_cwd ?? values.source_root
    const current = await pathFingerprint(process.cwd(), "current directory")
    const requested = await pathFingerprint(expectedCwd, "requested working directory")
    return identityResult(identity, {
      exact_path_accessible: await accessible(expectedCwd),
      cwd_matches_requested: current.resolved_path === requested.resolved_path,
      cwd_fingerprint: current.path_fingerprint,
      requested_cwd_fingerprint: requested.path_fingerprint,
    })
  }
  if (checkId === "directory_creation") {
    const target = safeProbePath(values.new_directory, "new directory")
    await mkdir(target, { recursive: true })
    const metadata = await stat(target)
    const result = { created_and_accessible: metadata.isDirectory() && await accessible(target), created_path_fingerprint: fingerprint(resolve(target)) }
    await rm(target, { recursive: true, force: false })
    return identityResult(identity, result)
  }
  throw new ProbeError(`unsupported MP-02 check: ${checkId}`)
}

async function observeWorkspaceCheck(identity, values, checkId) {
  const sourceRoot = resolve(values.source_root)
  if (checkId === "empty_workspace") {
    const target = safeProbePath(values.nested_path, "nested workspace")
    await mkdir(target, { recursive: true })
    const result = { workspace_created: (await stat(target)).isDirectory(), control_state_separate: resolve(target) !== sourceRoot, workspace_path_fingerprint: fingerprint(resolve(target)) }
    await rm(target, { recursive: true, force: false })
    return identityResult(identity, result)
  }
  if (checkId === "copied_repository") {
    return identityResult(identity, { repository_accessible: await accessible(sourceRoot), repository_path_fingerprint: fingerprint(sourceRoot) })
  }
  if (checkId === "repository_basename") {
    const top = resolve(await runGit(sourceRoot, ["rev-parse", "--show-toplevel"]))
    return identityResult(identity, { source_basename_preserved: top === sourceRoot && basename(top) === basename(sourceRoot), repository_basename_fingerprint: fingerprint(basename(top)) })
  }
  if (checkId === "basename_collision") {
    const target = safeProbePath(values.new_directory, "collision directory")
    await mkdir(target, { recursive: true })
    let collisionRejected = false
    try {
      await mkdir(target)
    } catch (error) {
      collisionRejected = error?.code === "EEXIST"
    }
    await rm(target, { recursive: true, force: false })
    return identityResult(identity, { collision_rejected: collisionRejected, collision_path_fingerprint: fingerprint(resolve(target)) })
  }
  if (checkId === "worktree_placement") {
    const nested = resolve(values.nested_path)
    return identityResult(identity, {
      worktree_user_path: await accessible(sourceRoot),
      control_root_not_workspace: !nested.startsWith(`${sourceRoot}/`),
      workspace_path_fingerprint: fingerprint(sourceRoot),
      control_candidate_path_fingerprint: fingerprint(nested),
    })
  }
  throw new ProbeError(`unsupported MP-03 check: ${checkId}`)
}

async function observeProviderAncestry(identity, values) {
  const chain = await processChain()
  const provider = values.provider ?? process.env.CHARIOX_PARITY_PROVIDER
  const commands = chain.map(({ command }) => command)
  const observedProvider = OFFICIAL_PROVIDERS.has(provider ?? "")
    && (commands.some((command) => command.toLowerCase().includes(provider.toLowerCase()))
      || parseBoolean(process.env.CHARIOX_PARITY_PROVIDER_PROCESS_OBSERVED))
  const observedBwrapAncestor = commands.some((command) => /(^|\s|\/)bwrap(?:\s|$)/i.test(command))
  const worker = requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_WORKER_EVIDENCE_JSON"), "worker evidence")
  const boundaryEvidence = process.env.CHARIOX_PARITY_ANCESTRY_EVIDENCE_JSON
    ? requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_ANCESTRY_EVIDENCE_JSON"), "provider ancestry")
    : null
  const comparison = {
    provider_observed: boundaryEvidence ? boundaryEvidence.provider_observed === true : observedProvider,
    bwrap_ancestor: boundaryEvidence ? boundaryEvidence.bwrap_ancestor === true : observedBwrapAncestor,
    fresh_worker: boundaryEvidence ? boundaryEvidence.fresh_worker === true : worker.fresh_worker === true,
    ancestry_complete: boundaryEvidence ? boundaryEvidence.ancestry_complete === true : chain.length > 0 && chain.at(-1)?.pid === 1,
  }
  if (!comparison.provider_observed || comparison.bwrap_ancestor || !comparison.fresh_worker || !comparison.ancestry_complete) {
    throw new ProbeError(`provider ancestry did not prove the ordinary worker boundary: ${JSON.stringify(comparison)}`)
  }
  return identityResult(identity, {
    ...comparison,
    observed_bwrap_ancestor: observedBwrapAncestor,
    command_chain_fingerprint: fingerprint(commands.join("\n")),
    boundary_evidence_fingerprint: boundaryEvidence ? fingerprint(boundaryEvidence.evidence_id ?? "provider-ancestry") : null,
  })
}

async function observeProviderEnvironment(identity, values) {
  const baseline = parseJsonEnv("CHARIOX_PARITY_ORDINARY_ENVIRONMENT_JSON")
  const home = await pathFingerprint(process.env.HOME, "HOME")
  const charioxHome = await pathFingerprint(process.env.CHARIOX_HOME ?? join(home.resolved_path, ".chariox"), "Chariox home")
  const cwd = await pathFingerprint(process.cwd(), "current directory")
  const uid = typeof process.getuid === "function" ? process.getuid() : null
  const gid = typeof process.getgid === "function" ? process.getgid() : null
  const matches = home.path_fingerprint === baseline.home_fingerprint
    && charioxHome.path_fingerprint === baseline.chariox_home_fingerprint
    && cwd.path_fingerprint === baseline.cwd_fingerprint
    && uid === baseline.uid
    && gid === baseline.gid
  const ordinaryUser = uid === baseline.uid && gid === baseline.gid
  if (!matches || !ordinaryUser) throw new ProbeError("provider environment differs from the observed ordinary baseline")
  return identityResult(identity, {
    home_matches_ordinary: true,
    chariox_home_matches_ordinary: true,
    cwd_matches_requested: cwd.resolved_path === resolve(values.source_root),
    ordinary_user: ordinaryUser,
    home_fingerprint: home.path_fingerprint,
    chariox_home_fingerprint: charioxHome.path_fingerprint,
    cwd_fingerprint: cwd.path_fingerprint,
    uid,
    gid,
  })
}

async function observeIsolationEnvironment(identity) {
  const markers = await environmentMarkers(await processChain())
  const managedMarkerAbsent = !markers.has("CHARIOX_MANAGED_PROVIDER_ISOLATION")
  const bwrapEnvironmentAbsent = !markers.has("CHARIOX_MANAGED_PROVIDER_BWRAP") && !markers.has("CHARIOX_MANAGED_PROVIDER_BWRAP_ARGS")
  if (!managedMarkerAbsent || !bwrapEnvironmentAbsent) throw new ProbeError("managed isolation marker is present")
  return identityResult(identity, { managed_marker_absent: true, bwrap_environment_absent: true, marker_fingerprint: fingerprint([...markers].sort()) })
}

async function observeMountVisibility(identity, values) {
  const mountInfo = await readFile("/proc/self/mountinfo", "utf8")
  const mounts = normalizeMountInfo(mountInfo)
  const mountFingerprint = fingerprint(mounts)
  const baseline = process.env.CHARIOX_PARITY_ORDINARY_MOUNT_FINGERPRINT
  if (!DIGEST.test(baseline ?? "")) throw new ProbeError("ordinary mount fingerprint is required")
  const visible = await Promise.all([
    accessible(values.home_path),
    accessible(values.tmp_path),
    accessible(values.source_root),
  ])
  const matches = mountFingerprint === baseline
  if (!visible.every(Boolean) || !matches) throw new ProbeError("mount visibility differs from the observed ordinary baseline")
  return identityResult(identity, {
    mounts_match_ordinary: true,
    mount_probe_complete: true,
    mount_fingerprint: mountFingerprint,
    mount_count: mounts.length,
    visible_paths: visible,
  })
}

async function observePrivilegeState(identity) {
  const status = await readFile("/proc/self/status", "utf8")
  const noNewPrivs = /^NoNewPrivs:\s*(\d+)$/m.exec(status)?.[1]
  const capEff = /^CapEff:\s*([0-9a-f]+)$/im.exec(status)?.[1]?.toLowerCase()
  if (!noNewPrivs || !capEff) throw new ProbeError("privilege fields are missing")
  const umask = process.umask().toString(8).padStart(4, "0")
  const uid = typeof process.getuid === "function" ? process.getuid() : null
  const gid = typeof process.getgid === "function" ? process.getgid() : null
  const baseline = parseJsonEnv("CHARIOX_PARITY_ORDINARY_PRIVILEGE_JSON")
  const boundaryEvidence = process.env.CHARIOX_PARITY_PRIVILEGE_EVIDENCE_JSON
    ? requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_PRIVILEGE_EVIDENCE_JSON"), "privilege state")
    : null
  if (boundaryEvidence && typeof boundaryEvidence.no_new_privs !== "boolean") {
    throw new ProbeError("privilege state evidence is missing no_new_privs")
  }
  const comparableNoNewPrivs = boundaryEvidence ? boundaryEvidence.no_new_privs === true : noNewPrivs === "1"
  const capabilitiesMatch = capEff === String(baseline.cap_eff ?? "").toLowerCase()
  const umaskMatches = umask === baseline.umask && uid === baseline.uid && gid === baseline.gid
  if (!capabilitiesMatch || !umaskMatches || comparableNoNewPrivs) throw new ProbeError("privilege state differs from the observed ordinary baseline")
  return identityResult(identity, {
    no_new_privs: false,
    capabilities_match_ordinary: true,
    umask_matches_ordinary: true,
    cap_eff: capEff,
    umask,
    uid,
    gid,
    observed_no_new_privs: noNewPrivs === "1",
    privilege_evidence_fingerprint: boundaryEvidence ? fingerprint(boundaryEvidence.evidence_id ?? "privilege-state") : null,
  })
}

async function connectTcp(host, port) {
  return await new Promise((resolvePromise) => {
    const socket = net.createConnection({ host, port })
    const timer = setTimeout(() => {
      socket.destroy()
      resolvePromise(false)
    }, 5_000)
    timer.unref?.()
    socket.once("connect", () => {
      clearTimeout(timer)
      socket.end()
      resolvePromise(true)
    })
    socket.once("error", () => {
      clearTimeout(timer)
      socket.destroy()
      resolvePromise(false)
    })
  })
}

async function observeNetwork(identity) {
  const probe = parseJsonEnv("CHARIOX_PARITY_NETWORK_PROBE_JSON")
  const host = typeof probe.host === "string" ? probe.host : ""
  const port = Number(probe.port)
  if (!host || !Number.isInteger(port) || port < 1 || port > 65535) throw new ProbeError("network probe is invalid")
  const addresses = await dns.lookup(host, { all: true, verbatim: true })
  const families = [...new Set(addresses.map((entry) => entry.family).sort())]
  if (families.length === 0) throw new ProbeError("network probe returned no address families")
  const reachable = await connectTcp(host, port)
  const observed = { endpoint: safeIdentifier(String(probe.name ?? `${host}:${port}`), "network endpoint"), port, address_families: families, reachable }
  const networkFingerprint = fingerprint(observed)
  const baseline = parseJsonEnv("CHARIOX_PARITY_ORDINARY_NETWORK_JSON")
  if (networkFingerprint !== baseline.network_fingerprint || reachable !== baseline.reachable || canonicalJson(families) !== canonicalJson(baseline.address_families)) {
    throw new ProbeError("network reachability differs from the observed ordinary baseline")
  }
  return identityResult(identity, {
    network_matches_ordinary: true,
    address_families_recorded: true,
    network_fingerprint: networkFingerprint,
    address_families: families,
    reachable,
  })
}

async function observePackageTool(identity) {
  const probe = parseJsonEnv("CHARIOX_PARITY_PACKAGE_PROBE_JSON")
  if (typeof probe.command !== "string" || !Array.isArray(probe.args)
    || typeof probe.install_command !== "string" || !Array.isArray(probe.install_args)) {
    throw new ProbeError("package/tool probe is invalid")
  }
  const tool = await runCommand(probe.command, probe.args)
  const install = await runCommand(probe.install_command, probe.install_args)
  const expectedTool = Number.isInteger(probe.expected_exit_code) ? probe.expected_exit_code : 0
  const expectedInstall = Number.isInteger(probe.install_expected_exit_code) ? probe.install_expected_exit_code : 0
  const toolProbeSucceeded = tool.code === expectedTool && !tool.signal
  const installProbeSucceeded = install.code === expectedInstall && !install.signal
  if (!toolProbeSucceeded || !installProbeSucceeded) throw new ProbeError("package/tool probe failed")
  return identityResult(identity, {
    tool_probe_succeeded: true,
    install_probe_succeeded: true,
    tool: safeIdentifier(probe.tool ?? basename(probe.command), "package tool"),
    operation: safeIdentifier(probe.operation ?? "permitted-installation-probe", "package operation"),
    tool_stdout_fingerprint: fingerprint(tool.stdout),
    install_stdout_fingerprint: fingerprint(install.stdout),
  })
}

async function observeSessionAgent(identity) {
  const evidence = requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_SESSION_AGENT_EVIDENCE_JSON"), "session/agent evidence")
  if (typeof evidence.session_id !== "string" || evidence.session_id.length === 0
    || typeof evidence.agent_id !== "string" || evidence.agent_id.length === 0
    || evidence.session_created !== true || evidence.agent_created !== true || evidence.official_command !== true) {
    throw new ProbeError("session/agent evidence is incomplete")
  }
  return identityResult(identity, {
    session_created: true,
    agent_created: true,
    official_command: true,
    session_identity_fingerprint: fingerprint(evidence.session_id),
    agent_identity_fingerprint: fingerprint(evidence.agent_id),
  })
}

async function observeTerminalFileGit(identity, values) {
  const scratch = await mkdtemp(join("/tmp", "chariox-parity-terminal-"))
  try {
    const file = join(scratch, "terminal-file.txt")
    const marker = `chariox-parity-${process.pid}`
    const terminal = await runCommand("sh", ["-c", "pwd >/dev/null"], { cwd: values.source_root })
    await writeFile(file, marker, "utf8")
    const readBack = await readFile(file, "utf8")
    const git = await runCommand("git", ["status", "--porcelain=1", "--untracked-files=all"], { cwd: values.source_root })
    if (terminal.code !== 0 || terminal.signal || readBack !== marker || git.code !== 0 || git.signal) throw new ProbeError("terminal/file/git product observation failed")
    return identityResult(identity, {
      terminal_ok: true,
      file_ok: true,
      git_ok: true,
      terminal_cwd_fingerprint: fingerprint(resolve(values.source_root)),
      file_marker_fingerprint: fingerprint(marker),
    })
  } finally {
    await rm(scratch, { recursive: true, force: false })
  }
}

async function observeAttachments(identity) {
  const attachmentPath = process.env.CHARIOX_PARITY_ATTACHMENT_PATH
  if (!attachmentPath) throw new ProbeError("attachment observation path is missing")
  await accessible(attachmentPath, constants.R_OK | constants.W_OK)
  const metadata = await stat(attachmentPath)
  const status = await readFile("/proc/self/status", "utf8")
  const capEff = /^CapEff:\s*([0-9a-f]+)$/im.exec(status)?.[1]?.toLowerCase()
  if (!capEff) throw new ProbeError("attachment capability observation is missing")
  const mode = (metadata.mode & 0o7777).toString(8).padStart(4, "0")
  const baseline = parseJsonEnv("CHARIOX_PARITY_ORDINARY_ATTACHMENT_JSON")
  if (mode !== baseline.mode || capEff !== String(baseline.cap_eff ?? "").toLowerCase()) throw new ProbeError("attachment permissions differ from ordinary baseline")
  return identityResult(identity, {
    attachments_ok: true,
    permissions_ok: true,
    capabilities_ok: true,
    attachment_path_fingerprint: fingerprint(await realpath(attachmentPath)),
    permission_mode: mode,
    cap_eff: capEff,
  })
}

async function observeProjectSetup(identity) {
  const evidence = requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_PROJECT_SETUP_EVIDENCE_JSON"), "project setup evidence")
  if (evidence.project_setup_ok !== true || typeof evidence.project_identity !== "string" || evidence.project_identity.length === 0) {
    throw new ProbeError("project setup evidence is incomplete")
  }
  return identityResult(identity, { project_setup_ok: true, project_identity_fingerprint: fingerprint(evidence.project_identity) })
}

const REPOSITORY_ROOT_REQUIREMENTS = Object.freeze({
  repository_root_default: "default_root_correct",
  repository_root_custom: "custom_root_persisted",
  repository_root_inheritance: "child_inherits_root",
  repository_root_override_rejected: "client_override_rejected",
})

async function observeRepositoryRoot(identity, checkId) {
  const requiredKey = REPOSITORY_ROOT_REQUIREMENTS[checkId]
  if (!requiredKey) throw new ProbeError(`unsupported repository-root check: ${checkId}`)
  const evidence = requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_REPOSITORY_ROOT_EVIDENCE_JSON"), "repository-root evidence")
  const entry = requireObservedEvidence(evidence[checkId], `repository-root evidence for ${checkId}`)
  if (entry[requiredKey] !== true) throw new ProbeError(`${checkId} product observation is incomplete`)
  return identityResult(identity, {
    [requiredKey]: true,
    repository_root_evidence_fingerprint: fingerprint(entry.evidence_id ?? `${checkId}:${process.pid}`),
  })
}

const LIFECYCLE_REQUIREMENTS = Object.freeze({
  reconnect_orphan_recovery: ["reconnect_ok", "orphan_recovered"],
  restart_recovery: ["restart_recovered"],
  reconnect_history_result_identity: ["history_preserved", "result_identity_preserved"],
  queued_prompts: ["queued_prompt_preserved", "queued_prompt_advanced"],
  active_turn_state: ["active_turn_state_preserved"],
})

async function observeLifecycle(identity, checkId) {
  const evidence = parseJsonEnv("CHARIOX_PARITY_LIFECYCLE_EVIDENCE_JSON")
  const entry = requireObservedEvidence(evidence[checkId], `lifecycle evidence for ${checkId}`)
  for (const key of LIFECYCLE_REQUIREMENTS[checkId] ?? []) if (entry[key] !== true) throw new ProbeError(`${checkId} is incomplete`)
  return identityResult(identity, {
    ...Object.fromEntries((LIFECYCLE_REQUIREMENTS[checkId] ?? []).map((key) => [key, true])),
    evidence_identity_fingerprint: fingerprint(entry.evidence_id ?? `${checkId}:${process.pid}`),
  })
}

async function observeControlFileProtection(identity) {
  const controlFile = process.env.CHARIOX_PARITY_CONTROL_FILE
  const sibling = process.env.CHARIOX_PARITY_CONTROL_SIBLING
  if (!controlFile || !sibling) throw new ProbeError("control file protection paths are missing")
  let controlDenied = false
  try {
    await accessible(controlFile, constants.R_OK | constants.W_OK)
  } catch (error) {
    if (error?.code !== "EACCES" && error?.code !== "EPERM") throw error
    controlDenied = true
  }
  if (!controlDenied) {
    const productEvidence = requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_CONTROL_PROTECTION_EVIDENCE_JSON"), "control file protection")
    controlDenied = productEvidence.control_file_denied === true
  }
  const parentWorkspace = dirname(resolve(controlFile))
  let parentWorkspaceAccessible = false
  try {
    await accessible(parentWorkspace, constants.R_OK | constants.W_OK | constants.X_OK)
    parentWorkspaceAccessible = true
  } catch (error) {
    if (error?.code !== "EACCES" && error?.code !== "EPERM") throw error
  }
  const siblingMetadata = await stat(sibling)
  const siblingAccessMode = constants.R_OK | constants.W_OK
    | (siblingMetadata.isDirectory() ? constants.X_OK : 0)
  let siblingAccessible = false
  try {
    await accessible(sibling, siblingAccessMode)
    siblingAccessible = true
  } catch (error) {
    if (error?.code !== "EACCES" && error?.code !== "EPERM") throw error
  }
  if (!controlDenied) throw new ProbeError("exact control file protection was not observed")
  if (!parentWorkspaceAccessible) throw new ProbeError("control file parent workspace is not accessible for workspace operations")
  if (!siblingAccessible) throw new ProbeError("control file sibling workspace is not accessible for workspace operations")
  return identityResult(identity, {
    control_file_denied: true,
    parent_workspace_accessible: true,
    sibling_accessible: true,
    control_path_fingerprint: fingerprint(resolve(controlFile)),
    sibling_path_fingerprint: fingerprint(await realpath(sibling)),
  })
}

async function observeFilesystemPermissions(identity, values) {
  const metadata = await stat(values.source_root)
  const mode = (metadata.mode & 0o7777).toString(8).padStart(4, "0")
  const uid = metadata.uid
  const gid = metadata.gid
  const baseline = parseJsonEnv("CHARIOX_PARITY_ORDINARY_FILESYSTEM_PERMISSIONS_JSON")
  if (mode !== baseline.mode || uid !== baseline.uid || gid !== baseline.gid) throw new ProbeError("filesystem permissions differ from ordinary baseline")
  return identityResult(identity, { permissions_match_ordinary: true, mode, uid, gid, path_fingerprint: fingerprint(resolve(values.source_root)) })
}

function normalizeLimits(text) {
  return String(text).split("\n").filter(Boolean).map((line) => line.trim().replace(/\s+/g, " ")).sort()
}

async function observeResourceLimits(identity) {
  const limits = normalizeLimits(await readFile("/proc/self/limits", "utf8"))
  const limitsFingerprint = fingerprint(limits)
  const baseline = process.env.CHARIOX_PARITY_ORDINARY_RESOURCE_LIMITS_FINGERPRINT
  if (!DIGEST.test(baseline ?? "") || limitsFingerprint !== baseline) throw new ProbeError("resource limits differ from ordinary baseline")
  return identityResult(identity, { limits_observed: true, limits_fingerprint: limitsFingerprint, limit_count: limits.length })
}

async function observeStructuredErrors(identity) {
  const evidence = requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_STRUCTURED_ERROR_EVIDENCE_JSON"), "structured error evidence")
  if (evidence.structured_errors !== true || typeof evidence.error_code !== "string" || evidence.error_code.length === 0) throw new ProbeError("structured error evidence is incomplete")
  return identityResult(identity, { structured_errors: true, error_code: safeIdentifier(evidence.error_code, "error code"), retryable: evidence.retryable === true })
}

async function observeProtocol(identity) {
  const evidence = requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_PROTOCOL_EVIDENCE_JSON"), "protocol evidence")
  if (evidence.protocol_behavior_ok !== true || typeof evidence.protocol_identity !== "string" || evidence.protocol_identity.length === 0) throw new ProbeError("protocol behavior evidence is incomplete")
  return identityResult(identity, { protocol_behavior_ok: true, protocol_identity_fingerprint: fingerprint(evidence.protocol_identity) })
}

async function observeCleanup(identity) {
  const evidence = requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_CLEANUP_RESULT_JSON"), "cleanup evidence")
  for (const key of ["owned_processes_gone", "owned_artifacts_removed", "foreign_processes_untouched", "cleanup_complete"]) {
    if (evidence[key] !== true) throw new ProbeError(`cleanup evidence is incomplete: ${key}`)
  }
  return identityResult(identity, {
    owned_processes_gone: true,
    owned_artifacts_removed: true,
    foreign_processes_untouched: true,
    cleanup_complete: true,
    cleanup_receipt_fingerprint: fingerprint(evidence.receipt_id ?? `cleanup:${process.pid}`),
  })
}

async function observeRelease(identity) {
  const evidence = requireObservedEvidence(parseJsonEnv("CHARIOX_PARITY_RELEASE_EVIDENCE_JSON"), "release evidence")
  for (const key of ["ordinary_release_not_applicable", "signed_release_present", "signature_verified", "atomic_activation", "rollback_verified"]) {
    if (typeof evidence[key] !== "boolean") throw new ProbeError(`release evidence is missing ${key}`)
  }
  if (typeof evidence.release_digest !== "string" || (evidence.signed_release_present && !DIGEST.test(evidence.release_digest))) {
    throw new ProbeError("release evidence has no valid digest")
  }
  return identityResult(identity, {
    ordinary_release_not_applicable: evidence.ordinary_release_not_applicable,
    signed_release_present: evidence.signed_release_present,
    signature_verified: evidence.signature_verified,
    atomic_activation: evidence.atomic_activation,
    rollback_verified: evidence.rollback_verified,
    release_digest: evidence.release_digest,
    release_receipt_fingerprint: fingerprint(evidence.receipt_id ?? `release:${process.pid}`),
  })
}

const SHUTDOWN_CHECKS = new Set([...CHECKS.get("MP-09")])

async function observeShutdown(identity, values, checkId) {
  if (!SHUTDOWN_CHECKS.has(checkId)) throw new ProbeError(`unsupported shutdown check: ${checkId}`)
  const evidence = parseJsonEnv("CHARIOX_PARITY_SHUTDOWN_EVIDENCE_JSON")
  const trigger = checkId.slice("shutdown_".length)
  const entry = requireObservedEvidence(evidence[trigger], `shutdown evidence for ${trigger}`)
  const expectation = SHUTDOWN_EXPECTATIONS[trigger]
  if (!expectation) throw new ProbeError(`shutdown evidence has no reviewed expectation for ${trigger}`)
  if (entry.trigger && entry.trigger !== trigger) throw new ProbeError(`shutdown evidence trigger mismatch for ${trigger}`)
  const ordinary = values.topology === "ordinary"
  const expectedOutcome = ordinary ? "ordinary-remained-running" : expectation.outcome
  const expectedManagedPolicy = !ordinary
  const expectedWorkerStopped = ordinary ? false : expectation.workerStopped
  const expectedCleanupConfirmed = ordinary ? false : expectation.cleanupConfirmed
  const expectedMeasuredBoundary = ordinary ? false : expectation.measuredFromLastAgentFinished
  if (typeof entry.observed_outcome !== "string" || entry.observed_outcome !== expectedOutcome
    || entry.expected_outcome !== expectedOutcome
    || entry.managed_policy !== expectedManagedPolicy
    || entry.observation_complete !== true
    || entry.worker_stopped !== expectedWorkerStopped
    || entry.cleanup_confirmed !== expectedCleanupConfirmed
    || entry.measured_from_last_agent_finished !== expectedMeasuredBoundary) {
    throw new ProbeError(`shutdown evidence is incomplete for ${trigger}`)
  }
  const configuredDelay = entry.configured_delay_seconds ?? null
  const observedDelay = entry.observed_delay_seconds ?? null
  if (expectation.delay === null || ordinary) {
    if (configuredDelay !== null || observedDelay !== null) throw new ProbeError(`shutdown delay evidence is invalid for ${trigger}`)
  } else {
    const configuredValid = Number.isInteger(configuredDelay) && configuredDelay >= 0
      && (expectation.delay === "positive" ? configuredDelay > 0 : configuredDelay === expectation.delay)
    if (!configuredValid) throw new ProbeError(`shutdown configured delay is invalid for ${trigger}`)
    if (expectation.observesDelay) {
      if (!Number.isFinite(observedDelay) || observedDelay < configuredDelay || observedDelay > configuredDelay + 120) {
        throw new ProbeError(`shutdown observed delay is invalid for ${trigger}`)
      }
    } else if (observedDelay !== null) {
      throw new ProbeError(`shutdown observed delay is not expected for ${trigger}`)
    }
  }
  return identityResult(identity, {
    trigger,
    expected_outcome: expectedOutcome,
    observed_outcome: expectedOutcome,
    managed_policy: expectedManagedPolicy,
    observation_complete: true,
    worker_stopped: expectedWorkerStopped,
    cleanup_confirmed: expectedCleanupConfirmed,
    measured_from_last_agent_finished: expectedMeasuredBoundary,
    configured_delay_seconds: configuredDelay,
    observed_delay_seconds: observedDelay,
    shutdown_receipt_fingerprint: fingerprint(entry.receipt_id ?? `${trigger}:${process.pid}`),
  })
}

async function observe(values) {
  const identity = await verifyProbeIdentity(values)
  const { parity_row: rowId, parity_check: checkId } = values
  if (rowId === "MP-10" && checkId === "fresh_worker") return observeFreshWorker(identity)
  if (rowId === "MP-08" && checkId === "official_provider_identity") return observeProviderIdentity(identity, values)
  if (rowId === "MP-10" && checkId === "capture_boundary") return observeCaptureBoundary(identity)
  if (rowId === "MP-02") return observeDirectoryCheck(identity, values, checkId)
  if (rowId === "MP-03" && checkId === "control_file_protection") return observeControlFileProtection(identity)
  if (rowId === "MP-03" && checkId === "filesystem_permissions") return observeFilesystemPermissions(identity, values)
  if (rowId === "MP-01" && checkId === "provider_ancestry") return observeProviderAncestry(identity, values)
  if (rowId === "MP-04" && checkId === "provider_environment") return observeProviderEnvironment(identity, values)
  if (rowId === "MP-01" && checkId === "managed_isolation_environment") return observeIsolationEnvironment(identity)
  if (rowId === "MP-01" && checkId === "mount_visibility") return observeMountVisibility(identity, values)
  if (rowId === "MP-01" && checkId === "privilege_state") return observePrivilegeState(identity)
  if (rowId === "MP-01" && checkId === "network_reachability") return observeNetwork(identity)
  if (rowId === "MP-01" && checkId === "package_tool_installation") return observePackageTool(identity)
  if (rowId === "MP-05") return observeWorkspaceCheck(identity, values, checkId)
  if (rowId === "MP-06") return observeRepositoryRoot(identity, checkId)
  if (rowId === "MP-08" && checkId === "session_agent_launch") return observeSessionAgent(identity)
  if (rowId === "MP-08" && checkId === "terminal_file_git") return observeTerminalFileGit(identity, values)
  if (rowId === "MP-08" && checkId === "attachments_permissions_capabilities") return observeAttachments(identity)
  if (rowId === "MP-08" && checkId === "project_setup") return observeProjectSetup(identity)
  if (rowId === "MP-08" && checkId === "resource_limits") return observeResourceLimits(identity)
  if (rowId === "MP-08" && checkId === "structured_errors") return observeStructuredErrors(identity)
  if (rowId === "MP-08" && checkId === "protocol_behavior") return observeProtocol(identity)
  if (rowId === "MP-08" && checkId === "cleanup") return observeCleanup(identity)
  if (rowId === "MP-07" && checkId === "signed_release_activation") return observeRelease(identity)
  if (rowId === "MP-09") return observeShutdown(identity, values, checkId)
  throw new ProbeError(`${rowId}/${checkId} has no product-backed observation`)
}

async function main(argv = process.argv.slice(2)) {
  let values
  try {
    values = parseArgs(argv)
    const result = await observe(values)
    process.stdout.write(`${JSON.stringify({ ok: true, result })}\n`)
    return 0
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    if (values?.json) process.stdout.write(`${JSON.stringify({ ok: false, error: message })}\n`)
    else process.stderr.write(`${message}\n`)
    return 1
  }
}

if (import.meta.url === `file://${process.argv[1]}`) process.exitCode = await main()

export { main, observe, parseArgs, verifyProbeIdentity }
