import { spawn } from "node:child_process"
import { chmod, copyFile, cp, mkdir, mkdtemp, open, opendir, rm, stat } from "node:fs/promises"
import os from "node:os"
import path from "node:path"

import {
  realProviderEnv,
  repoRoot,
  runLoggedCommand,
} from "./live-provider-thread-transfer-runtime.mjs"

export function safeLabel(value) {
  return value.replace(/[^a-zA-Z0-9_.-]+/g, "-")
}

function safePathComponent(value, label) {
  if (
    typeof value !== "string"
    || value.length === 0
    || value.length > 120
    || value === "."
    || value === ".."
    || !/^[a-zA-Z0-9_.-]+$/.test(value)
  ) {
    throw new Error(`${label} is not a safe path component`)
  }
  return value
}

export function materializedProviderEnvironment({
  provider,
  storageRoot,
  ownerUserId,
  profileId,
}) {
  if (!path.isAbsolute(storageRoot)) {
    throw new Error("worker storage root must be absolute")
  }
  const normalizedProvider = safePathComponent(provider, "provider")
  const profileRoot = path.join(
    storageRoot,
    "provider-accounts",
    safePathComponent(ownerUserId, "owner user id"),
    normalizedProvider,
    safePathComponent(profileId, "profile id"),
  )
  if (normalizedProvider === "codex") {
    return { CODEX_HOME: path.join(profileRoot, "codex") }
  }
  if (normalizedProvider === "opencode") {
    const configRoot = path.join(profileRoot, "config")
    return {
      XDG_DATA_HOME: path.join(profileRoot, "data"),
      XDG_CONFIG_HOME: configRoot,
      XDG_STATE_HOME: path.join(profileRoot, "state"),
      XDG_CACHE_HOME: path.join(profileRoot, "cache"),
      OPENCODE_CONFIG_DIR: path.join(configRoot, "opencode"),
    }
  }
  if (normalizedProvider === "claude") {
    return { CLAUDE_CONFIG_DIR: path.join(profileRoot, "claude") }
  }
  throw new Error(`unsupported materialized provider environment: ${provider}`)
}

async function findCodexRollout(codexHome, providerSessionId) {
  const expectedSuffix = `-${safePathComponent(providerSessionId, "provider session id")}.jsonl`
  const pending = [
    path.join(codexHome, "sessions"),
    path.join(codexHome, "archived_sessions"),
  ]
  let visitedEntries = 0
  while (pending.length > 0) {
    const directory = pending.pop()
    let handle
    try {
      handle = await opendir(directory)
    } catch (error) {
      if (error?.code === "ENOENT") continue
      throw error
    }
    for await (const entry of handle) {
      visitedEntries += 1
      if (visitedEntries > 250_000) {
        throw new Error("Codex rollout search exceeded its entry limit")
      }
      const pathname = path.join(directory, entry.name)
      if (entry.isDirectory()) pending.push(pathname)
      else if (entry.isFile() && entry.name.endsWith(expectedSuffix)) return pathname
    }
  }
  return null
}

async function runOpenCodeStateCommand(
  command,
  args,
  providerEnv,
  stdoutFd = "ignore",
  timeoutMs = 60_000,
) {
  const child = spawn(command, args, {
    env: {
      ...process.env,
      ...providerEnv,
    },
    stdio: ["ignore", stdoutFd, "pipe"],
  })
  let stderrBytes = 0
  child.stderr?.on("data", (chunk) => {
    stderrBytes += chunk.length
  })
  let timedOut = false
  const timeout = setTimeout(() => {
    timedOut = true
    child.kill("SIGKILL")
  }, timeoutMs)
  timeout.unref()
  let status
  try {
    status = await new Promise((resolve, reject) => {
      child.once("error", reject)
      child.once("close", (code, signal) => resolve({ code, signal }))
    })
  } finally {
    clearTimeout(timeout)
  }
  if (timedOut) {
    throw new Error(`OpenCode ${args[0]} timed out after ${timeoutMs} ms`)
  }
  if (status.code !== 0) {
    throw new Error(
      `OpenCode ${args[0]} failed with ${status.signal ?? `exit ${status.code}`}; stderr bytes: ${stderrBytes}`,
    )
  }
}

async function transferOpenCodeThreadState({
  providerSessionId,
  sourceProviderEnv,
  destinationProviderEnv,
  openCodeCommand,
  openCodeCommandTimeoutMs,
}) {
  safePathComponent(providerSessionId, "provider session id")
  if (!sourceProviderEnv.XDG_DATA_HOME || !destinationProviderEnv.XDG_DATA_HOME) {
    throw new Error("OpenCode thread transfer requires source and destination XDG_DATA_HOME")
  }
  const temporaryRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-opencode-thread-transfer-"))
  const exportPath = path.join(temporaryRoot, "session.json")
  try {
    const exportFile = await open(exportPath, "w", 0o600)
    try {
      await runOpenCodeStateCommand(
        openCodeCommand,
        ["export", providerSessionId],
        sourceProviderEnv,
        exportFile.fd,
        openCodeCommandTimeoutMs,
      )
    } finally {
      await exportFile.close()
    }
    const exportBytes = (await stat(exportPath)).size
    if (exportBytes === 0 || exportBytes > 64 * 1024 * 1024) {
      throw new Error(`OpenCode session export has invalid size: ${exportBytes} bytes`)
    }
    await runOpenCodeStateCommand(
      openCodeCommand,
      ["import", exportPath],
      destinationProviderEnv,
      "ignore",
      openCodeCommandTimeoutMs,
    )
    return {
      provider: "opencode",
      provider_session_id: providerSessionId,
      copied: [{
        kind: "opencode_session_export",
        byte_length: exportBytes,
      }],
    }
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true })
  }
}

