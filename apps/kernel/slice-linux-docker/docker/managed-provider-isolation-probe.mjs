#!/usr/bin/env node

import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import fs from "node:fs/promises"
import { createRequire } from "node:module"
import path from "node:path"

const require = createRequire(
  process.env.CHARIOX_PROBE_PACKAGE_JSON ?? new URL("../toolchain/package.json", import.meta.url),
)
const WebSocket = require("ws")

const kernelUrl = process.env.CHARIOX_KERNEL_URL ?? "ws://127.0.0.1:43119"
const authToken = process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN
const workspace = process.env.CHARIOX_MANAGED_ISOLATION_PROBE_WORKSPACE ?? "/workspace"
const worktree = process.env.CHARIOX_MANAGED_ISOLATION_PROBE_WORKTREE ?? workspace
const resultPath = process.env.CHARIOX_MANAGED_ISOLATION_PROBE_RESULT ??
  path.join(workspace, ".chariox-managed-isolation-probe.result")
const launchCapturePath = process.env.CHARIOX_MANAGED_ISOLATION_PROBE_CAPTURE ?? ""
const accountProfile = process.env.CHARIOX_MANAGED_ISOLATION_PROBE_ACCOUNT ?? "default"
const model = process.env.CHARIOX_MANAGED_ISOLATION_PROBE_MODEL ?? "gpt-5.4"
const nativeTui = /^(?:1|true|yes|on)$/i.test(
  process.env.CHARIOX_MANAGED_ISOLATION_PROBE_NATIVE_TUI ?? "",
)
const requireNestedUsernsDenied = /^(?:1|true|yes|on)$/i.test(
  process.env.CHARIOX_MANAGED_ISOLATION_REQUIRE_NESTED_USERNS_DENIED ?? "",
)
const timeoutMs = Number(process.env.CHARIOX_PROBE_TIMEOUT_MS ?? 45_000)

assert.ok(authToken, "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN is required")
assert.ok(path.isAbsolute(workspace), "probe workspace must be absolute")
assert.ok(path.isAbsolute(resultPath), "probe result path must be absolute")
assert.ok(
  !launchCapturePath || path.isAbsolute(launchCapturePath),
  "probe launch capture path must be absolute",
)
assert.ok(Number.isSafeInteger(timeoutMs) && timeoutMs >= 1_000, "probe timeout must be at least one second")

const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds))

