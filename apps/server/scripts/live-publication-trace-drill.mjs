#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, writeFile } from "node:fs/promises"
import { createServer } from "node:http"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import { WebSocket } from "ws"
import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import {
  applyWorkflowCodeRequest,
  controlWorkflowPublicationRuntimeRequest,
  createSessionRequest,
  createWorkflowPublicationRequest,
  getProviderAuthStatusRequest,
  getProviderCatalogRequest,
  getWorkflowRunRequest,
} from "@arroba/kernel-client/ipc-requests"

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..")
const kernelUrl = process.env.ARROBA_KERNEL_URL ?? "ws://127.0.0.1:44120/kernel"
const workflowCodeNodePath = process.env.ARROBA_WORKFLOW_CODE_NODE ?? process.execPath
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\..+/, "").replace("T", "-")
const artifactsDir = resolve(process.env.ARROBA_TRACE_DRILL_ARTIFACTS_DIR ?? join(repoRoot, ".artifacts", `live-publication-traces-${stamp}`))
const preflightOnly = process.argv.includes("--preflight-only")
const continueOnPreflightFailure = process.argv.includes("--continue-on-preflight-failure")
const requestedProviders = csvArg("--providers")
const requestedTransports = csvArg("--transports")
const requestedPolicies = csvArg("--policies")
const activeRuntimeStops = []
const runtimeStartTimeoutMs = Number(process.env.ARROBA_TRACE_DRILL_RUNTIME_START_TIMEOUT_MS ?? 90_000)
const protocolTimeoutMs = Number(process.env.ARROBA_TRACE_DRILL_PROTOCOL_TIMEOUT_MS ?? 300_000)
const providerSpecs = [
  {
    provider: "opencode",
    requestedModel: "Kimi K2.6",
    acceptableModelIds: ["kimi-k2.6"],
    authMode: "cli_catalog",
  },
  {
    provider: "claude-headless",
    requestedModel: "Sonnet 3.6",
    acceptableModelIds: ["claude-sonnet-3-6", "claude-sonnet-3.6"],
    authMode: "kernel",
  },
  {
    provider: "codex",
    requestedModel: "gpt-5.5",
    acceptableModelIds: ["gpt-5.5"],
    authMode: "kernel",
  },
]
const transports = [
  { id: "human_http", route: "/prompt/*", methods: ["GET", "POST"], transport: { kind: "human_http" }, parser: { kind: "path_template", template: "/prompt/:prompt" }, mode: "sync" },
  { id: "api_sse_json", route: "/invoke", methods: ["POST"], transport: { kind: "api_sse_json" }, parser: { kind: "json_body" }, mode: "async" },
  { id: "websocket_json", route: "/socket", methods: ["GET"], transport: { kind: "websocket_json" }, parser: null, mode: "async" },
  { id: "mcp", route: "/mcp", methods: ["POST"], transport: { kind: "mcp" }, parser: null, mode: "sync" },
  { id: "schedule_only", route: null, methods: [], transport: { kind: "schedule_only" }, parser: null, mode: "async", kind: "schedule_only" },
]
const policies = [
  { id: "none", levels: null },
  { id: "output_summary", levels: ["output_summary"] },
  { id: "assistant_messages", levels: ["assistant_messages"] },
  { id: "thinking", levels: ["thinking"] },
  { id: "tool_use", levels: ["tool_use"] },
  { id: "all", levels: ["output_summary", "assistant_messages", "thinking", "tool_use"] },
  { id: "mixed", mixed: true },
]
const markerNames = {
  output_summary: "TRACE_SUMMARY",
  assistant_messages: "TRACE_ASSISTANT",
  thinking: "TRACE_THINKING",
  tool_use: "TRACE_TOOL",
  final: "TRACE_FINAL",
}

await mkdir(artifactsDir, { recursive: true })
const client = new LocalIpcClient(kernelUrl)
const report = {
  generated_at: new Date().toISOString(),
  kernel_url: kernelUrl,
  workflow_code_node_path: workflowCodeNodePath,
  artifacts_dir: artifactsDir,
  preflight: null,
  matrix: [],
  screenshots: {},
}