export async function transferProviderThreadStateToWorker({
  provider,
  providerSessionId,
  sourceProviderEnv,
  destinationProviderEnv,
  openCodeCommand = process.env.CHARIOX_OPENCODE_BIN?.trim() || "opencode",
  openCodeCommandTimeoutMs = 60_000,
}) {
  if (provider === "opencode") {
    return transferOpenCodeThreadState({
      providerSessionId,
      sourceProviderEnv,
      destinationProviderEnv,
      openCodeCommand,
      openCodeCommandTimeoutMs,
    })
  }
  if (provider !== "codex") {
    throw new Error(`provider thread state transfer is not implemented for ${provider}`)
  }
  const sourceCodexHome = sourceProviderEnv.CODEX_HOME
  const destinationCodexHome = destinationProviderEnv.CODEX_HOME
  if (!sourceCodexHome || !destinationCodexHome) {
    throw new Error("Codex thread transfer requires source and destination CODEX_HOME")
  }
  const source = await findCodexRollout(sourceCodexHome, providerSessionId)
  if (!source) {
    throw new Error(`Codex rollout not found for provider session ${providerSessionId}`)
  }
  const relativePath = path.relative(sourceCodexHome, source)
  if (relativePath.startsWith("..") || path.isAbsolute(relativePath)) {
    throw new Error("Codex rollout escaped the source provider home")
  }
  const destination = path.join(destinationCodexHome, relativePath)
  await mkdir(path.dirname(destination), { recursive: true })
  await copyFile(source, destination)
  await chmod(destination, 0o600)
  return {
    provider,
    provider_session_id: providerSessionId,
    copied: [{
      kind: "codex_rollout",
      relative_path: relativePath,
      byte_length: (await stat(destination)).size,
    }],
  }
}

export async function pathStat(pathname) {
  try {
    return await stat(pathname)
  } catch (error) {
    if (error?.code === "ENOENT") return null
    throw error
  }
}

export function providerStateCopySpecs(provider, providerEnv = realProviderEnv()) {
  if (provider === "codex") {
    return [
      {
        label: "codex-home",
        source: providerEnv.CODEX_HOME,
        target: "/home/slice/.codex",
        kind: "dir",
      },
    ]
  }
  if (provider === "opencode") {
    const opencodeDataHome = providerEnv.OPENCODE_DATA_HOME
      ?? path.join(providerEnv.XDG_DATA_HOME, "opencode")
    return [
      {
        label: "opencode-data",
        source: opencodeDataHome,
        target: "/home/slice/.local/share/opencode",
        kind: "dir",
      },
      {
        label: "opencode-config",
        source: providerEnv.OPENCODE_CONFIG_DIR,
        target: "/home/slice/.config/opencode",
        kind: "dir",
      },
    ]
  }
  if (provider === "claude-p" || provider === "claude-headless" || provider === "claude") {
    const home = providerEnv.HOME ?? process.env.HOME ?? os.homedir()
    return [
      {
        label: "claude-home",
        source: path.join(home, ".claude"),
        target: "/home/slice/.claude",
        kind: "dir",
      },
      {
        label: "claude-json",
        source: path.join(home, ".claude.json"),
        target: "/home/slice/.claude.json",
        kind: "file",
      },
    ]
  }
  return []
}

export async function transferProviderStateToWorker({
  provider,
  sourceProviderEnv,
  destinationProviderEnv,
}) {
  const destinations = new Map(
    providerStateCopySpecs(provider, destinationProviderEnv)
      .map((spec) => [spec.label, spec]),
  )
  const evidence = {
    provider,
    copied: [],
    missing: [],
  }
  for (const source of providerStateCopySpecs(provider, sourceProviderEnv)) {
    const destination = destinations.get(source.label)
    if (!destination) continue
    const sourceStat = await pathStat(source.source)
    if (!sourceStat) {
      evidence.missing.push({ label: source.label, kind: source.kind })
      continue
    }
    if (source.kind === "dir") {
      await mkdir(destination.source, { recursive: true })
      await cp(source.source, destination.source, {
        recursive: true,
        force: true,
      })
    } else {
      await mkdir(path.dirname(destination.source), { recursive: true })
      await copyFile(source.source, destination.source)
      await chmod(destination.source, 0o600).catch(() => {})
    }
    evidence.copied.push({
      label: source.label,
      kind: source.kind,
      destination: destination.source,
    })
  }
  return evidence
}

