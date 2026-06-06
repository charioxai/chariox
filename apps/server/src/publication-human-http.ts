import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import { getWorkflowRunRequest } from "@arroba/kernel-client/ipc-requests"

import { defaultKernelEndpoint } from "./kernel-publication-client.js"
import { waitForWorkflowRunByInvocationRequestId } from "./publication-run-correlation.js"
import { pumpPublicationRuntime } from "./publication-runtime-pump.js"
import {
  collectPublicationTraceEvents,
  createPublicationTraceStreamState,
  type PublicationTraceStreamState,
} from "./publication-trace-events.js"
import type {
  GatewayRequest,
  WorkflowInvocationResult,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"
import { isTerminalWorkflowRunStatus } from "./workflow-run-status.js"

type HumanHttpApp = {
  get: (path: string, handler: (request: { params?: unknown }, reply: HumanHttpReply) => unknown) => unknown
}

type HumanHttpReply = {
  code: (code: number) => HumanHttpReply
  type: (contentType: string) => HumanHttpReply
  hijack: () => void
  raw: {
    destroyed?: boolean
    writeHead: (statusCode: number, headers: Record<string, string>) => unknown
    write: (chunk: string) => unknown
    end: () => unknown
  }
}

type HumanHttpStreamState = {
  partialIds: Set<string>
  traces: PublicationTraceStreamState
}

export const HUMAN_HTTP_FORM_INVOKE_PATH = "/.well-known/arroba/publication/human-http/invoke"

export function installHumanHttpRoutes(app: HumanHttpApp, publication: WorkflowPublicationConfig) {
  app.get("/", async (_request, reply) => {
    if (!isHumanHttpPublication(publication)) {
      reply.code(404)
      return { error: "not found" }
    }
    reply.type("text/html; charset=utf-8")
    return humanHttpFormPage(publication)
  })

  app.get("/.well-known/arroba/publication/runs/:workflowRunId/events", async (request, reply) => {
    const params = request.params as { workflowRunId?: string }
    const workflowRunId = params.workflowRunId
    if (!workflowRunId) {
      reply.code(400)
      return { error: "workflow run id is required" }
    }
    await streamWorkflowRunEvents(reply, publication, workflowRunId)
  })

  app.get("/.well-known/arroba/publication/invocations/:requestId/events", async (request, reply) => {
    const params = request.params as { requestId?: string }
    const requestId = params.requestId
    if (!requestId) {
      reply.code(400)
      return { error: "invocation request id is required" }
    }
    await streamInvocationEvents(reply, publication, requestId)
  })
}

export function shouldReturnHumanHtml(
  request: GatewayRequest,
  publication: WorkflowPublicationConfig,
) {
  if (!isHumanHttpPublication(publication)) return false
  if (request.method.toUpperCase() !== "GET") return false
  const accept = request.headers.accept
  const values = Array.isArray(accept) ? accept : [accept]
  return values.some((value) => typeof value === "string" && value.includes("text/html"))
}

export function forwardHumanHttpResult(
  reply: Pick<HumanHttpReply, "code" | "type">,
  publication: WorkflowPublicationConfig,
  result: WorkflowInvocationResult,
  invocationRequestId?: string,
) {
  reply.code(200).type("text/html; charset=utf-8")
  return humanHttpStatusPage(publication, result, invocationRequestId)
}

function isHumanHttpPublication(publication: WorkflowPublicationConfig) {
  return !publication.transport || publication.transport === "human_http"
}

function humanHttpFormPage(publication: WorkflowPublicationConfig) {
  const promptTarget = promptTargetParts(publication.route ?? "/*")
  return htmlDocument(
    "Workflow",
    [
      "<main>",
      "  <section class=\"panel\">",
      "    <h1>Workflow</h1>",
      "    <form id=\"invoke-form\">",
      "      <textarea name=\"prompt\" rows=\"8\" autofocus></textarea>",
      "      <div class=\"actions\">",
      "        <input type=\"file\" name=\"artifact\" multiple>",
      "        <button type=\"submit\">Run</button>",
      "      </div>",
      "    </form>",
      "  </section>",
      "</main>",
      "<script>",
      `const promptTarget = ${safeJson(promptTarget)};`,
      `const formInvokePath = ${safeJson(HUMAN_HTTP_FORM_INVOKE_PATH)};`,
      "document.querySelector('#invoke-form')?.addEventListener('submit', async (event) => {",
      "  event.preventDefault();",
      "  const form = event.currentTarget;",
      "  const data = new FormData(form);",
      "  const prompt = String(data.get('prompt') ?? '').trim();",
      "  const files = data.getAll('artifact').filter((item) => item instanceof File && item.size > 0);",
      "  if (!files.length) {",
      "    const encoded = encodeURIComponent(prompt);",
      "    window.location.href = `${promptTarget.prefix}${encoded}${promptTarget.suffix}`;",
      "    return;",
      "  }",
      "  const button = form.querySelector('button[type=\"submit\"]');",
      "  if (button) button.disabled = true;",
      "  const artifacts = await Promise.all(files.map(readArtifact));",
      "  const response = await fetch(formInvokePath, {",
      "    method: 'POST',",
      "    headers: { accept: 'text/html', 'content-type': 'application/json' },",
      "    body: JSON.stringify({ prompt, artifacts }),",
      "  });",
      "  const html = await response.text();",
      "  document.open();",
      "  document.write(html);",
      "  document.close();",
      "});",
      "function readArtifact(file) {",
      "  return new Promise((resolve, reject) => {",
      "    const reader = new FileReader();",
      "    reader.addEventListener('load', () => resolve({",
      "      name: file.name,",
      "      type: file.type || 'application/octet-stream',",
      "      size_bytes: file.size,",
      "      data_url: String(reader.result || ''),",
      "    }));",
      "    reader.addEventListener('error', () => reject(reader.error || new Error('artifact read failed')));",
      "    reader.readAsDataURL(file);",
      "  });",
      "}",
      "</script>",
    ].join("\n"),
  )
}

function humanHttpStatusPage(
  publication: WorkflowPublicationConfig,
  result: WorkflowInvocationResult,
  invocationRequestId?: string,
) {
  const workflowRunId = result.workflow_run?.id ?? null
  const eventsUrl = workflowRunId
    ? `/.well-known/arroba/publication/runs/${encodeURIComponent(workflowRunId)}/events`
    : result.queued && invocationRequestId
      ? `/.well-known/arroba/publication/invocations/${encodeURIComponent(invocationRequestId)}/events`
    : null
  return htmlDocument(
    "Workflow Run",
    [
      "<main class=\"split-viewer\">",
      "  <section class=\"output-pane\">",
      "    <header class=\"pane-header\">",
      "      <h1>Workflow Run</h1>",
      "      <p id=\"status\">Accepted</p>",
      "    </header>",
      "    <pre id=\"output\"></pre>",
      "    <div id=\"html-output\" class=\"html-output\" hidden></div>",
      "  </section>",
      "  <aside class=\"trace-pane\">",
      "    <header class=\"pane-header\">",
      "      <h2>Traces</h2>",
      "      <p id=\"trace-status\">No exposed traces</p>",
      "    </header>",
      "    <div id=\"trace-feed\"></div>",
      "  </aside>",
      "</main>",
      "<script>",
      `const initialResult = ${safeJson(result)};`,
      `const eventsUrl = ${safeJson(eventsUrl)};`,
      "const statusEl = document.querySelector('#status');",
      "const outputEl = document.querySelector('#output');",
      "const htmlOutputEl = document.querySelector('#html-output');",
      "const traceStatusEl = document.querySelector('#trace-status');",
      "const traceFeedEl = document.querySelector('#trace-feed');",
      "const partialOutputs = [];",
      "function renderRun(run) {",
      "  if (!run) return;",
      "  statusEl.textContent = run.status || 'accepted';",
      "  if (run.final_output) renderFinalOutput(run.final_output.message);",
      "}",
      "function renderPartialOutput(message) {",
      "  if (typeof message !== 'string' || !message) return;",
      "  partialOutputs.push(message);",
      "  if (!htmlOutputEl.hidden) return;",
      "  outputEl.textContent = partialOutputs.join('\\n\\n');",
      "}",
      "function renderFinalOutput(message) {",
      "  if (typeof message !== 'string') {",
      "    outputEl.textContent = JSON.stringify(message, null, 2);",
      "    return;",
      "  }",
      "  const html = renderableHtml(message);",
      "  if (html !== null) {",
      "    outputEl.hidden = true;",
      "    htmlOutputEl.hidden = false;",
      "    htmlOutputEl.innerHTML = '';",
      "    const frame = document.createElement('iframe');",
      "    frame.setAttribute('sandbox', 'allow-scripts allow-forms allow-popups allow-modals');",
      "    frame.srcdoc = html;",
      "    htmlOutputEl.append(frame);",
      "    return;",
      "  }",
      "  htmlOutputEl.hidden = true;",
      "  htmlOutputEl.innerHTML = '';",
      "  outputEl.hidden = false;",
      "  outputEl.textContent = message;",
      "}",
      "function renderableHtml(message) {",
      "  try {",
      "    const parsed = JSON.parse(message);",
      "    if (parsed && parsed.kind === 'html' && typeof parsed.html === 'string') return parsed.html;",
      "  } catch {",
      "  }",
      "  return null;",
      "}",
      "function renderTrace(trace) {",
      "  if (!traceFeedEl) return;",
      "  traceStatusEl.textContent = 'Live traces';",
      "  const item = document.createElement('article');",
      "  item.className = 'trace-item';",
      "  const alias = trace.agent_alias || trace.agent_id || 'agent';",
      "  const label = trace.node_label || trace.node_id || 'node';",
      "  item.innerHTML = `<div class=\"trace-meta\"><span>${escapeText(alias)}</span><span>${escapeText(label)}</span><span>${escapeText(trace.level || '')}</span></div><pre></pre>`;",
      "  item.querySelector('pre').textContent = trace.message || JSON.stringify(trace.data ?? trace, null, 2);",
      "  traceFeedEl.append(item);",
      "  traceFeedEl.scrollTop = traceFeedEl.scrollHeight;",
      "}",
      "function escapeText(value) {",
      "  return String(value).replace(/[&<>\\\"]/g, (ch) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '\\\"': '&quot;' }[ch]));",
      "}",
      "renderRun(initialResult.workflow_run);",
      "if (!eventsUrl) {",
      "  if (initialResult.queued) statusEl.textContent = 'Queued';",
      "} else {",
      "  const events = new EventSource(publicationEventsUrl(eventsUrl));",
      "  events.addEventListener('queued', () => { statusEl.textContent = 'Queued'; });",
      "  events.addEventListener('status', (event) => renderRun(JSON.parse(event.data).workflow_run));",
      "  events.addEventListener('partial', (event) => { renderPartialOutput(JSON.parse(event.data).message || ''); });",
      "  events.addEventListener('trace', (event) => { renderTrace(JSON.parse(event.data)); });",
      "  events.addEventListener('final', (event) => { renderRun(JSON.parse(event.data).workflow_run); events.close(); });",
      "  events.addEventListener('timeout', () => { statusEl.textContent = 'Still running'; events.close(); });",
      "  events.addEventListener('error', () => { statusEl.textContent = 'Connection interrupted'; });",
      "}",
      "function publicationEventsUrl(path) {",
      "  if (!path || !path.startsWith('/')) return path;",
      "  const match = window.location.pathname.match(/^(\\/display\\/[^/]+)/);",
      "  return match ? `${match[1]}${path}` : path;",
      "}",
      "</script>",
    ].join("\n"),
  )
}

async function streamInvocationEvents(
  reply: HumanHttpReply,
  publication: WorkflowPublicationConfig,
  requestId: string,
) {
  reply.hijack()
  reply.raw.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-cache, no-transform",
    connection: "keep-alive",
  })
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    writeSse(reply, "queued", { invocation_id: requestId })
    const workflowRun = await waitForWorkflowRunByInvocationRequestId(client, publication, requestId, {
      shouldContinue: () => !reply.raw.destroyed,
    })
    if (!workflowRun) {
      writeSse(reply, "timeout", { invocation_id: requestId })
      return
    }
    writeSse(reply, "status", { workflow_run: workflowRun })
    const state: HumanHttpStreamState = { partialIds: new Set(), traces: createPublicationTraceStreamState() }
    emitPartialOutputs(reply, workflowRun, state)
    emitTraceOutputs(reply, publication, workflowRun, state)
    if (isTerminalWorkflowRunStatus(workflowRun.status)) {
      writeSse(reply, "final", { workflow_run: workflowRun })
      return
    }
    await streamWorkflowRunEventsWithClient(reply, publication, workflowRun.id, client, state)
  } catch (error) {
    writeSse(reply, "error", { error: error instanceof Error ? error.message : String(error) })
  } finally {
    await client.close().catch(() => {})
    reply.raw.end()
  }
}