try {
  report.preflight = await preflight(client)
  await writeArtifact("preflight.json", report.preflight)
  if (preflightOnly) {
    await writeReport()
    process.exit(report.preflight.ok ? 0 : 2)
  }
  if (!report.preflight.ok && !continueOnPreflightFailure) {
    await writeReport()
    console.error(`live publication trace drill preflight failed; see ${join(artifactsDir, "report.json")}`)
    process.exit(2)
  }
  for (const provider of report.preflight.providers.filter((entry) => entry.ok && selected(requestedProviders, entry.provider))) {
    await runProviderMatrix(client, provider)
  }
  await writeReport()
  const failures = report.matrix.filter((entry) => entry.status !== "pass")
  process.exit(failures.length === 0 ? 0 : 1)
} finally {
  await stopActiveRuntimes(client)
  await client.close().catch(() => {})
}

async function preflight(client) {
  const cli = {
    codex: await commandVersion("codex", ["--version"]),
    claude: await commandVersion("claude", ["--version"]),
    opencode: await commandVersion("opencode", ["--version"]),
  }
  const catalogResponse = await client.send(getProviderCatalogRequest())
  const catalog = catalogResponse?.ProviderCatalog?.catalog
  const providers = []
  for (const spec of providerSpecs) {
    const providerCatalog = catalog?.all?.find((provider) => provider.id === spec.provider) ?? null
    const model = resolveRequestedModel(providerCatalog, spec)
    const auth = await providerAuth(client, spec)
    providers.push({
      provider: spec.provider,
      requested_model: spec.requestedModel,
      resolved_model: model?.id ?? null,
      resolved_model_name: model?.name ?? null,
      model_status: model?.status ?? null,
      catalog_provider_present: Boolean(providerCatalog),
      auth,
      ok: Boolean(providerCatalog && model && auth.ok),
      unavailable_reason: providerCatalog
        ? model
          ? auth.ok ? null : auth.reason
          : `requested model ${spec.requestedModel} was not found in provider catalog`
        : "provider was not found in provider catalog",
      available_models: providerCatalog ? Object.values(providerCatalog.models ?? {}).map((model) => ({ id: model.id, name: model.name, status: model.status })) : [],
    })
  }
  return {
    ok: providers.every((provider) => provider.ok),
    cli,
    providers,
  }
}

async function providerAuth(client, spec) {
  if (spec.authMode === "cli_catalog") {
    return { ok: true, mode: "cli_catalog", reason: null }
  }
  try {
    const response = await client.send(getProviderAuthStatusRequest(spec.provider))
    const status = response?.ProviderAuthStatus?.status
    const ok = status?.auth_state === "authenticated"
    return { ok, mode: "kernel", status, reason: ok ? null : `auth_state=${status?.auth_state ?? "unknown"}` }
  } catch (error) {
    return { ok: false, mode: "kernel", reason: error instanceof Error ? error.message : String(error) }
  }
}

function resolveRequestedModel(providerCatalog, spec) {
  const models = Object.values(providerCatalog?.models ?? {})
  return models.find((model) => spec.acceptableModelIds.includes(model.id))
    ?? models.find((model) => normalizeModelToken(model.name) === normalizeModelToken(spec.requestedModel))
    ?? null
}

function normalizeModelToken(value) {
  return String(value ?? "").toLowerCase().replace(/[^a-z0-9]+/g, "")
}

