import { chmod, copyFile, cp, mkdir, stat } from "node:fs/promises"
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
  const container = `arroba-slice-${sliceName}`
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
  const container = `arroba-slice-${sliceName}`
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
