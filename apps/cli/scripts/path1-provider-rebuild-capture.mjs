#!/usr/bin/env node

// Read-only Hetzner observations for the Path-1 rebuild evidence set.
import { open, lstat, realpath, unlink } from "node:fs/promises"
import { basename, dirname, isAbsolute, join, parse, relative, resolve, sep } from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

export const PROVIDER_CAPTURE_SCHEMA = "chariox.path1-provider-rebuild-capture/v1"

const API_BASE_URL = "https://api.hetzner.cloud/v1"
const API_ORIGIN = new URL(API_BASE_URL).origin
const DEFAULT_TIMEOUT_MS = 10_000
const MAX_RESPONSE_BYTES = 1024 * 1024
const PAGE_SIZE = 50
const MAX_ACTION_PAGES = 20
const REPOSITORY_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../../..")

function requireValue(condition, message) {
  if (!condition) throw new Error(message)
}

function inputId(value, label) {
  requireValue(typeof value === "string" && /^[1-9][0-9]{0,15}$/.test(value)
    && Number.isSafeInteger(Number(value)), `${label} is invalid`)
  return value
}

function providerId(value, label) {
  requireValue(Number.isSafeInteger(value) && value > 0, `${label} is missing`)
  return String(value)
}

function canonicalRequestedAt(value) {
  requireValue(typeof value === "string" && /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/.test(value)
    && Number.isFinite(Date.parse(value)) && new Date(value).toISOString() === value,
  "requestedAt must be a canonical UTC timestamp")
  return { value, milliseconds: Date.parse(value) }
}

function providerTimestamp(value, label) {
  const match = typeof value === "string"
    ? /^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.(\d{1,9}))?Z$/.exec(value)
    : null
  requireValue(match, `${label} is missing or invalid`)
  const millis = (match[2] ?? "").slice(0, 3).padEnd(3, "0")
  const normalized = `${match[1]}.${millis}Z`
  const milliseconds = Date.parse(normalized)
  requireValue(Number.isFinite(milliseconds) && new Date(milliseconds).toISOString() === normalized,
    `${label} is missing or invalid`)
  return { value, milliseconds }
}

function captureTime(now) {
  const value = now()
  requireValue(value instanceof Date && Number.isFinite(value.getTime()), "capture clock is invalid")
  return value.toISOString()
}

function requestUrl(pathAndQuery) {
  const url = new URL(`${API_BASE_URL}${pathAndQuery}`)
  requireValue(url.origin === API_ORIGIN && url.pathname.startsWith("/v1/"),
    "provider request origin is invalid")
  return url.toString()
}

async function boundedBody(response, maximumBytes) {
  const length = response.headers?.get?.("content-length")
  if (length !== null && length !== undefined) {
    requireValue(/^\d+$/.test(length) && Number(length) <= maximumBytes,
      "provider response exceeds the size limit")
  }
  const reader = response.body?.getReader?.()
  requireValue(reader, "provider response body is missing")
  const chunks = []
  let size = 0
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    size += value.byteLength
    if (size > maximumBytes) {
      void reader.cancel().catch(() => {})
      throw new Error("provider response exceeds the size limit")
    }
    chunks.push(Buffer.from(value))
  }
  return Buffer.concat(chunks, size)
}

async function getJson(pathAndQuery, token, {
  fetchImpl = globalThis.fetch,
  timeoutMs = DEFAULT_TIMEOUT_MS,
  maximumBytes = MAX_RESPONSE_BYTES,
} = {}) {
  requireValue(typeof token === "string" && token.length > 0 && !/[\r\n\0]/.test(token),
    "an explicit Hetzner token is required")
  requireValue(typeof fetchImpl === "function", "fetch is unavailable")
  requireValue(Number.isInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= 60_000,
    "request timeout is invalid")
  requireValue(Number.isInteger(maximumBytes) && maximumBytes > 0 && maximumBytes <= MAX_RESPONSE_BYTES,
    "response size limit is invalid")

  const controller = new AbortController()
  let timeout
  const timedOut = new Promise((_, reject) => {
    timeout = setTimeout(() => {
      controller.abort()
      reject(new Error("provider request timed out"))
    }, timeoutMs)
  })
  const request = (async () => {
    let response
    try {
      response = await fetchImpl(requestUrl(pathAndQuery), {
        method: "GET",
        headers: { authorization: `Bearer ${token}`, accept: "application/json" },
        redirect: "error",
        signal: controller.signal,
      })
    } catch {
      throw new Error("provider request failed")
    }
    requireValue(response?.status === 200 && response.ok === true,
      "provider returned an unsuccessful response")
    const bytes = await boundedBody(response, maximumBytes)
    let value
    try {
      value = JSON.parse(bytes.toString("utf8"))
    } catch {
      throw new Error("provider response is not valid JSON")
    }
    requireValue(value && typeof value === "object" && !Array.isArray(value),
      "provider response has an invalid shape")
    return value
  })()
  try {
    return await Promise.race([request, timedOut])
  } finally {
    clearTimeout(timeout)
  }
}