async function runProviderMatrix(client, provider) {
  const sessionAlias = `trace-${provider.provider}-${Date.now()}`
  const session = await unwrap(
    client.send(createSessionRequest(`workspace-${sessionAlias}`, `worktree-${sessionAlias}`, sessionAlias)),
    "SessionCreated",
  )
  const sessionId = session.session.id
  const workflowSource = workflowCodeSource(provider)
  const applied = await unwrap(
    client.send(applyWorkflowCodeRequest(sessionId, workflowCodeNodePath, workflowSource)),
    "WorkflowCodeApplied",
  )
  await writeArtifact(`${provider.provider}/workflow-applied.json`, applied)
  const workflowId = applied.result.apply.workflow_id
  const endpointId = applied.result.apply.endpoint_ids.entry
  const nodeIds = applied.result.apply.node_ids

  for (const policy of policies.filter((policy) => selected(requestedPolicies, policy.id))) {
    for (const transport of transports.filter((transport) => selected(requestedTransports, transport.id))) {
      const matrixEntry = {
        provider: provider.provider,
        model: provider.resolved_model,
        transport: transport.id,
        policy: policy.id,
        status: "pending",
        thinking_emitted: null,
        tool_use_emitted: null,
        artifacts: {},
        error: null,
      }
      report.matrix.push(matrixEntry)
      const prefix = `${provider.provider}/${policy.id}/${transport.id}`
      try {
        const publication = await createPublication(client, sessionId, workflowId, endpointId, nodeIds, policy, transport, prefix)
        await writeArtifact(`${prefix}/publication.json`, publication)
        const runtime = await startPublicationRuntime(client, sessionId, publication.id, transport)
        activeRuntimeStops.push({ sessionId, publicationId: publication.id })
        await writeArtifact(`${prefix}/runtime.json`, runtime)
        const raw = await invokeTransport(runtime.local_url ?? runtime.open_url ?? runtime.viewer_url, transport, provider, policy)
        await writeArtifact(`${prefix}/raw-response.json`, raw)
        const statusPayload = runtime.local_url ? await waitForPublicationStatus(runtime.local_url, transport) : null
        await writeArtifact(`${prefix}/publication-status.json`, statusPayload)
        const latestRunId = statusPayload?.latest_run?.id ?? raw.workflow_run_id ?? null
        const workflowRunSessionId = statusPayload?.runtime_session_id ?? sessionId
        const workflowRun = latestRunId ? await workflowRunSnapshot(client, workflowRunSessionId, latestRunId) : null
        await writeArtifact(`${prefix}/workflow-run.json`, workflowRun)
        const assertions = assertExposure({
          visible: JSON.stringify([raw, statusPayload]),
          workflowRun,
          policy,
          nodeIds,
        })
        await writeArtifact(`${prefix}/assertions.json`, assertions)
        matrixEntry.thinking_emitted = assertions.provider_emitted.thinking
        matrixEntry.tool_use_emitted = assertions.provider_emitted.tool_use
        matrixEntry.artifacts = {
          publication: `${prefix}/publication.json`,
          raw_response: `${prefix}/raw-response.json`,
          status: `${prefix}/publication-status.json`,
          workflow_run: `${prefix}/workflow-run.json`,
          assertions: `${prefix}/assertions.json`,
        }
        matrixEntry.status = assertions.ok ? "pass" : "fail"
        if (!assertions.ok) matrixEntry.error = assertions.failures.join("; ")
      } catch (error) {
        matrixEntry.status = "fail"
        matrixEntry.error = error instanceof Error ? error.stack ?? error.message : String(error)
        await writeArtifact(`${prefix}/error.txt`, matrixEntry.error)
      } finally {
        await stopRuntime(client, sessionId, matrixEntry, prefix)
      }
    }
  }
}

async function createPublication(client, sessionId, workflowId, endpointId, nodeIds, policy, transport, prefix) {
  const traceExposure = traceExposureForPolicy(policy, nodeIds)
  const response = await unwrap(
    client.send(createWorkflowPublicationRequest(sessionId, workflowId, endpointId, {
      alias: `${prefix}`.replaceAll("/", "-").slice(0, 60),
      queueRef: "default",
      kind: transport.kind ?? "ingress",
      route: transport.route,
      methods: transport.methods,
      transport: transport.transport,
      parser: transport.parser,
      traceExposure,
      mode: transport.mode,
      syncTimeoutMs: 240_000,
      pollMs: 500,
    })),
    "WorkflowPublicationCreated",
  )
  return response.publication
}

function traceExposureForPolicy(policy, nodeIds) {
  if (policy.levels == null && !policy.mixed) return null
  if (policy.mixed) {
    return {
      nodes: {
        [nodeIds.planner]: ["output_summary", "assistant_messages", "thinking", "tool_use"],
        [nodeIds.worker]: ["output_summary"],
        [nodeIds.finalizer]: [],
      },
    }
  }
  return {
    nodes: Object.fromEntries(Object.values(nodeIds).map((nodeId) => [nodeId, policy.levels])),
  }
}

async function startPublicationRuntime(client, sessionId, publicationId, transport) {
  const port = await freePort()
  const response = await unwrap(
    withTimeout(client.send(controlWorkflowPublicationRuntimeRequest(sessionId, publicationId, "start", {
      host: "127.0.0.1",
      port,
      kernelUrl,
    })), runtimeStartTimeoutMs, `starting publication runtime ${publicationId}`),
    "WorkflowPublicationRuntimeControlled",
  )
  if (transport.id !== "schedule_only" && !response.local_url) throw new Error("publication runtime did not return a local_url")
  if (response.local_url) await waitForGatewayReady(response.local_url)
  return response
}