async function streamWorkflowRunEvents(
  reply: HumanHttpReply,
  publication: WorkflowPublicationConfig,
  workflowRunId: string,
) {
  reply.hijack()
  reply.raw.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-cache, no-transform",
    connection: "keep-alive",
  })
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    await streamWorkflowRunEventsWithClient(reply, publication, workflowRunId, client, {
      partialIds: new Set(),
      traces: createPublicationTraceStreamState(),
    })
  } catch (error) {
    writeSse(reply, "error", { error: error instanceof Error ? error.message : String(error) })
  } finally {
    await client.close().catch(() => {})
    reply.raw.end()
  }
}

async function streamWorkflowRunEventsWithClient(
  reply: HumanHttpReply,
  publication: WorkflowPublicationConfig,
  workflowRunId: string,
  client: LocalIpcClient,
  state: HumanHttpStreamState,
) {
  const timeoutMs = publication.sync_timeout_ms ?? 30_000
  const pollMs = publication.poll_ms ?? 500
  const deadline = Date.now() + timeoutMs
  let lastStatus: string | null = null
  while (Date.now() < deadline && !reply.raw.destroyed) {
    await pumpPublicationRuntime(client, publication)
    const response = await client.send<Record<string, unknown>>(
      getWorkflowRunRequest(publication.session_id, workflowRunId),
    )
    const workflowRun = (response.WorkflowRun as { workflow_run?: WorkflowRun } | undefined)?.workflow_run ?? null
    if (workflowRun && workflowRun.status !== lastStatus) {
      lastStatus = workflowRun.status
      writeSse(reply, "status", { workflow_run: workflowRun })
    }
    if (workflowRun) {
      emitPartialOutputs(reply, workflowRun, state)
      emitTraceOutputs(reply, publication, workflowRun, state)
    }
    if (workflowRun && isTerminalWorkflowRunStatus(workflowRun.status)) {
      writeSse(reply, "final", { workflow_run: workflowRun })
      return
    }
    await sleep(pollMs)
  }
  writeSse(reply, "timeout", { workflow_run_id: workflowRunId })
}

