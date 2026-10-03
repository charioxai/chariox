#!/usr/bin/env node
// Bounded saved-slice image depth. Docker's layer store refuses images deeper
// than 125 layers (overlay2), and every capture of a slice that already runs on
// a saved image (a save or backup after a restore or clone) commits one more
// layer. A capture that would exceed MAX_CAPTURED_IMAGE_LAYERS therefore
// flattens the stopped/paused container's root filesystem into a one-layer
// image with the same configuration instead of committing on top of its parent.
// The headroom below 125 is for extension builds layered on saved images.
// Keep in sync with MAX_CAPTURED_IMAGE_LAYERS in the kernel's capture_depth.rs.
import { spawn, spawnSync } from "node:child_process"
import { fileURLToPath } from "node:url"

export const MAX_CAPTURED_IMAGE_LAYERS = 100

export function captureNeedsFlatten(parentLayerCount, maxLayers = MAX_CAPTURED_IMAGE_LAYERS) {
  if (!Number.isSafeInteger(parentLayerCount) || parentLayerCount < 1) {
    throw new Error("captured image parent depth is unavailable")
  }
  if (!Number.isSafeInteger(maxLayers) || maxLayers < 1) throw new Error("captured image depth limit is invalid")
  return parentLayerCount + 1 > maxLayers
}

function refuse(reason) { throw new Error(`flattened slice capture refused: ${reason}`) }

// Dockerfile double-quoted words: backslash, quote and $ are the only escapes.
function quoted(value) {
  if (typeof value !== "string" || /[\0\r\n]/.test(value)) refuse("configuration value is not a single line")
  return `"${value.replace(/[\\"$]/g, (character) => `\\${character}`)}"`
}

const empty = (value) => value === undefined || value === null || value === ""
  || (Array.isArray(value) && value.length === 0)
  || (typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === 0)

// `docker commit` keeps the container's configuration. `docker import` starts
// from none, so restate every field a slice image uses; refuse what it cannot hold.
export function importChanges(config) {
  if (!config || typeof config !== "object") refuse("container configuration is unavailable")
  if (!empty(config.Healthcheck)) refuse("a health check cannot be restated")
  if (!empty(config.OnBuild)) refuse("ONBUILD triggers cannot be restated")
  if (!empty(config.Shell)) refuse("a custom shell cannot be restated")
  const changes = []
  for (const entry of config.Env ?? []) {
    const separator = typeof entry === "string" ? entry.indexOf("=") : -1
    const name = separator > 0 ? entry.slice(0, separator) : ""
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) refuse("environment entry is not NAME=value")
    changes.push(`ENV ${name}=${quoted(entry.slice(separator + 1))}`)
  }
  if (!empty(config.User)) {
    if (!/^[A-Za-z0-9_.:-]+$/.test(config.User)) refuse("user is not a plain name")
    changes.push(`USER ${config.User}`)
  }
  if (!empty(config.WorkingDir)) changes.push(`WORKDIR ${quoted(config.WorkingDir)}`)
  if (!empty(config.Entrypoint)) changes.push(`ENTRYPOINT ${JSON.stringify(config.Entrypoint)}`)
  if (!empty(config.Cmd)) changes.push(`CMD ${JSON.stringify(config.Cmd)}`)
  for (const [name, value] of Object.entries(config.Labels ?? {})) {
    changes.push(`LABEL ${quoted(name)}=${quoted(value)}`)
  }
  for (const port of Object.keys(config.ExposedPorts ?? {})) {
    if (!/^[0-9]{1,5}(?:\/(?:tcp|udp|sctp))?$/.test(port)) refuse("exposed port is invalid")
    changes.push(`EXPOSE ${port}`)
  }
  const volumes = Object.keys(config.Volumes ?? {})
  if (volumes.length) changes.push(`VOLUME ${JSON.stringify(volumes)}`)
  if (!empty(config.StopSignal)) {
    if (!/^[A-Z0-9+]+$/.test(config.StopSignal)) refuse("stop signal is invalid")
    changes.push(`STOPSIGNAL ${config.StopSignal}`)
  }
  return changes.flatMap((change) => ["--change", change])
}

const RESTATED = ["Env", "User", "WorkingDir", "Entrypoint", "Cmd", "Labels", "ExposedPorts", "Volumes", "StopSignal"]

export function flattenedConfigMismatches(source, flattened) {
  const normalized = (value) => JSON.stringify(empty(value) ? null : value)
  return RESTATED.filter((field) => normalized(source?.[field]) !== normalized(flattened?.[field]))
}