async function waitForGatewayReady(localUrl) {
  const statusUrl = new URL("/.well-known/arroba/publication/status", localUrl).toString()
  const deadline = Date.now() + runtimeStartTimeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = await fetch(statusUrl, { signal: AbortSignal.timeout(1_000) })
      if (response.ok) return
      lastError = new Error(`status endpoint returned HTTP ${response.status}`)
    } catch (error) {
      lastError = error
    }
    await sleep(500)
  }
  const message = lastError instanceof Error ? lastError.message : String(lastError)
  throw new Error(`publication runtime at ${localUrl} did not become ready: ${message}`)
}

async function stopRuntime(client, sessionId, matrixEntry, prefix) {
  const stop = activeRuntimeStops.pop()
  if (!stop) return
  try {
    const stopped = await unwrap(
      client.send(controlWorkflowPublicationRuntimeRequest(sessionId, stop.publicationId, "stop")),
      "WorkflowPublicationRuntimeControlled",
    )
    await writeArtifact(`${prefix}/runtime-stopped.json`, stopped)
  } catch (error) {
    const message = error instanceof Error ? error.stack ?? error.message : String(error)
    matrixEntry.runtime_stop_error = message
    await writeArtifact(`${prefix}/runtime-stop-error.txt`, message)
  }
}

async function stopActiveRuntimes(client) {
  while (activeRuntimeStops.length > 0) {
    const stop = activeRuntimeStops.pop()
    try {
      await client.send(controlWorkflowPublicationRuntimeRequest(stop.sessionId, stop.publicationId, "stop"))
    } catch {
      // Best-effort cleanup during process shutdown.
    }
  }
}