function emitTraceOutputs(
  reply: HumanHttpReply,
  publication: WorkflowPublicationConfig,
  workflowRun: WorkflowRun,
  state: HumanHttpStreamState,
) {
  for (const trace of collectPublicationTraceEvents(publication, workflowRun, state.traces)) {
    writeSse(reply, "trace", trace)
  }
}

function emitPartialOutputs(
  reply: HumanHttpReply,
  workflowRun: WorkflowRun,
  state: HumanHttpStreamState,
) {
  for (const output of workflowRun.intermediate_outputs ?? []) {
    if (state.partialIds.has(output.id)) continue
    state.partialIds.add(output.id)
    writeSse(reply, "partial", {
      id: output.id,
      workflow_run_id: workflowRun.id,
      message: output.output.message,
      valid: output.valid,
      warning: output.warning ?? null,
    })
  }
}

function writeSse(reply: HumanHttpReply, event: string, payload: unknown) {
  reply.raw.write(`event: ${event}\n`)
  reply.raw.write(`data: ${JSON.stringify(payload)}\n\n`)
}

function promptTargetParts(route: string) {
  const wildcardIndex = route.indexOf("*")
  if (wildcardIndex >= 0) {
    return {
      prefix: route.slice(0, wildcardIndex),
      suffix: route.slice(wildcardIndex + 1),
    }
  }
  const prefix = route.endsWith("/") ? route : `${route}/`
  return { prefix, suffix: "" }
}