function withTimeout(promise, label, milliseconds = timeoutMs) {
  let timer
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} timed out after ${milliseconds}ms`)), milliseconds)
    }),
  ]).finally(() => clearTimeout(timer))
}

async function connectKernel() {
  const deadline = Date.now() + timeoutMs
  let lastError
  while (Date.now() < deadline) {
    const socket = new WebSocket(kernelUrl, {
      headers: { Authorization: `Bearer ${authToken}` },
    })
    try {
      await withTimeout(
        new Promise((resolve, reject) => {
          socket.once("open", resolve)
          socket.once("error", reject)
          socket.once("unexpected-response", (_request, response) => {
            reject(new Error(`kernel rejected WebSocket authentication with HTTP ${response.statusCode}`))
          })
        }),
        "authenticated kernel WebSocket connection",
        Math.min(2_000, Math.max(deadline - Date.now(), 1)),
      )
      return socket
    } catch (error) {
      lastError = error
      socket.terminate()
      await sleep(250)
    }
  }
  throw new Error(`authenticated kernel WebSocket connection failed: ${lastError?.message ?? lastError}`)
}

async function readProbeResult() {
  const deadline = Date.now() + timeoutMs
  let lastError
  while (Date.now() < deadline) {
    try {
      return await fs.readFile(resultPath, "utf8")
    } catch (error) {
      lastError = error
      if (error?.code !== "ENOENT") throw error
      await sleep(100)
    }
  }
  throw new Error(`sandbox probe result was not written: ${lastError?.message ?? lastError}`)
}

const socket = await connectKernel()
const requestNamespace = randomUUID()
let sequence = 0
const pending = new Map()
socket.on("message", (payload) => {
  let frame
  try {
    frame = JSON.parse(String(payload))
  } catch {
    return
  }
  if (frame.type !== "response") return
  const deferred = pending.get(frame.request_id)
  if (!deferred) return
  pending.delete(frame.request_id)
  if (frame.error) {
    deferred.reject(new Error(`${frame.error.code ?? "kernel_error"}: ${frame.error.message ?? "request failed"}`))
  } else {
    deferred.resolve(frame.response)
  }
})
socket.on("close", () => {
  for (const deferred of pending.values()) deferred.reject(new Error("kernel WebSocket closed"))
  pending.clear()
})

async function send(request, label) {
  const requestId = `managed-provider-isolation-${requestNamespace}-${++sequence}`
  const response = new Promise((resolve, reject) => pending.set(requestId, { resolve, reject }))
  socket.send(JSON.stringify({
    type: "request",
    request_id: requestId,
    command_id: requestId,
    request,
  }))
  return withTimeout(response, label)
}

async function waitForProviderRunRunning(providerRunId) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const response = await send({
      GetProviderRun: { provider_run_id: providerRunId },
    }, "probe provider run readiness")
    const providerRun = response?.ProviderRun?.provider_run
    assert.ok(providerRun, `kernel did not return ProviderRun for ${providerRunId}`)
    const state = String(providerRun.state ?? "").toLowerCase()
    if (state === "running") return
    if (["failed", "ended"].includes(state)) {
      const diagnostic = sanitizeProbeDiagnostic(
        providerRun.terminal_diagnostic ?? providerRun.terminalDiagnostic,
      )
      const suffix = diagnostic ? `terminal_diagnostic=${diagnostic}` : ""
      const renderedSuffix = suffix ? `; ${suffix}` : ""
      throw new Error(`isolation probe provider run entered ${providerRun.state}${renderedSuffix}`)
    }
    await sleep(100)
  }
  throw new Error(`isolation probe provider run ${providerRunId} did not become ready`)
}

let sessionId
let probeSucceeded = false
try {
  await fs.rm(resultPath, { force: true })
  const created = await send({
    CreateSession: {
      workspace_id: workspace,
      worktree_id: worktree,
    },
  }, "probe session creation")
  sessionId = created?.SessionCreated?.session?.id
  assert.ok(sessionId, `kernel did not return SessionCreated: ${JSON.stringify(created)}`)

  const launched = await send({
    LaunchProviderRun: {
      session_id: sessionId,
      agent_id: null,
      adapter_key: "codex",
      provider: "codex",
      account_profile: accountProfile,
      model,
      variant: null,
      structured_endpoint: null,
      provider_session_id: null,
      native_tui: nativeTui,
    },
  }, "real Codex provider launch")
  assert.ok(
    launched?.ProviderRunLaunched?.provider_run ||
      launched?.ProviderRunLaunchAccepted?.provider_run,
    `kernel did not launch the real Codex provider: ${JSON.stringify(launched)}`,
  )
  const providerRun = launched.ProviderRunLaunched?.provider_run ??
    launched.ProviderRunLaunchAccepted.provider_run

  await writeLaunchCapture(providerRun)

  await waitForProviderRunRunning(providerRun.id)
  const result = await readProbeResult()
  assert.match(result, /^managed_provider_isolation=ok$/m)
  assert.match(result, /^real_provider=\//m)
  assert.match(result, new RegExp(`^workspace=${escapeRegExp(workspace)}$`, "m"))
  if (requireNestedUsernsDenied) {
    assert.match(result, /^nested_userns=denied$/m)
  }
  assert.match(result, /^outside_repository=\/tmp\//m)
  assert.match(result, /^outside_clone=\/tmp\//m)
  probeSucceeded = true
  process.stdout.write(`${JSON.stringify({
    authenticated: true,
    provider: "codex",
    accountProfile,
    workspace,
    denied: [
      "kernel state and vault",
      "Docker broker",
      "host process roots",
    ],
    })}\n`)
} catch (error) {
  // The wrapper can fail before Codex binds its endpoint. Keep its private
  // report instead of deleting the only explanation behind a readiness timeout.
  // Print only the path, never untrusted report contents or provider credentials.
  if (await fs.lstat(resultPath).then((stat) => stat.isFile(), () => false)) {
    process.stderr.write(`wrapper_result=${JSON.stringify(resultPath)}\n`)
  }
  throw error
} finally {
  if (sessionId) {
    await send({ EndSession: { session_id: sessionId } }, "probe session cleanup").catch(() => {})
  }
  if (probeSucceeded) {
    await fs.rm(resultPath, { force: true }).catch(() => {})
  }
  socket.close()
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}

async function writeLaunchCapture(providerRun) {
  if (!launchCapturePath) return

  const args = providerRun.pty_args ?? providerRun.ptyArgs ?? []
  const environment = providerRun.pty_env ?? providerRun.ptyEnv ?? {}
  const accountEnvironmentNames = new Set([
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
    "OPENCODE_CONFIG_DIR",
  ])
  const accountEnvironment = Object.fromEntries(
    Object.entries(environment)
      .filter(([name]) => accountEnvironmentNames.has(name))
      .map(([name, value]) => [name, String(value)]),
  )

  const capture = {
    schema: 1,
    provider_run_id: providerRun.id,
    pty_program: providerRun.pty_program ?? providerRun.ptyProgram ?? null,
    pty_args: sanitizeLaunchArgs(args),
    pty_env_account_paths: accountEnvironment,
    namespace_account_paths: extractSetenvAccountPaths(args),
    pty_env_remove: providerRun.pty_env_remove ?? providerRun.ptyEnvRemove ?? [],
    working_directory: providerRun.working_directory ?? providerRun.workingDirectory ?? null,
    structured_endpoint: providerRun.structured_endpoint ?? providerRun.structuredEndpoint ?? null,
  }
  await fs.writeFile(launchCapturePath, `${JSON.stringify(capture, null, 2)}\n`, { mode: 0o600 })
  await fs.chmod(launchCapturePath, 0o600)
}

function sanitizeLaunchArgs(args) {
  const sanitized = []
  for (let index = 0; index < args.length; index += 1) {
    const value = String(args[index])
    const previous = String(args[index - 1] ?? "")
    if (previous === "--setenv" && isSensitiveEnvironmentName(value)) {
      sanitized.push(value)
      if (index + 1 < args.length) {
        sanitized.push("[redacted]")
        index += 1
      }
      continue
    }
    sanitized.push(looksLikeSecret(value) ? "[redacted]" : value)
  }
  return sanitized
}

function isSensitiveEnvironmentName(value) {
  return /(?:TOKEN|SECRET|PASSWORD|CREDENTIAL|AUTH|COOKIE|KEY)/i.test(value)
}

function extractSetenvAccountPaths(args) {
  const names = new Set([
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
    "OPENCODE_CONFIG_DIR",
  ])
  const paths = {}
  for (let index = 0; index + 2 < args.length; index += 1) {
    if (args[index] === "--setenv" && names.has(String(args[index + 1]))) {
      paths[String(args[index + 1])] = String(args[index + 2])
    }
  }
  return paths
}

function looksLikeSecret(value) {
  return /^(?:sk|sess|key|tok|secret|bearer)[-_][A-Za-z0-9._-]{10,}$/i.test(value) ||
    /^(?:Bearer|Basic)\s+\S+$/i.test(value)
}

function sanitizeProbeDiagnostic(value) {
  if (value === undefined || value === null) return ""
  let diagnostic = String(value)
    .replace(/[\u0000-\u001f\u007f]/g, " ")
    .replace(/\s+/g, " ")
  diagnostic = diagnostic.replace(
    /\b(authorization|api[_-]?key|token|secret|password|credential(?:s)?|cookie|prompt|stderr|stdout)\s*[:=]\s*.*?(?=\s+\b[\w.-]+\s*[:=]|$)/gi,
    "$1=[redacted]",
  )
  diagnostic = diagnostic.replace(/\b(Bearer|Basic)\s+\S+/gi, "$1 [redacted]")
  return diagnostic.slice(0, 1_024)
}