async function invokeTransport(localUrl, transport, provider, policy) {
  if (transport.id === "schedule_only") return { transport: transport.id, message: "no ingress; status endpoint only" }
  const prompt = promptFor(provider, transport, policy)
  const base = new URL(localUrl)
  if (transport.id === "human_http") {
    const url = new URL(`/prompt/${encodeURIComponent(prompt)}`, base)
    const response = await fetch(url, { headers: { accept: "application/json" }, signal: AbortSignal.timeout(protocolTimeoutMs) })
    return { transport: transport.id, url: url.toString(), status: response.status, body: await response.text() }
  }
  if (transport.id === "api_sse_json") {
    const url = new URL("/invoke", base)
    const response = await fetch(url, {
      method: "POST",
      headers: { accept: "text/event-stream", "content-type": "application/json" },
      body: JSON.stringify({ prompt }),
      signal: AbortSignal.timeout(protocolTimeoutMs),
    })
    return { transport: transport.id, url: url.toString(), status: response.status, body: await response.text() }
  }
  if (transport.id === "websocket_json") {
    const url = new URL("/socket", base)
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:"
    return await invokeWebSocket(url.toString(), { type: "invoke", input: { prompt } })
  }
  if (transport.id === "mcp") {
    const url = new URL("/mcp", base)
    const initialize = await postJson(url, { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-03-26" } })
    const tools = await postJson(url, { jsonrpc: "2.0", id: 2, method: "tools/list" })
    const toolName = tools?.result?.tools?.[0]?.name
    const called = await postJson(url, { jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: toolName, arguments: { prompt } } })
    return { transport: transport.id, url: url.toString(), initialize, tools, called }
  }
  throw new Error(`unsupported transport ${transport.id}`)
}

async function waitForPublicationStatus(localUrl, transport) {
  const statusUrl = new URL("/.well-known/arroba/publication/status", localUrl).toString()
  const deadline = Date.now() + (transport.id === "schedule_only" ? 180_000 : 30_000)
  let last = null
  while (Date.now() < deadline) {
    last = await fetchJson(statusUrl)
    if (transport.id !== "schedule_only") return last
    if (last?.latest_run?.id || last?.last_run?.id || last?.runs?.length > 0) return last
    await sleep(1_000)
  }
  return last
}

function promptFor(provider, transport, policy) {
  const suffix = `${provider.provider}_${transport.id}_${policy.id}`.replace(/[^a-z0-9_]/gi, "_")
  return [
    `Run live publication trace validation for ${suffix}.`,
    `Follow the workflow node instructions exactly and keep channel-specific marker text in the channel requested by each node.`,
    `The final workflow output should only contain the final-output marker requested by the finalizer node.`,
  ].join("\n")
}

function workflowCodeSource(provider) {
  const model = JSON.stringify(provider.resolved_model)
  const providerId = JSON.stringify(provider.provider)
  const outputSummaryMarker = markerNames.output_summary
  const assistantMarker = markerNames.assistant_messages
  const toolMarker = markerNames.tool_use
  const finalMarker = markerNames.final
  return `
workflow.define({ alias: "live-trace-${provider.provider}", maxConcurrent: 4 });
const finalOutput = workflow.schema({
  handle: "final",
  alias: "Trace final",
  schema: { type: "object", additionalProperties: true }
});
const handoff = workflow.schema({
  handle: "handoff",
  alias: "Trace handoff",
  schema: { type: "object", additionalProperties: true }
});
workflow.define({ runOutputSchema: finalOutput });
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "trace-planner", provider: ${providerId}, model: ${model} }),
  publicLabel: "Planner",
  instructions: "Create a concise plan. Put ${outputSummaryMarker} only in your node summary. Put ${assistantMarker} only in the JSON handoff message to Worker. Do not put ${toolMarker} or ${finalMarker} in this node output.",
  canCompleteWorkflowRun: false,
  maxTurns: 2,
  canvas: { x: 0, y: 120 },
});
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "trace-worker", provider: ${providerId}, model: ${model} }),
  publicLabel: "Worker",
  instructions: "Do the work. Use normal Arroba workflow tools to validate and hand off to Finalizer. Put ${toolMarker} only in the tool-mediated handoff message. Do not put ${outputSummaryMarker}, ${assistantMarker}, or ${finalMarker} in this node output.",
  canCompleteWorkflowRun: false,
  maxTurns: 2,
  canvas: { x: 320, y: 120 },
});
const finalizer = workflow.node({
  handle: "finalizer",
  agent: workflow.newAgent({ alias: "trace-finalizer", provider: ${providerId}, model: ${model} }),
  publicLabel: "Finalizer",
  instructions: "Complete the workflow with a compact JSON-compatible final output containing ${finalMarker}. Do not include ${outputSummaryMarker}, ${assistantMarker}, or ${toolMarker} in the final workflow output.",
  canCompleteWorkflowRun: true,
  maxTurns: 2,
  canvas: { x: 640, y: 120 },
});
workflow.edge(planner, worker, { handle: "planner_worker", handoffSchema: handoff });
workflow.edge(worker, finalizer, { handle: "worker_finalizer", handoffSchema: handoff });
const entry = workflow.endpoint(planner, { handle: "entry", alias: "entry", canvas: { x: -220, y: 120 } });
workflow.watchdog(entry, { handle: "schedule", queue: "default", enabled: true, intervalSeconds: 1, invocationPrompt: "Scheduled live trace validation.", policy: "queue", maxWakeups: 1 });
`
}

function assertExposure({ visible, workflowRun, policy, nodeIds }) {
  const serializedRun = JSON.stringify(workflowRun ?? {})
  const providerEmitted = {
    output_summary: serializedRun.includes(markerNames.output_summary) || /summary/i.test(serializedRun),
    assistant_messages: serializedRun.includes(markerNames.assistant_messages),
    thinking: /thinking_traces":\s*\[/.test(serializedRun) && !/thinking_traces":\s*\[\s*\]/.test(serializedRun),
    tool_use: /runtime_tool_calls":\s*\[/.test(serializedRun) && !/runtime_tool_calls":\s*\[\s*\]/.test(serializedRun),
  }
  const enabled = policy.mixed
    ? new Set(["output_summary", "assistant_messages", "thinking", "tool_use"])
    : new Set(policy.levels ?? [])
  const failures = []
  for (const level of ["output_summary", "assistant_messages", "thinking", "tool_use"]) {
    const marker = markerNames[level]
    const present = visible.includes(marker)
      || (level === "thinking" && visible.includes("thinking_traces"))
      || (level === "tool_use" && visible.includes("runtime_tool_calls"))
    if (enabled.has(level) && providerEmitted[level] && !present) failures.push(`${level} was emitted by provider but absent from exposed output`)
    if (!enabled.has(level) && present) failures.push(`${level} marker leaked while disabled`)
  }
  if (policy.mixed) {
    const finalizerId = nodeIds.finalizer
    const finalizerLeak = visible.includes(finalizerId) && (visible.includes(markerNames.thinking) || visible.includes(markerNames.tool_use))
    if (finalizerLeak) failures.push("mixed per-node policy leaked hidden finalizer detail")
  }
  return { ok: failures.length === 0, failures, provider_emitted: providerEmitted }
}

async function workflowRunSnapshot(client, sessionId, runId) {
  const response = await client.send(getWorkflowRunRequest(sessionId, runId))
  return response?.WorkflowRun?.workflow_run ?? response
}

async function fetchJson(url) {
  const response = await fetch(url)
  const text = await response.text()
  try {
    return JSON.parse(text)
  } catch {
    return { status: response.status, text }
  }
}

async function postJson(url, payload) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
    signal: AbortSignal.timeout(protocolTimeoutMs),
  })
  return JSON.parse(await response.text())
}