function htmlDocument(title: string, body: string) {
  return [
    "<!doctype html>",
    "<html lang=\"en\">",
    "<head>",
    "  <meta charset=\"utf-8\">",
    "  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    `  <title>${escapeHtml(title)}</title>`,
    "  <style>",
    "    :root { color-scheme: light; }",
    "    * { box-sizing: border-box; }",
    "    body { margin: 0; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif; background: #f5f6f2; color: #1f2328; }",
    "    main { max-width: 780px; margin: 0 auto; padding: 32px 20px; }",
    "    .panel { border: 1px solid #d0d0c8; background: #fff; border-radius: 8px; padding: 20px; }",
    "    .split-viewer { display: grid; grid-template-columns: minmax(0, 1fr) minmax(280px, 34%); gap: 0; max-width: none; width: 100vw; min-height: 100vh; margin: 0; padding: 0; }",
    "    .output-pane, .trace-pane { min-width: 0; min-height: 100vh; padding: 20px; overflow: auto; }",
    "    .output-pane { background: #ffffff; border-right: 1px solid #d7dbd2; }",
    "    .trace-pane { background: #f1f3ed; }",
    "    .pane-header { display: flex; align-items: baseline; justify-content: space-between; gap: 16px; margin-bottom: 14px; }",
    "    h1, h2 { margin: 0; font-size: 18px; line-height: 1.2; letter-spacing: 0; }",
    "    h2 { font-size: 14px; text-transform: uppercase; color: #4d564b; }",
    "    #status, #trace-status { margin: 0; color: #586069; font-size: 13px; }",
    "    textarea { box-sizing: border-box; width: 100%; padding: 12px; border: 1px solid #bbb; border-radius: 6px; font: inherit; resize: vertical; }",
    "    .actions { display: flex; flex-wrap: wrap; gap: 12px; align-items: center; margin-top: 12px; }",
    "    button { padding: 9px 14px; border: 0; border-radius: 6px; background: #1f2328; color: #fff; font: inherit; }",
    "    pre { white-space: pre-wrap; overflow-wrap: anywhere; }",
    "    #output { margin: 0; font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }",
    "    .html-output { width: 100%; height: calc(100vh - 76px); border: 1px solid #d7dbd2; background: #fff; }",
    "    .html-output iframe { display: block; width: 100%; height: 100%; border: 0; background: #fff; }",
    "    #trace-feed { display: flex; flex-direction: column; gap: 10px; }",
    "    .trace-item { border: 1px solid #d7dbd2; background: #fff; border-radius: 6px; padding: 10px; }",
    "    .trace-meta { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 8px; }",
    "    .trace-meta span { border: 1px solid #d7dbd2; border-radius: 999px; padding: 2px 7px; color: #3f4b3c; font-size: 12px; line-height: 1.4; }",
    "    .trace-item pre { margin: 0; font: 12px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; color: #22272e; }",
    "    @media (max-width: 760px) {",
    "      .split-viewer { grid-template-columns: 1fr; }",
    "      .output-pane, .trace-pane { min-height: 50vh; }",
    "      .output-pane { border-right: 0; border-bottom: 1px solid #d7dbd2; }",
    "      .html-output { height: 54vh; }",
    "    }",
    "  </style>",
    "</head>",
    "<body>",
    body,
    "</body>",
    "</html>",
    "",
  ].join("\n")
}

function safeJson(value: unknown) {
  return JSON.stringify(value).replace(/</g, "\\u003c")
}

function escapeHtml(value: string) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\"/g, "&quot;")
}

async function sleep(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}