function serverIdentity(envelope, expectedServerId) {
  const server = envelope?.server
  requireValue(server && typeof server === "object" && !Array.isArray(server),
    "provider server response is incomplete")
  const identity = {
    serverId: providerId(server.id, "server id"),
    imageId: providerId(server.image?.id, "server image id"),
    serverTypeId: providerId(server.server_type?.id, "server type id"),
    locationId: providerId(server.location?.id, "server location id"),
    datacenterId: providerId(server.datacenter?.id, "server datacenter id"),
  }
  requireValue(identity.serverId === expectedServerId, "provider returned the wrong server")
  return identity
}

async function readServer(serverId, token, options) {
  return serverIdentity(await getJson(`/servers/${serverId}`, token, options), serverId)
}

function validatePagination(envelope, requestedPage) {
  const pagination = envelope?.meta?.pagination
  requireValue(pagination && pagination.page === requestedPage && pagination.per_page === PAGE_SIZE
    && Number.isInteger(pagination.last_page) && pagination.last_page >= requestedPage
    && pagination.last_page <= MAX_ACTION_PAGES
    && pagination.previous_page === (requestedPage === 1 ? null : requestedPage - 1)
    && pagination.next_page === (requestedPage < pagination.last_page ? requestedPage + 1 : null),
  "provider action history pagination is incomplete or outside the limit")
  return pagination.last_page
}

async function readActionHistory(serverId, token, options) {
  const ids = new Set()
  let target
  let lastPage
  for (let page = 1; page <= (lastPage ?? MAX_ACTION_PAGES); page += 1) {
    const envelope = await getJson(`/servers/${serverId}/actions?page=${page}&per_page=${PAGE_SIZE}`, token, options)
    const pageLast = validatePagination(envelope, page)
    if (lastPage !== undefined) requireValue(pageLast === lastPage, "provider action history changed during pagination")
    lastPage = pageLast
    const actions = envelope.actions
    requireValue(Array.isArray(actions) && actions.length <= PAGE_SIZE,
      "provider action history has an invalid shape")
    for (const action of actions) {
      const id = providerId(action?.id, "action id")
      requireValue(!ids.has(id), "provider action history contains a duplicate action")
      ids.add(id)
      if (id === options.actionId) target = action
    }
  }
  requireValue(target, "the requested provider action was not found")
  return target
}

function selectRebuildAction(action, serverId, actionId, requestedAt, capturedAt) {
  requireValue(providerId(action.id, "action id") === actionId,
    "provider returned the wrong action")
  requireValue(Array.isArray(action.resources) && action.resources.length === 1
    && action.resources[0]?.type === "server"
    && providerId(action.resources[0]?.id, "action server id") === serverId,
  "provider action is not bound to the requested server")
  requireValue(action.command === "rebuild", "provider action is not a rebuild")
  if (action.status === "error") throw new Error("provider rebuild action failed")
  requireValue(action.status === "success", "provider rebuild action is incomplete")
  requireValue(Object.hasOwn(action, "error") && action.error === null,
    "provider rebuild action has an error")
  const started = providerTimestamp(action.started, "action start time")
  const finished = providerTimestamp(action.finished, "action finish time")
  requireValue(started.milliseconds >= requestedAt.milliseconds,
    "provider rebuild action predates requestedAt")
  requireValue(finished.milliseconds >= started.milliseconds,
    "provider rebuild action times are inconsistent")
  requireValue(finished.milliseconds <= Date.parse(capturedAt),
    "provider rebuild action finishes after capture time")
  return {
    actionId,
    command: action.command,
    status: action.status,
    error: action.error,
    started: started.value,
    finished: finished.value,
  }
}

export async function captureProviderBefore({ serverId, token, fetchImpl, now = () => new Date(), timeoutMs }) {
  const id = inputId(serverId, "server id")
  const options = { fetchImpl, timeoutMs }
  const server = await readServer(id, token, options)
  return {
    schema: PROVIDER_CAPTURE_SCHEMA,
    provider: "hetzner-cloud",
    kind: "before",
    capturedAt: captureTime(now),
    server,
  }
}

