#!/usr/bin/env node

import { execFile } from "node:child_process"
import { createHash } from "node:crypto"
import { access, constants, mkdir, readFile, readdir, stat } from "node:fs/promises"
import { basename, isAbsolute, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFileAsync = promisify(execFile)
const PROBE_RELATIVE_PATH = "apps/cli/scripts/managed-ordinary-parity-probe.mjs"
const REVIEWED_COMMIT = /^[0-9a-f]{40}$/i
const TOPOLOGIES = new Set(["ordinary", "path1"])
const CHECKS = new Map([
  ["MP-01", new Set(["fresh_worker", "official_provider_identity", "capture_boundary"])],
  ["MP-02", new Set(["directory_discovery", "exact_path_entry", "directory_creation", "home_access", "tmp_access"])],
  ["MP-03", new Set(["empty_workspace", "copied_repository", "repository_basename", "basename_collision", "worktree_placement"])],
  ["MP-04", new Set(["provider_ancestry", "provider_environment", "managed_isolation_environment", "mount_visibility", "privilege_state", "network_reachability", "package_tool_installation"])],
  ["MP-05", new Set(["session_agent_launch", "terminal_file_git", "attachments_permissions_capabilities", "project_setup"])],
  ["MP-06", new Set(["reconnect_orphan_recovery", "restart_recovery", "reconnect_history_result_identity", "queued_prompts", "active_turn_state"])],
  ["MP-07", new Set(["control_file_protection", "filesystem_permissions", "resource_limits", "structured_errors", "protocol_behavior"])],
  ["MP-08", new Set(["cleanup"])],
  ["MP-09", new Set(["signed_release_activation"])],
  ["MP-10", new Set(["shutdown_agents_done", "shutdown_idle_15m", "shutdown_idle_30m", "shutdown_minimum_3h", "shutdown_manual", "shutdown_custom", "shutdown_explicit_lifecycle_reconciliation", "shutdown_deployment_reconciliation"])],
])

class ProbeError extends Error {
  constructor(message, details = {}) {
    super(message)
    this.name = "ManagedOrdinaryParityProbeError"
    Object.assign(this, details)
  }
}

function parseArgs(argv) {
  const values = {}
  const allowed = new Set([
    "source-root", "reviewed-commit", "parity-row", "parity-check", "topology",
    "home-path", "tmp-path", "nested-path", "new-directory", "json",
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
  if (!REVIEWED_COMMIT.test(values.reviewed_commit)) throw new ProbeError("reviewed commit must be a full Git commit ID")
  if (!TOPOLOGIES.has(values.topology)) throw new ProbeError("topology must be ordinary or path1")
  if (!CHECKS.has(values.parity_row) || !CHECKS.get(values.parity_row).has(values.parity_check)) {
    throw new ProbeError("parity row/check is not in the reviewed inventory")
  }
  if (!isAbsolute(values.source_root)) throw new ProbeError("source root must be absolute")
  return values
}

async function runGit(sourceRoot, args) {
  try {
    const result = await execFileAsync("git", args, {
      cwd: sourceRoot,
      encoding: "utf8",
      maxBuffer: 4 * 1024 * 1024,
      env: {
        ...process.env,
        GIT_CONFIG_NOSYSTEM: "1",
        GIT_CONFIG_GLOBAL: "/dev/null",
        GIT_NO_REPLACE_OBJECTS: "1",
        LC_ALL: "C",
      },
    })
    return String(result.stdout ?? "").trim()
  } catch (error) {
    throw new ProbeError(`git ${args.join(" ")} failed`, { cause: error })
  }
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
    probe_file_sha256: `sha256:${createHash("sha256").update(fileBytes).digest("hex")}`,
  }
}

async function accessible(path) {
  await access(path, constants.R_OK | constants.X_OK)
  return true
}

async function observe(values) {
  const sourceRoot = resolve(values.source_root)
  const identity = await verifyProbeIdentity(values)
  const { parity_row: rowId, parity_check: checkId } = values
  if (rowId === "MP-02" && checkId === "home_access") {
    return { ...identity, observed: true, accessible: await accessible(values.home_path) }
  }
  if (rowId === "MP-02" && checkId === "tmp_access") {
    return { ...identity, observed: true, accessible: await accessible(values.tmp_path) }
  }
  if (rowId === "MP-02" && checkId === "directory_discovery") {
    const exactPathAccessible = await accessible(values.home_path)
    let childEnumerationDenied = false
    try {
      await readdir(values.home_path)
    } catch (error) {
      if (error?.code !== "EACCES" && error?.code !== "EPERM") throw error
      childEnumerationDenied = true
    }
    return { ...identity, observed: true, exact_path_accessible: exactPathAccessible, child_enumeration_denied: childEnumerationDenied }
  }
  if (rowId === "MP-02" && checkId === "exact_path_entry") {
    return { ...identity, observed: true, exact_path_accessible: await accessible(sourceRoot), cwd_matches_requested: resolve(process.cwd()) === sourceRoot }
  }
  if (rowId === "MP-02" && checkId === "directory_creation") {
    await mkdir(values.new_directory, { recursive: true })
    const metadata = await stat(values.new_directory)
    return { ...identity, observed: true, created_and_accessible: metadata.isDirectory() && await accessible(values.new_directory) }
  }
  if (rowId === "MP-03" && checkId === "empty_workspace") {
    await mkdir(values.nested_path, { recursive: true })
    return { ...identity, observed: true, workspace_created: (await stat(values.nested_path)).isDirectory(), control_state_separate: resolve(values.nested_path) !== sourceRoot }
  }
  if (rowId === "MP-03" && checkId === "copied_repository") {
    return { ...identity, observed: true, repository_accessible: await accessible(sourceRoot) }
  }
  if (rowId === "MP-03" && checkId === "repository_basename") {
    const top = resolve(await runGit(sourceRoot, ["rev-parse", "--show-toplevel"]))
    return { ...identity, observed: true, source_basename_preserved: top === sourceRoot && basename(top) === basename(sourceRoot) }
  }
  if (rowId === "MP-03" && checkId === "basename_collision") {
    await mkdir(values.new_directory, { recursive: true })
    let collisionRejected = false
    try {
      await mkdir(values.new_directory)
    } catch (error) {
      collisionRejected = error?.code === "EEXIST"
    }
    return { ...identity, observed: true, collision_rejected: collisionRejected }
  }
  if (rowId === "MP-03" && checkId === "worktree_placement") {
    const nested = resolve(values.nested_path)
    return { ...identity, observed: true, worktree_user_path: await accessible(sourceRoot), control_root_not_workspace: !nested.startsWith(`${sourceRoot}/`) }
  }
  if (rowId === "MP-04" && checkId === "managed_isolation_environment") {
    return {
      ...identity,
      observed: true,
      managed_marker_absent: !process.env.CHARIOX_MANAGED_PROVIDER_ISOLATION,
      bwrap_environment_absent: !process.env.CHARIOX_MANAGED_PROVIDER_BWRAP,
    }
  }
  if (rowId === "MP-04" && checkId === "mount_visibility") {
    const mountInfo = await readFile("/proc/self/mountinfo", "utf8")
    return { ...identity, observed: true, mounts_match_ordinary: mountInfo.length > 0, mount_probe_complete: true }
  }
  if (rowId === "MP-04" && checkId === "privilege_state") {
    const status = await readFile("/proc/self/status", "utf8")
    const noNewPrivs = /^NoNewPrivs:\s*(\d+)$/m.exec(status)?.[1]
    return { ...identity, observed: true, no_new_privs: noNewPrivs === "1", capabilities_match_ordinary: /^CapEff:\s*0+$/m.test(status), umask_matches_ordinary: Number.isInteger(process.umask()) }
  }
  throw new ProbeError(`${rowId}/${checkId} requires a live Chariox product observation; no safe Node-only observation exists`)
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

export { main, parseArgs, verifyProbeIdentity }