export async function runDockerCommandForTransfer(root, label, args, timeoutMs) {
  await runLoggedCommand("docker", args, {
    cwd: repoRoot,
    env: process.env,
    stdoutPath: path.join(root, `${safeLabel(label)}.stdout.log`),
    stderrPath: path.join(root, `${safeLabel(label)}.stderr.log`),
    timeoutMs,
  })
}

export async function transferProviderStateToSlice({ provider, root, sliceName, timeoutMs, providerEnv }) {
  const container = `chariox-slice-${sliceName}`
  const evidence = {
    provider,
    container,
    copied: [],
    missing: [],
  }
  for (const spec of providerStateCopySpecs(provider, providerEnv)) {
    const sourceStat = await pathStat(spec.source)
    if (!sourceStat) {
      evidence.missing.push({
        label: spec.label,
        kind: spec.kind,
        target: spec.target,
      })
      continue
    }
    if (spec.kind === "dir" && !sourceStat.isDirectory()) {
      evidence.missing.push({
        label: spec.label,
        kind: spec.kind,
        target: spec.target,
        reason: "source is not a directory",
      })
      continue
    }
    if (spec.kind === "file" && !sourceStat.isFile()) {
      evidence.missing.push({
        label: spec.label,
        kind: spec.kind,
        target: spec.target,
        reason: "source is not a file",
      })
      continue
    }

    const targetDir = spec.kind === "file" ? path.posix.dirname(spec.target) : spec.target
    await runDockerCommandForTransfer(
      root,
      `${provider}-${spec.label}-mkdir`,
      ["exec", "-u", "root", container, "bash", "-lc", `mkdir -p ${JSON.stringify(targetDir)}`],
      Math.min(timeoutMs, 60_000),
    )
    if (spec.kind === "dir") {
      await runDockerCommandForTransfer(
        root,
        `${provider}-${spec.label}-copy`,
        ["cp", `${spec.source}/.`, `${container}:${spec.target}/`],
        Math.min(timeoutMs, 180_000),
      )
    } else {
      await runDockerCommandForTransfer(
        root,
        `${provider}-${spec.label}-copy`,
        ["cp", spec.source, `${container}:${spec.target}`],
        Math.min(timeoutMs, 60_000),
      )
    }
    await runDockerCommandForTransfer(
      root,
      `${provider}-${spec.label}-chown`,
      ["exec", "-u", "root", container, "bash", "-lc", `chown -R slice:slice ${JSON.stringify(spec.target)}`],
      Math.min(timeoutMs, 60_000),
    )
    evidence.copied.push({
      label: spec.label,
      kind: spec.kind,
      target: spec.target,
    })
  }
  return evidence
}

export async function transferProviderStateFromSlice({ provider, root, sliceName, timeoutMs, providerEnv }) {
  const container = `chariox-slice-${sliceName}`
  const copyEnv = provider === "claude-p" || provider === "claude-headless" || provider === "claude"
    ? { ...providerEnv, HOME: path.join(root, "returned-claude-home") }
    : providerEnv
  const evidence = {
    provider,
    container,
    ...(copyEnv !== providerEnv ? { destination: copyEnv.HOME, destination_mode: "artifact_only" } : {}),
    copied: [],
    missing: [],
  }
  for (const spec of providerStateCopySpecs(provider, copyEnv)) {
    const targetStat = await dockerPathStat(container, spec.target, root, `${provider}-${spec.label}-reverse-stat`, timeoutMs)
    if (!targetStat.exists) {
      evidence.missing.push({
        label: spec.label,
        kind: spec.kind,
        target: spec.target,
      })
      continue
    }

    if (spec.kind === "dir") {
      await mkdir(spec.source, { recursive: true })
      await runDockerCommandForTransfer(
        root,
        `${provider}-${spec.label}-reverse-copy`,
        ["cp", `${container}:${spec.target}/.`, `${spec.source}/`],
        Math.min(timeoutMs, 180_000),
      )
    } else {
      await mkdir(path.dirname(spec.source), { recursive: true })
      await runDockerCommandForTransfer(
        root,
        `${provider}-${spec.label}-reverse-copy`,
        ["cp", `${container}:${spec.target}`, spec.source],
        Math.min(timeoutMs, 60_000),
      )
      await chmod(spec.source, 0o600).catch(() => {})
    }
    evidence.copied.push({
      label: spec.label,
      kind: spec.kind,
      source: spec.target,
    })
  }
  return evidence
}

export async function dockerPathStat(container, containerPath, root, label, timeoutMs) {
  const stdoutPath = path.join(root, `${safeLabel(label)}.stdout.log`)
  const stderrPath = path.join(root, `${safeLabel(label)}.stderr.log`)
  try {
    await runLoggedCommand("docker", ["exec", container, "bash", "-lc", `test -e ${JSON.stringify(containerPath)}`], {
      cwd: repoRoot,
      env: process.env,
      stdoutPath,
      stderrPath,
      timeoutMs: Math.min(timeoutMs, 30_000),
    })
    return { exists: true }
  } catch (error) {
    return {
      exists: false,
      error: error.message ?? String(error),
      stdoutPath,
      stderrPath,
    }
  }
}
