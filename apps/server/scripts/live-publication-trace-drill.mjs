#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, mkdtemp, writeFile } from "node:fs/promises"
import { createServer } from "node:http"
import { tmpdir } from "node:os"
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
  getWorkflowPublicationRequest,
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
const scheduleOnlyObservationTimeoutMs = Number(process.env.ARROBA_TRACE_DRILL_SCHEDULE_ONLY_TIMEOUT_MS ?? 480_000)
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
  { id: "api_sse_json", route: "/invoke", methods: ["POST"], transport: { kind: "api_sse_json" }, parser: { kind: "json" }, mode: "async" },
  { id: "websocket_json", route: "/socket", methods: [], transport: { kind: "websocket_json" }, parser: null, mode: "async" },
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
    const modelResolution = resolveRequestedModel(providerCatalog, spec)
    const model = modelResolution.model
    const auth = await providerAuth(client, spec)
    providers.push({
      provider: spec.provider,
      requested_model: spec.requestedModel,
      requested_model_available: modelResolution.requestedAvailable,
      resolved_model: model?.id ?? null,
      resolved_model_name: model?.name ?? null,
      model_status: model?.status ?? null,
      model_resolution: modelResolution.reason,
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
  const requested = models.find((model) => spec.acceptableModelIds.includes(model.id))
    ?? models.find((model) => normalizeModelToken(model.name) === normalizeModelToken(spec.requestedModel))
  if (requested) {
    return { model: requested, requestedAvailable: true, reason: "requested" }
  }
  const fallback = models.find((model) => isUsableFallbackModel(model) && matchesRequestedModelFamily(model, spec))
    ?? models.find(isUsableFallbackModel)
    ?? models[0]
    ?? null
  return {
    model: fallback,
    requestedAvailable: false,
    reason: fallback
      ? `fallback: requested model ${spec.requestedModel} was not found`
      : `missing: requested model ${spec.requestedModel} was not found and provider has no catalog models`,
  }
}

function isUsableFallbackModel(model) {
  const status = String(model?.status ?? "").toLowerCase()
  if (!status) return true
  return !/(unavailable|disabled|deprecated|unauth|missing|error)/.test(status)
}

function matchesRequestedModelFamily(model, spec) {
  const requested = normalizeModelToken(spec.requestedModel)
  const candidate = normalizeModelToken(`${model?.id ?? ""} ${model?.name ?? ""}`)
  const families = ["sonnet", "opus", "haiku", "fable", "gpt", "kimi"]
  return families.some((family) => requested.includes(family) && candidate.includes(family))
}

function normalizeModelToken(value) {
  return String(value ?? "").toLowerCase().replace(/[^a-z0-9]+/g, "")
}

async function runProviderMatrix(client, provider) {
  const sessionAlias = `trace-${provider.provider}-${Date.now()}`
  const workspaceRoot = await mkdtemp(join(tmpdir(), `${sessionAlias}-`))
  const session = await unwrap(
    client.send(createSessionRequest(workspaceRoot, workspaceRoot, sessionAlias)),
    "SessionCreated",
  )
  const sessionId = session.session.id
  const appliedByTransport = new Map()

  for (const policy of policies.filter((policy) => selected(requestedPolicies, policy.id))) {
    for (const transport of transports.filter((transport) => selected(requestedTransports, transport.id))) {
      const { workflowId, endpointId, nodeIds } = await workflowForTransport(client, sessionId, provider, transport, appliedByTransport)
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
      let rawResponseWritten = false
      try {
        const publication = await createPublication(client, sessionId, workflowId, endpointId, nodeIds, policy, transport, prefix)
        await writeArtifact(`${prefix}/publication.json`, publication)
        const runtime = await startPublicationRuntime(client, sessionId, publication.id, transport, prefix)
        activeRuntimeStops.push({ sessionId, publicationId: publication.id })
        await writeArtifact(`${prefix}/runtime.json`, runtime)
        const raw = await withTimeout(
          invokeTransport(runtime.local_url ?? runtime.open_url ?? runtime.viewer_url, transport, provider, policy),
          protocolTimeoutMs + 5_000,
          `invoking ${transport.id} publication ${publication.id}`,
        )
        await writeArtifact(`${prefix}/raw-response.json`, raw)
        rawResponseWritten = true
        let statusPayload = runtime.local_url
          ? await waitForPublicationStatus(runtime.local_url, transport)
          : transport.id === "schedule_only"
            ? await waitForScheduleOnlyPublicationStatus(client, sessionId, publication.id)
            : null
        if (transport.id === "schedule_only" && !latestRunIdFromStatus(statusPayload)) {
          const stopped = await stopRuntime(client, sessionId, matrixEntry, prefix)
          if (stopped) statusPayload = statusPayloadFromRuntimeControl(stopped, statusPayload)
        }
        await writeArtifact(`${prefix}/publication-status.json`, statusPayload)
        const latestRunId = latestRunIdFromStatus(statusPayload) ?? raw.workflow_run_id ?? null
        const workflowRunSessionId = statusPayload?.runtime_session_id ?? statusPayload?.inspect?.publication?.session_id ?? sessionId
        const workflowRun = latestRunId ? await workflowRunSnapshot(client, workflowRunSessionId, latestRunId) : null
        await writeArtifact(`${prefix}/workflow-run.json`, workflowRun)
        const assertions = assertExposure({
          raw,
          statusPayload,
          visible: JSON.stringify([raw, statusPayload]),
          workflowRun,
          policy,
          transport,
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
        if (!rawResponseWritten) {
          await writeArtifact(`${prefix}/raw-response.json`, {
            transport: transport.id,
            timed_out: /timed out/i.test(matrixEntry.error),
            error: matrixEntry.error,
          })
        }
        await writeArtifact(`${prefix}/error.txt`, matrixEntry.error)
      } finally {
        await stopRuntime(client, sessionId, matrixEntry, prefix)
      }
    }
  }
}

async function workflowForTransport(client, sessionId, provider, transport, appliedByTransport) {
  const cacheKey = transport.id === "schedule_only"
    ? null
    : transport.id
  const cached = cacheKey ? appliedByTransport.get(cacheKey) : null
  if (cached) return cached
  const workflowSource = workflowCodeSource(provider, transport)
  const applied = await unwrap(
    client.send(applyWorkflowCodeRequest(sessionId, workflowCodeNodePath, workflowSource)),
    "WorkflowCodeApplied",
  )
  await writeArtifact(`${provider.provider}/${transport.id}/workflow-applied.json`, applied)
  const compiled = {
    workflowId: applied.result.apply.workflow_id,
    endpointId: applied.result.apply.endpoint_ids.entry,
    nodeIds: applied.result.apply.node_ids,
  }
  if (cacheKey) appliedByTransport.set(cacheKey, compiled)
  return compiled
}

async function createPublication(client, sessionId, workflowId, endpointId, nodeIds, policy, transport, prefix) {
  const traceExposure = traceExposureForPolicy(policy, nodeIds)
  const publicationOptions = {
    alias: `${prefix}`.replaceAll("/", "-").slice(0, 60),
    queueRef: "default",
    kind: transport.kind ?? "ingress",
    route: transport.route,
    methods: transport.methods,
    transport: transport.transport,
    parser: transport.parser,
    traceExposure,
    syncTimeoutMs: 240_000,
    pollMs: 500,
  }
  if (transport.id !== "schedule_only") publicationOptions.mode = transport.mode
  const response = await unwrap(
    client.send(createWorkflowPublicationRequest(sessionId, workflowId, endpointId, publicationOptions)),
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

async function startPublicationRuntime(client, sessionId, publicationId, transport, prefix) {
  const failures = []
  for (let attempt = 1; attempt <= 2; attempt += 1) {
    const port = await freePort()
    const response = await unwrap(
      withTimeout(client.send(controlWorkflowPublicationRuntimeRequest(sessionId, publicationId, "start", {
        host: "127.0.0.1",
        port,
        kernelUrl,
      })), runtimeStartTimeoutMs, `starting publication runtime ${publicationId}`),
      "WorkflowPublicationRuntimeControlled",
    )
    await writeArtifact(`${prefix}/runtime-start-attempt-${attempt}.json`, response)
    await writeArtifact(`${prefix}/runtime-start.json`, response)
    try {
      if (transport.id !== "schedule_only" && !response.local_url) throw new Error("publication runtime did not return a local_url")
      if (response.local_url) await waitForGatewayReady(response.local_url)
      return response
    } catch (error) {
      const message = error instanceof Error ? error.stack ?? error.message : String(error)
      failures.push(message)
      await writeArtifact(`${prefix}/runtime-start-attempt-${attempt}-error.txt`, message)
      await client.send(controlWorkflowPublicationRuntimeRequest(sessionId, publicationId, "stop")).catch(() => {})
      if (attempt === 2) throw new Error(failures.join("\n--- retry ---\n"))
    }
  }
  throw new Error("publication runtime did not start")
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
    return stopped
  } catch (error) {
    const message = error instanceof Error ? error.stack ?? error.message : String(error)
    matrixEntry.runtime_stop_error = message
    await writeArtifact(`${prefix}/runtime-stop-error.txt`, message)
    return null
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
    return await invokeHumanHttp(url.toString())
  }
  if (transport.id === "api_sse_json") {
    const url = new URL("/invoke", base)
    return await invokeApiSse(url.toString(), {
      method: "POST",
      headers: { accept: "text/event-stream", "content-type": "application/json" },
      body: JSON.stringify({ prompt }),
    })
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
    const calledRaw = await postJsonCapture(url, { jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: toolName, arguments: { prompt } } })
    return { transport: transport.id, url: url.toString(), initialize, tools, called: calledRaw.json, called_raw: calledRaw }
  }
  throw new Error(`unsupported transport ${transport.id}`)
}

async function invokeHumanHttp(url) {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(new Error(`human_http invocation timed out for ${url}`)), protocolTimeoutMs)
  let status = null
  let body = ""
  try {
    const response = await fetch(url, { headers: { accept: "application/json" }, signal: controller.signal })
    status = response.status
    body = await response.text()
    return { transport: "human_http", url, status, body }
  } catch (error) {
    return {
      transport: "human_http",
      url,
      status,
      body,
      timed_out: error?.name === "AbortError" || /timed out/i.test(String(error?.message ?? error)),
      error: error instanceof Error ? error.message : String(error),
    }
  } finally {
    clearTimeout(timeout)
  }
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

async function waitForScheduleOnlyPublicationStatus(client, sessionId, publicationId) {
  const deadline = Date.now() + scheduleOnlyObservationTimeoutMs
  let last = null
  while (Date.now() < deadline) {
    const inspect = await unwrap(
      client.send(controlWorkflowPublicationRuntimeRequest(sessionId, publicationId, "inspect")),
      "WorkflowPublicationRuntimeControlled",
    )
    const publicationResponse = await unwrap(
      client.send(getWorkflowPublicationRequest(sessionId, publicationId)),
      "WorkflowPublication",
    )
    last = {
      inspect,
      publication: publicationResponse.publication,
      latest_run: publicationResponse.publication?.latest_run ?? inspect.publication?.latest_run ?? null,
      last_run: publicationResponse.publication?.last_run ?? inspect.publication?.last_run ?? null,
      latest_output: publicationResponse.publication?.latest_output ?? inspect.publication?.latest_output ?? null,
      runtime_session_id: inspect.publication?.session_id ?? sessionId,
    }
    const latestRunId = latestRunIdFromStatus(last)
    if (latestRunId) {
      const workflowRun = await workflowRunSnapshot(client, last.runtime_session_id, latestRunId).catch(() => null)
      last.workflow_run = workflowRun
      if (workflowRun?.status && isTerminalStatus(workflowRun.status)) return last
    }
    await sleep(1_000)
  }
  return last
}

function isTerminalStatus(status) {
  return ["Completed", "Failed", "Stopped"].includes(status)
}

function latestRunIdFromStatus(statusPayload) {
  return statusPayload?.latest_run?.id
    ?? statusPayload?.last_run?.id
    ?? statusPayload?.publication?.latest_run?.id
    ?? statusPayload?.publication?.last_run?.id
    ?? statusPayload?.inspect?.publication?.latest_run?.id
    ?? statusPayload?.inspect?.publication?.last_run?.id
    ?? null
}

function statusPayloadFromRuntimeControl(controlResponse, previousStatus) {
  const publication = controlResponse?.publication ?? null
  return {
    ...(previousStatus ?? {}),
    runtime_control: controlResponse,
    publication,
    latest_run: publication?.latest_run ?? previousStatus?.latest_run ?? null,
    last_run: publication?.last_run ?? previousStatus?.last_run ?? null,
    latest_output: publication?.latest_output ?? previousStatus?.latest_output ?? null,
    runtime_session_id: publication?.session_id ?? previousStatus?.runtime_session_id ?? null,
  }
}

function promptFor(provider, transport, policy) {
  const suffix = `${provider.provider}_${transport.id}_${policy.id}`.replace(/[^a-z0-9_]/gi, "_")
  return [
    `Run live publication trace validation for ${suffix}.`,
    `Follow the workflow node instructions exactly and keep channel-specific marker text in the channel requested by each node.`,
    `The final workflow output should only contain the final-output marker requested by the finalizer node.`,
  ].join("\n")
}

function workflowCodeSource(provider, transport) {
  const model = JSON.stringify(provider.resolved_model)
  const providerId = JSON.stringify(provider.provider)
  const outputSummaryMarker = markerNames.output_summary
  const assistantMarker = markerNames.assistant_messages
  const toolMarker = markerNames.tool_use
  const finalMarker = markerNames.final
  const workflowAlias = JSON.stringify(`live-trace-${provider.provider}-${transport.id}`)
  const watchdog = transport.id === "schedule_only"
    ? `workflow.watchdog(entry, { handle: "schedule", queue: "default", enabled: true, intervalSeconds: 1, invocationPrompt: "Scheduled live trace validation.", policy: "queue", maxWakeups: 1 });`
    : ""
  return `
workflow.define({ alias: ${workflowAlias}, maxConcurrent: 4 });
const finalOutput = workflow.schema({
  handle: "final",
  alias: "Trace final",
  schema: { type: "object", additionalProperties: true }
});
workflow.define({ runOutputSchema: finalOutput });
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "trace-planner", provider: ${providerId}, model: ${model} }),
  publicLabel: "Planner",
  instructions: "Create a concise plan. Set your node summary to exactly ${outputSummaryMarker}. Send the Worker a compact JSON handoff message exactly {\\\"message\\\":\\\"${assistantMarker}\\\"}. Do not include ${assistantMarker}, ${toolMarker}, or ${finalMarker} in your summary.",
  canCompleteWorkflowRun: false,
  maxTurns: 2,
  canvas: { x: 0, y: 120 },
});
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "trace-worker", provider: ${providerId}, model: ${model} }),
  publicLabel: "Worker",
  instructions: "Call workflow_console_write exactly once with content ${toolMarker}. Set your node summary to exactly worker_done. Then send the Finalizer a compact JSON handoff message exactly {\\\"message\\\":\\\"worker_done\\\"}. Do not include any TRACE_ marker text in your summary or handoff message.",
  canCompleteWorkflowRun: false,
  maxTurns: 2,
  canvas: { x: 320, y: 120 },
});
const finalizer = workflow.node({
  handle: "finalizer",
  agent: workflow.newAgent({ alias: "trace-finalizer", provider: ${providerId}, model: ${model} }),
  publicLabel: "Finalizer",
  instructions: "Set your node summary to exactly final_done. Complete the workflow with compact JSON exactly {\\\"message\\\":\\\"${finalMarker}\\\"}. Do not include ${outputSummaryMarker}, ${assistantMarker}, or ${toolMarker} in the final workflow output.",
  canCompleteWorkflowRun: true,
  maxTurns: 2,
  canvas: { x: 640, y: 120 },
});
workflow.edge(planner, worker, { handle: "planner_worker" });
workflow.edge(worker, finalizer, { handle: "worker_finalizer" });
const entry = workflow.endpoint(planner, { handle: "entry", alias: "entry", canvas: { x: -220, y: 120 } });
${watchdog}
`
}

function assertExposure({ raw, statusPayload, visible, workflowRun, policy, transport, nodeIds }) {
  const serializedRun = JSON.stringify(workflowRun ?? {})
  const providerEmitted = {
    output_summary: serializedRun.includes(markerNames.output_summary),
    assistant_messages: serializedRun.includes(markerNames.assistant_messages),
    thinking: /thinking_traces":\s*\[/.test(serializedRun) && !/thinking_traces":\s*\[\s*\]/.test(serializedRun),
    tool_use: /runtime_tool_calls":\s*\[/.test(serializedRun) && !/runtime_tool_calls":\s*\[\s*\]/.test(serializedRun),
  }
  const enabled = new Set(policy.levels ?? [])
  const failures = []
  for (const failure of protocolFailures(raw, statusPayload, workflowRun, transport)) {
    failures.push(failure)
  }
  for (const level of ["output_summary", "assistant_messages", "thinking", "tool_use"]) {
    const marker = markerNames[level]
    const structuralPresent = structurallyExposesLevel(visible, level)
    const markerPresent = visible.includes(marker)
    const present = markerPresent || structuralPresent
    if (!policy.mixed && enabled.has(level) && providerEmitted[level] && !present) {
      failures.push(`${level} was emitted by provider but absent from exposed output`)
    }
    if (!policy.mixed && !enabled.has(level)) {
      if (structuralPresent) {
        failures.push(`${level} structural data leaked while disabled`)
      } else if (!(enabled.has("thinking") && level !== "thinking") && markerPresent) {
        failures.push(`${level} marker leaked while disabled`)
      }
    }
  }
  if (policy.mixed) {
    const plannerSummaryVisible = visible.includes(markerNames.output_summary)
    const plannerAssistantVisible = visible.includes(markerNames.assistant_messages)
    if (providerEmitted.output_summary && !plannerSummaryVisible) {
      failures.push("mixed per-node policy hid planner output_summary")
    }
    if (providerEmitted.assistant_messages && !plannerAssistantVisible) {
      failures.push("mixed per-node policy hid planner assistant_messages")
    }
    if (nodeDetailLeaks([raw, statusPayload], nodeIds.worker, ["assistant_messages", "thinking", "tool_use"])) {
      failures.push("mixed per-node policy leaked worker detail beyond output_summary")
    }
    if (nodeDetailLeaks([raw, statusPayload], nodeIds.finalizer, ["output_summary", "assistant_messages", "thinking", "tool_use"])) {
      failures.push("mixed per-node policy leaked hidden finalizer detail")
    }
  }
  return { ok: failures.length === 0, failures, provider_emitted: providerEmitted }
}

function nodeDetailLeaks(value, nodeId, disallowedLevels) {
  if (!nodeId || value == null) return false
  if (Array.isArray(value)) return value.some((entry) => nodeDetailLeaks(entry, nodeId, disallowedLevels))
  if (typeof value !== "object") return false
  const record = value
  if (record.node_id === nodeId) {
    if (typeof record.level === "string" && disallowedLevels.includes(record.level)) return true
    if (disallowedLevels.includes("output_summary")) {
      if (record.summary !== undefined || record.completion_summary !== undefined) return true
      if (record.completion && typeof record.completion === "object" && record.completion.summary !== undefined) return true
    }
    if (disallowedLevels.includes("assistant_messages")) {
      if (record.handoff_payload !== undefined || record.message_type === "handoff") return true
      if (record.completion && typeof record.completion === "object" && record.completion.output !== undefined) return true
    }
    if (disallowedLevels.includes("thinking") && record.thinking_traces !== undefined) return true
    if (disallowedLevels.includes("tool_use") && record.turn_envelope !== undefined) return true
  }
  return Object.values(record).some((entry) => nodeDetailLeaks(entry, nodeId, disallowedLevels))
}

function structurallyExposesLevel(visible, level) {
  if (level === "output_summary") {
    return visible.includes('"level":"output_summary"')
      || visible.includes('"completion_summary"')
  }
  if (level === "assistant_messages") {
    return visible.includes('"level":"assistant_messages"')
      || visible.includes('"handoff_payload"')
      || visible.includes('"message_type":"handoff"')
  }
  if (level === "thinking") {
    return visible.includes("thinking_traces") || visible.includes('"level":"thinking"')
  }
  if (level === "tool_use") {
    return visible.includes("runtime_tool_calls") || visible.includes('"level":"tool_use"')
  }
  return false
}

function protocolFailures(raw, statusPayload, workflowRun, transport) {
  const failures = []
  if (transport.id === "schedule_only") {
    if (!workflowRun?.id && !statusPayload?.latest_run?.id && !statusPayload?.last_run?.id) failures.push("schedule-only publication did not produce an observable run")
    if (workflowRun?.status && workflowRun.status !== "Completed") failures.push(`workflow run ended validation in status ${workflowRun.status}`)
    if (!workflowRun?.id) failures.push("workflow run snapshot was not captured")
    return failures
  }
  if (raw?.timed_out) failures.push(`${transport.id} protocol stream timed out before completion`)
  if (raw?.error && raw.status == null) failures.push(`${transport.id} protocol invocation error: ${raw.error}`)
  if (transport.id === "api_sse_json") {
    const events = Array.isArray(raw?.events) ? raw.events.map((event) => event.event) : []
    if (!events.includes("final")) failures.push("api_sse_json stream did not emit final event")
  }
  if (transport.id === "websocket_json") {
    const messages = Array.isArray(raw?.messages) ? raw.messages.join("\n") : ""
    if (!messages.includes('"type":"final"')) failures.push("websocket_json stream did not emit final frame")
  }
  if (transport.id === "mcp") {
    if (raw?.called_raw?.timed_out) failures.push("mcp tools/call timed out before returning JSON-RPC result")
    if (!raw?.called?.result) failures.push("mcp tools/call did not return a result")
  }
  if (transport.id === "human_http" && !(raw?.status >= 200 && raw?.status < 300)) failures.push(`human_http returned HTTP ${raw?.status ?? "unknown"}`)
  if (workflowRun?.status && workflowRun.status !== "Completed") failures.push(`workflow run ended validation in status ${workflowRun.status}`)
  if (!workflowRun?.id) failures.push("workflow run snapshot was not captured")
  return failures
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
  const captured = await postJsonCapture(url, payload)
  if (captured.json) return captured.json
  throw new Error(captured.error ?? `invalid JSON response from ${url}`)
}

async function postJsonCapture(url, payload) {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(new Error(`JSON request timed out for ${url}`)), protocolTimeoutMs)
  let status = null
  let text = ""
  try {
    const response = await fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
      signal: controller.signal,
    })
    status = response.status
    const reader = response.body?.getReader()
    if (!reader) {
      text = await response.text()
    } else {
      const decoder = new TextDecoder()
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        text += decoder.decode(value, { stream: true })
      }
      text += decoder.decode()
    }
    return parseCapturedJson(status, text)
  } catch (error) {
    const timedOut = error?.name === "AbortError" || /timed out/i.test(String(error?.message ?? error))
    return {
      status,
      text,
      json: null,
      timed_out: timedOut,
      error: error instanceof Error ? error.message : String(error),
    }
  } finally {
    clearTimeout(timeout)
  }
}