async function invokeWebSocket(url, payload) {
  return await new Promise((resolve, reject) => {
    const messages = []
    const socket = new WebSocket(url)
    const timeout = setTimeout(() => {
      socket.close()
      reject(new Error(`websocket invocation timed out for ${url}`))
    }, protocolTimeoutMs)
    socket.on("message", (data) => {
      const text = String(data)
      messages.push(text)
      if (text.includes('"type":"ready"')) socket.send(JSON.stringify(payload))
      if (text.includes('"type":"final"') || text.includes('"type":"error"')) {
        clearTimeout(timeout)
        socket.close()
        resolve({ transport: "websocket_json", url, messages })
      }
    })
    socket.on("error", (error) => {
      clearTimeout(timeout)
      reject(error)
    })
  })
}

async function freePort() {
  return await new Promise((resolve, reject) => {
    const server = createServer()
    server.listen(0, "127.0.0.1", () => {
      const address = server.address()
      const port = typeof address === "object" && address ? address.port : null
      server.close(() => port ? resolve(port) : reject(new Error("failed to allocate free port")))
    })
    server.on("error", reject)
  })
}

async function unwrap(promise, variant) {
  const response = await promise
  const payload = response?.[variant]
  if (!payload) throw new Error(`kernel response missing ${variant}: ${JSON.stringify(response)}`)
  return payload
}

async function withTimeout(promise, timeoutMs, label) {
  let timeout
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timeout = setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs} ms`)), timeoutMs)
      }),
    ])
  } finally {
    clearTimeout(timeout)
  }
}

async function commandVersion(command, args) {
  return await new Promise((resolve) => {
    const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"] })
    let output = ""
    child.stdout.on("data", (chunk) => output += String(chunk))
    child.stderr.on("data", (chunk) => output += String(chunk))
    child.on("error", (error) => resolve({ ok: false, command, error: error.message }))
    child.on("close", (code) => resolve({ ok: code === 0, command, code, output: output.trim() }))
  })
}

async function writeArtifact(relativePath, value) {
  const path = join(artifactsDir, relativePath)
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, typeof value === "string" ? value : JSON.stringify(value, null, 2))
  return path
}

function csvArg(name) {
  const prefix = `${name}=`
  const arg = process.argv.find((entry) => entry.startsWith(prefix))
  if (!arg) return null
  const values = arg.slice(prefix.length).split(",").map((entry) => entry.trim()).filter(Boolean)
  return values.length > 0 ? new Set(values) : null
}

function selected(set, value) {
  return !set || set.has(value)
}

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

async function writeReport() {
  await writeArtifact("report.json", report)
  const lines = [
    "# Live Publication Trace Drill",
    "",
    `Generated: ${report.generated_at}`,
    `Kernel: ${report.kernel_url}`,
    `Workflow-code Node: ${report.workflow_code_node_path}`,
    `Artifacts: ${report.artifacts_dir}`,
    "",
    "## Preflight",
    "",
    "| Provider | Requested model | Resolved model | Auth | Status |",
    "|---|---|---|---|---|",
    ...(report.preflight?.providers ?? []).map((provider) => `| ${provider.provider} | ${provider.requested_model} | ${provider.resolved_model ?? "missing"} | ${provider.auth?.ok ? "ok" : "failed"} | ${provider.ok ? "ok" : provider.unavailable_reason} |`),
    "",
    "## Matrix",
    "",
    "| Provider | Model | Transport | Policy | Status | Thinking | Tool use | Error |",
    "|---|---|---|---|---|---|---|---|",
    ...report.matrix.map((entry) => `| ${entry.provider} | ${entry.model ?? ""} | ${entry.transport} | ${entry.policy} | ${entry.status} | ${entry.thinking_emitted ?? ""} | ${entry.tool_use_emitted ?? ""} | ${(entry.error ?? "").replaceAll("|", "\\|")} |`),
    "",
  ]
  await writeArtifact("report.md", lines.join("\n"))
}