export async function captureProviderAfter({
  serverId, imageId, actionId, requestedAt: requestedAtValue, token,
  fetchImpl, now = () => new Date(), timeoutMs,
}) {
  const id = inputId(serverId, "server id")
  const expectedImageId = inputId(imageId, "image id")
  const expectedActionId = inputId(actionId, "action id")
  const requestedAt = canonicalRequestedAt(requestedAtValue)
  const options = { fetchImpl, timeoutMs, actionId: expectedActionId }
  const serverBefore = await readServer(id, token, options)
  requireValue(serverBefore.imageId === expectedImageId,
    "provider server has the wrong image before action history read")
  const actionValue = await readActionHistory(id, token, options)
  const serverAfter = await readServer(id, token, options)
  const capturedAt = captureTime(now)
  requireValue(Date.parse(capturedAt) >= requestedAt.milliseconds,
    "requestedAt is after capture time")
  const action = selectRebuildAction(actionValue, id, expectedActionId, requestedAt, capturedAt)
  requireValue(serverAfter.imageId === expectedImageId
    && Object.keys(serverBefore).every((field) => serverBefore[field] === serverAfter[field]),
  "provider server identity or image changed during action history read")
  return {
    schema: PROVIDER_CAPTURE_SCHEMA,
    provider: "hetzner-cloud",
    kind: "after",
    capturedAt,
    requestedAt: requestedAt.value,
    serverBefore,
    action,
    serverAfter,
  }
}

function within(root, target) {
  const path = relative(root, target)
  return path === "" || (path !== ".." && !path.startsWith(`..${sep}`) && !isAbsolute(path))
}

async function assertNoSymlinkPath(directory) {
  const absolute = resolve(directory)
  const root = parse(absolute).root
  let current = root
  for (const component of absolute.slice(root.length).split(sep).filter(Boolean)) {
    current = join(current, component)
    let metadata
    try {
      metadata = await lstat(current)
    } catch {
      throw new Error("output parent path is unavailable")
    }
    requireValue(!metadata.isSymbolicLink(), "output path contains a symlink")
    requireValue(metadata.isDirectory(), "output parent path is not a directory")
  }
  return absolute
}

export async function validateCaptureOutput(file, repositoryRoot = REPOSITORY_ROOT) {
  requireValue(typeof file === "string" && isAbsolute(file), "output path must be absolute")
  const output = resolve(file)
  const parent = await assertNoSymlinkPath(dirname(output))
  const canonicalParent = await realpath(parent)
  const canonicalRoot = await realpath(repositoryRoot)
  requireValue(!within(canonicalRoot, join(canonicalParent, basename(output))),
    "output must be outside the repository")
  try {
    await lstat(output)
    throw new Error("output must be a new file")
  } catch (error) {
    if (error?.message === "output must be a new file") throw error
    if (error?.code !== "ENOENT") throw new Error("output cannot be inspected")
  }
  return output
}

export async function writeCaptureOutput(file, capture, repositoryRoot = REPOSITORY_ROOT) {
  const output = await validateCaptureOutput(file, repositoryRoot)
  const handle = await open(output, "wx", 0o600)
  try {
    await handle.writeFile(`${JSON.stringify(capture, null, 2)}\n`, "utf8")
    await handle.chmod(0o600)
  } catch {
    await handle.close().catch(() => {})
    await unlink(output).catch(() => {})
    throw new Error("capture evidence could not be written")
  }
  await handle.close()
  return output
}

function parseArguments(argv) {
  const [kind, ...values] = argv
  const allowed = kind === "before"
    ? ["--server-id", "--output"]
    : kind === "after"
      ? ["--server-id", "--image-id", "--action-id", "--requested-at", "--output"]
      : []
  requireValue(allowed.length > 0 && values.length === allowed.length * 2,
    "usage: [before|after] with the required options")
  const args = new Map()
  for (let index = 0; index < values.length; index += 2) {
    requireValue(allowed.includes(values[index]) && !args.has(values[index])
      && typeof values[index + 1] === "string" && values[index + 1].length > 0,
    "invalid provider capture arguments")
    args.set(values[index], values[index + 1])
  }
  requireValue(allowed.every((flag) => args.has(flag)), "provider capture arguments are incomplete")
  return { kind, args }
}

async function main() {
  const { kind, args } = parseArguments(process.argv.slice(2))
  const output = args.get("--output")
  await validateCaptureOutput(output)
  const token = process.env.HETZNER_TOKEN
  requireValue(typeof token === "string" && token.length > 0 && !/[\r\n\0]/.test(token),
    "set HETZNER_TOKEN explicitly on the authorized manager")
  const common = { serverId: args.get("--server-id"), token }
  const capture = kind === "before"
    ? await captureProviderBefore(common)
    : await captureProviderAfter({
      ...common,
      imageId: args.get("--image-id"),
      actionId: args.get("--action-id"),
      requestedAt: args.get("--requested-at"),
    })
  await writeCaptureOutput(output, capture)
  process.stdout.write(`Captured Hetzner ${kind} observations.\n`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(() => {
    process.stderr.write("Hetzner provider capture failed; no acceptance verdict.\n")
    process.exitCode = 1
  })
}