function parseCapturedJson(status, text) {
  try {
    return { status, text, json: JSON.parse(text.trim()), timed_out: false, error: null }
  } catch (error) {
    return {
      status,
      text,
      json: null,
      timed_out: false,
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

async function invokeApiSse(url, init) {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(new Error(`api_sse_json invocation timed out for ${url}`)), protocolTimeoutMs)
  const events = []
  let body = ""
  try {
    const response = await fetch(url, { ...init, signal: controller.signal })
    const reader = response.body?.getReader()
    if (!reader) return { transport: "api_sse_json", url, status: response.status, body, events }
    const decoder = new TextDecoder()
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      body += decoder.decode(value, { stream: true })
      parseSseEvents(body, events)
      if (events.some((event) => event.event === "final" || event.event === "error" || event.event === "timeout")) break
    }
    body += decoder.decode()
    parseSseEvents(body, events)
    await reader.cancel().catch(() => {})
    return { transport: "api_sse_json", url, status: response.status, body, events }
  } catch (error) {
    return {
      transport: "api_sse_json",
      url,
      status: null,
      body,
      events,
      timed_out: error?.name === "AbortError" || /timed out/i.test(String(error?.message ?? error)),
      error: error instanceof Error ? error.message : String(error),
    }
  } finally {
    clearTimeout(timeout)
  }
}

function parseSseEvents(body, events) {
  const parsedCount = events.reduce((count, event) => count + (event.raw?.length ?? 0), 0)
  const remaining = body.slice(parsedCount)
  const blocks = remaining.split(/\n\n/)
  for (let index = 0; index < blocks.length - 1; index += 1) {
    const raw = `${blocks[index]}\n\n`
    let eventName = "message"
    let data = ""
    for (const line of blocks[index].split(/\n/)) {
      if (line.startsWith("event:")) eventName = line.slice("event:".length).trim()
      if (line.startsWith("data:")) data += line.slice("data:".length).trim()
    }
    let parsed = data
    try {
      parsed = JSON.parse(data)
    } catch {
      // Keep raw string data for malformed or non-JSON event payloads.
    }
    events.push({ event: eventName, data: parsed, raw })
  }
}

async function invokeWebSocket(url, payload) {
  return await new Promise((resolve, reject) => {
    const messages = []
    const socket = new WebSocket(url)
    const timeout = setTimeout(() => {
      socket.close()
      resolve({ transport: "websocket_json", url, messages, timed_out: true })
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
    "| Provider | Requested model | Resolved model | Model resolution | Auth | Status |",
    "|---|---|---|---|---|---|",
    ...(report.preflight?.providers ?? []).map((provider) => `| ${provider.provider} | ${provider.requested_model} | ${provider.resolved_model ?? "missing"} | ${provider.model_resolution ?? ""} | ${provider.auth?.ok ? "ok" : "failed"} | ${provider.ok ? "ok" : provider.unavailable_reason} |`),
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