function inspect(docker, env, kind, reference) {
  const result = spawnSync(docker, [kind, "inspect", reference], { env, encoding: "utf8", timeout: 60_000, maxBuffer: 8 * 1024 * 1024 })
  if (result.status !== 0) refuse(`${kind} ${reference} cannot be inspected`)
  const records = JSON.parse(result.stdout)
  if (!Array.isArray(records) || records.length !== 1) refuse(`${kind} ${reference} is ambiguous`)
  return records[0]
}

function waitForExit(child, label) {
  return new Promise((resolve, reject) => {
    child.once("error", reject)
    child.once("close", (code, signal) => code === 0 ? resolve() : reject(new Error(`${label} failed (${signal ?? code})`)))
  })
}

// Streams `docker export` into `docker import` (no temporary copy), then
// verifies that the one-layer image restates the container's configuration.
export async function flattenContainerImage({ container, image, docker = "docker", env = process.env }) {
  const source = inspect(docker, env, "container", container)
  if (source.State?.Running && !source.State?.Paused) refuse("the container must be stopped or paused")
  const changes = importChanges(source.Config)
  const exporter = spawn(docker, ["export", source.Id], { env, stdio: ["ignore", "pipe", "inherit"] })
  const importer = spawn(docker, ["import", ...changes, "-", image], { env, stdio: ["pipe", "ignore", "inherit"] })
  // A failed side closes the pipe; the exit codes below report the failure.
  exporter.stdout.on("error", () => {})
  importer.stdin.on("error", () => {})
  exporter.stdout.pipe(importer.stdin)
  const exported = waitForExit(exporter, "docker export")
  const imported = waitForExit(importer, "docker import")
  try {
    await Promise.all([exported, imported])
  } catch (error) {
    exporter.kill("SIGKILL")
    importer.kill("SIGKILL")
    await Promise.allSettled([exported, imported])
    spawnSync(docker, ["image", "rm", "-f", image], { env, stdio: "ignore", timeout: 60_000 })
    throw error
  }
  const flattened = inspect(docker, env, "image", image)
  const mismatches = flattenedConfigMismatches(source.Config, flattened.Config)
  if (mismatches.length || flattened.RootFS?.Layers?.length !== 1) {
    spawnSync(docker, ["image", "rm", "-f", image], { env, stdio: "ignore", timeout: 60_000 })
    refuse(`the imported image differs in ${mismatches.join(", ") || "layer count"}`)
  }
  return flattened
}

// The flatten CLI prints exactly one JSON line with the imported image ID.
export function flattenedImageId(stdout) {
  const lines = Buffer.from(stdout ?? "").toString("utf8").trim().split("\n")
  let reported
  try { reported = JSON.parse(lines.at(-1)).image } catch { reported = undefined }
  if (typeof reported !== "string" || !/^sha256:[a-f0-9]{64}$/.test(reported)) {
    throw new Error("flattened slice capture did not report its image")
  }
  return reported
}

// Commit, or flatten once the parent is too deep. Used by the opt-in Docker
// regression; the broker and kernel apply the same rule to their own commits.
export async function captureContainerImage({ container, image, docker = "docker", env = process.env, maxLayers = MAX_CAPTURED_IMAGE_LAYERS }) {
  const source = inspect(docker, env, "container", container)
  const parent = inspect(docker, env, "image", source.Image)
  if (captureNeedsFlatten(parent.RootFS?.Layers?.length, maxLayers)) {
    return { flattened: true, image: await flattenContainerImage({ container: source.Id, image, docker, env }) }
  }
  const committed = spawnSync(docker, ["commit", source.Id, image], { env, encoding: "utf8", timeout: 600_000, maxBuffer: 1024 * 1024 })
  if (committed.status !== 0) throw new Error(`docker commit failed: ${committed.stderr.trim()}`)
  return { flattened: false, image: inspect(docker, env, "image", image) }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const [command, container, image] = process.argv.slice(2)
  if (command !== "flatten" || !container || !image) {
    console.error("usage: captured-image-depth.mjs flatten <container> <image>")
    process.exit(2)
  }
  try {
    const flattened = await flattenContainerImage({ container, image })
    console.log(JSON.stringify({ image: flattened.Id, layers: flattened.RootFS.Layers.length }))
  } catch (error) {
    console.error(`[slice-linux] ${error.message}`)
    process.exit(1)
  }
}
