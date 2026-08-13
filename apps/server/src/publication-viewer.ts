import type {
  PublicationTraceLevel,
  WorkflowInvocationResult,
  WorkflowPublicationConfig,
} from "./publication-types.js"
import { apiSseInvokePath } from "./publication-api-sse.js"
import {
  collectPublicationTraceEvents,
  createPublicationTraceStreamState,
} from "./publication-trace-events.js"
import { isTerminalWorkflowRunStatus } from "./workflow-run-status.js"
import { websocketInvokePath } from "./publication-websocket.js"

type ViewerApp = {
  get: (path: string, handler: (_request: unknown, reply: ViewerReply) => unknown) => unknown
}

type ViewerReply = {
  code: (code: number) => ViewerReply
  type: (contentType: string) => ViewerReply
}

type ViewerTraceNode = {
  nodeId: string
  nodeLabel: string
  agentAlias: string
  levels: PublicationTraceLevel[]
}

export const PUBLICATION_VIEWER_FORM_INVOKE_PATH = "/.well-known/chariox/publication/human-http/invoke"
export const PUBLICATION_VIEWER_INVOCATION_PATH = "/.well-known/chariox/publication/viewer/invocations"

export function installPublicationViewerRoutes(app: ViewerApp, publication: WorkflowPublicationConfig) {
  app.get("/", async (_request, reply) => {
    if (!viewerTransport(publication)) {
      reply.code(404)
      return { error: "not found" }
    }
    reply.type("text/html; charset=utf-8")
    return publicationViewerPage(publication)
  })
}

export function publicationViewerResultPage(
  publication: WorkflowPublicationConfig,
  result: WorkflowInvocationResult,
  invocationRequestId?: string,
  preserveRequestUrl = false,
  invocationInput?: unknown,
) {
  const workflowRunId = result.workflow_run?.id ?? null
  const terminal = isTerminalWorkflowRunStatus(result.workflow_run?.status ?? "")
  const eventsUrl = invocationRequestId && (result.queued || (workflowRunId && !terminal))
    ? `/.well-known/chariox/publication/invocations/${encodeURIComponent(invocationRequestId)}/events`
    : workflowRunId && !terminal
      ? `/.well-known/chariox/publication/runs/${encodeURIComponent(workflowRunId)}/events`
      : null
  return publicationViewerPage(publication, {
    result,
    eventsUrl,
    invocationRequestId: invocationRequestId ?? null,
    preserveRequestUrl,
    invocationInput,
  })
}

export function publicationViewerPage(
  publication: WorkflowPublicationConfig,
  options: {
    result?: WorkflowInvocationResult
    eventsUrl?: string | null
    invocationRequestId?: string | null
    preserveRequestUrl?: boolean
    invocationInput?: unknown
  } = {},
) {
  const transport = viewerTransport(publication) ?? "human_http"
  const traceNodes = viewerTraceNodes(publication)
  const config = {
    publicationId: publication.publication_id,
    transport,
    title: "Workflow Run",
    showComposer: viewerComposerEnabled(publication),
    initialResult: options.result ?? null,
    invocationRequestId: options.invocationRequestId ?? null,
    permalink: options.invocationRequestId && !options.preserveRequestUrl
      ? `${PUBLICATION_VIEWER_INVOCATION_PATH}/${encodeURIComponent(options.invocationRequestId)}`
      : null,
    initialTraces: options.result?.workflow_run
      ? collectPublicationTraceEvents(publication, options.result.workflow_run, createPublicationTraceStreamState())
      : [],
    optimisticPrompt: publicationInputPrompt(options.invocationInput),
    traceNodes,
    eventsUrl: options.eventsUrl ?? null,
    apiSseInvokePath: apiSseInvokePath(publication),
    websocketInvokePath: websocketInvokePath(publication),
    humanFormInvokePath: PUBLICATION_VIEWER_FORM_INVOKE_PATH,
    humanPromptTarget: promptTargetParts(publication.route ?? "/"),
    directRouteRoots: publicationDirectRouteRoots(publication),
  }
  const hasTraces = traceNodes.length > 0
  const showComposer = viewerComposerEnabled(publication)
  return htmlDocument(
    "Workflow Run",
    [
      `<main class="publication-viewer${hasTraces ? " has-traces" : ""}${showComposer ? " has-composer" : ""}">`,
      "  <section class=\"output-pane\">",
      "    <header class=\"viewer-bar\">",
      "      <div><span class=\"eyebrow\">Published workflow</span><h1>Workflow Run</h1></div>",
      "      <div class=\"run-state\"><span id=\"run-dot\"></span><span id=\"status\">Ready</span></div>",
      "      <p id=\"queue-status\" hidden></p>",
      "    </header>",
      "    <section id=\"output-surface\" class=\"output-surface\" aria-live=\"polite\">",
      "      <div id=\"empty-output\" class=\"empty-state\"><span>Output</span><p>The latest workflow update will appear here.</p></div>",
      "      <pre id=\"output\" hidden></pre>",
      "      <div id=\"html-output\" class=\"html-output\" hidden></div>",
      "    </section>",
      "  </section>",
      hasTraces ? "  <div id=\"rail-resizer\" class=\"rail-resizer\" role=\"separator\" aria-label=\"Resize traces\" aria-orientation=\"vertical\" tabindex=\"0\"></div>" : "",
      hasTraces ? traceRailMarkup(traceNodes) : "",
      showComposer ? composerMarkup(hasTraces) : "",
      "</main>",
      "<script>",
      `window.__charioxPublicationViewerConfig = ${safeJson(config)};`,
      viewerScript(),
      "</script>",
    ].filter(Boolean).join("\n"),
  )
}

export function viewerTraceNodes(publication: WorkflowPublicationConfig): ViewerTraceNode[] {
  const exposure = publication.trace_exposure?.nodes ?? {}
  return Object.entries(exposure)
    .filter(([, levels]) => levels.length > 0)
    .map(([nodeId, levels]) => {
      const context = publication.trace_context?.nodes[nodeId]
      return {
        nodeId,
        nodeLabel: context?.node_label?.trim() || nodeId,
        agentAlias: context?.agent_alias?.trim() || context?.agent_id?.trim() || nodeId,
        levels,
      }
    })
}

export function viewerComposerEnabled(publication: WorkflowPublicationConfig): boolean {
  const transport = viewerTransport(publication)
  if (transport === "api_sse_json" || transport === "websocket_json") return true
  if (transport !== "human_http") return false
  return (publication.methods ?? ["GET", "POST"]).includes("POST")
}

function publicationInputPrompt(input: unknown): string | null {
  if (typeof input === "string") return input.trim() || null
  if (!input || typeof input !== "object" || Array.isArray(input)) return null
  const prompt = (input as Record<string, unknown>).prompt
  return typeof prompt === "string" ? prompt.trim() || null : null
}

function viewerTransport(publication: WorkflowPublicationConfig) {
  const transport = publication.transport ?? "human_http"
  return transport === "human_http" || transport === "api_sse_json" || transport === "websocket_json"
    ? transport
    : null
}

function traceRailMarkup(nodes: ViewerTraceNode[]) {
  return [
    "  <aside id=\"trace-rail\" class=\"trace-rail\">",
    "    <header class=\"trace-bar\"><div><span class=\"eyebrow\">Live detail</span><h2>Traces</h2></div><span id=\"trace-status\">Waiting</span></header>",
    "    <nav id=\"trace-selector\" class=\"trace-selector\" aria-label=\"Trace pane\">",
    ...nodes.map((node, index) => `      <button type="button" data-trace-select="${escapeHtml(node.nodeId)}" aria-pressed="${index === 0 ? "true" : "false"}">${escapeHtml(node.nodeLabel)}</button>`),
    "    </nav>",
    "    <div id=\"trace-grid\" class=\"trace-grid\">",
    ...nodes.map((node, index) => [
      `      <section class="trace-agent-pane${index === 0 ? " is-selected" : ""}" data-trace-node="${escapeHtml(node.nodeId)}">`,
      `        <header><strong>${escapeHtml(node.nodeLabel)}</strong><span>${node.levels.length} ${node.levels.length === 1 ? "trace" : "traces"}</span></header>`,
      "        <div class=\"trace-feed\"><div class=\"trace-empty\">No trace activity yet.</div></div>",
      `        <footer>${escapeHtml(node.agentAlias)}</footer>`,
      "      </section>",
    ].join("\n")),
    "    </div>",
    "  </aside>",
  ].join("\n")
}

function composerMarkup(hasTraces: boolean) {
  return [
    `  <form id="invoke-form" class="invoke-form ${hasTraces ? "composer-under-traces" : "composer-under-output"}">`,
    "    <label for=\"prompt\" class=\"sr-only\">Prompt</label>",
    "    <textarea id=\"prompt\" name=\"prompt\" rows=\"2\" placeholder=\"Send a prompt…\"></textarea>",
    "    <label class=\"attach-button\" title=\"Attach files\"><input type=\"file\" name=\"artifact\" multiple><span aria-hidden=\"true\">＋</span><span class=\"sr-only\">Attach files</span></label>",
    "    <button type=\"submit\" aria-label=\"Send prompt\">↑</button>",
    "  </form>",
  ].join("\n")
}

function viewerScript() {
  return String.raw`
(() => {
const viewerConfig = window.__charioxPublicationViewerConfig || {};
const rootEl = document.querySelector('.publication-viewer');
const formEl = document.querySelector('#invoke-form');
const statusEl = document.querySelector('#status');
const runDotEl = document.querySelector('#run-dot');
const queueStatusEl = document.querySelector('#queue-status');
const outputEl = document.querySelector('#output');
const emptyOutputEl = document.querySelector('#empty-output');
const htmlOutputEl = document.querySelector('#html-output');
const traceRailEl = document.querySelector('#trace-rail');
const traceStatusEl = document.querySelector('#trace-status');
const traceKeys = new Set();
const outputOnlyEmbed = new URLSearchParams(window.location.search).get('chariox_embed') === 'output';

if (outputOnlyEmbed) rootEl?.classList.add('is-output-only');
if (!viewerConfig.showComposer && formEl) formEl.hidden = true;
if (viewerConfig.permalink) {
  const permalink = publicationUrl(viewerConfig.permalink);
  if (window.location.pathname !== permalink) window.history.replaceState(null, '', permalink);
}
setupTraceRail();
setupRailResize();
let latestWorkflowRun = viewerConfig.initialResult?.workflow_run || null;
let latestRunHydrationTimer = null;
renderRun(latestWorkflowRun);
if (latestWorkflowRun && isTerminalStatus(latestWorkflowRun.status)) {
  setTimeout(() => postSettledRun(latestWorkflowRun.status, latestWorkflowRun), 250);
}
renderOptimisticPrompt(viewerConfig.optimisticPrompt);
for (const trace of viewerConfig.initialTraces || []) renderTrace(trace);
if (!latestWorkflowRun) void hydrateLatestRun();
if (viewerConfig.eventsUrl) subscribeHumanHttpEvents(viewerConfig.eventsUrl);
if (viewerConfig.initialResult?.queued) renderQueueStatus(viewerConfig.initialResult.response || viewerConfig.initialResult);

formEl?.addEventListener('submit', async (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  const data = new FormData(form);
  const prompt = String(data.get('prompt') ?? '').trim();
  const files = data.getAll('artifact').filter((item) => item instanceof File && item.size > 0);
  if (!prompt && files.length === 0) return;
  form.reset();
  const artifacts = await Promise.all(files.map(readArtifact));
  await invokePublication(prompt, artifacts);
});

window.addEventListener('message', (event) => {
  if (event.source !== window.parent) return;
  if (event.data?.type === 'chariox:publication:snapshot') {
    if (latestWorkflowRun && isTerminalStatus(latestWorkflowRun.status)) {
      postSettledRun(latestWorkflowRun.status, latestWorkflowRun);
    }
    return;
  }
  if (event.data?.type !== 'chariox:publication:invoke') return;
  const prompt = String(event.data.prompt ?? '').trim();
  const artifacts = Array.isArray(event.data.artifacts) ? event.data.artifacts : [];
  if (!prompt && artifacts.length === 0) return;
  void invokePublication(prompt, artifacts);
});

async function invokePublication(prompt, artifacts) {
  const button = formEl?.querySelector('button[type="submit"]');
  if (button) button.disabled = true;
  setStatus('Submitting');
  try {
    if (viewerConfig.transport === 'human_http') await invokeHumanHttp(prompt, artifacts);
    if (viewerConfig.transport === 'api_sse_json') await invokeApiSse(prompt, artifacts);
    if (viewerConfig.transport === 'websocket_json') await invokeWebSocket(prompt, artifacts);
  } catch (error) {
    setStatus(error instanceof Error ? error.message : String(error), true);
  } finally {
    if (button) button.disabled = false;
  }
}

async function hydrateLatestRun() {
  if (latestRunHydrationTimer) {
    clearTimeout(latestRunHydrationTimer);
    latestRunHydrationTimer = null;
  }
  try {
    const response = await fetch(publicationUrl('/.well-known/chariox/publication/status'), {
      headers: { accept: 'application/json' },
    });
    if (!response.ok) return;
    const status = await response.json();
    if (status.latest_run) renderRun(status.latest_run);
    for (const trace of status.latest_traces || []) renderTrace(trace);
    if (status.latest_run && !isTerminalStatus(status.latest_run.status)) {
      latestRunHydrationTimer = setTimeout(() => void hydrateLatestRun(), 1_000);
    }
  } catch {
    if (latestWorkflowRun && !isTerminalStatus(latestWorkflowRun.status)) {
      latestRunHydrationTimer = setTimeout(() => void hydrateLatestRun(), 1_000);
    }
  }
}

async function invokeHumanHttp(prompt, artifacts) {
  resetForInvocation(prompt);
  const response = await fetch(publicationUrl(viewerConfig.humanFormInvokePath), {
    method: 'POST',
    headers: { accept: 'text/html', 'content-type': 'application/json' },
    body: JSON.stringify({ prompt, artifacts }),
  });
  const html = await response.text();
  const rootUrl = new URL(publicationUrl('/'), window.location.href);
  if (outputOnlyEmbed) rootUrl.searchParams.set('chariox_embed', 'output');
  window.history.replaceState(null, '', rootUrl.pathname + rootUrl.search);
  document.open();
  document.write(html);
  document.close();
}

async function invokeApiSse(prompt, artifacts) {
  resetForInvocation(prompt);
  const response = await fetch(publicationUrl(viewerConfig.apiSseInvokePath), {
    method: 'POST',
    headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
    body: JSON.stringify(inputPayload(prompt, artifacts)),
  });
  if (!response.ok) throw new Error('HTTP ' + response.status);
  if (!response.body) throw new Error('stream unavailable');
  await consumeSseStream(response.body);
}

async function invokeWebSocket(prompt, artifacts) {
  resetForInvocation(prompt);
  const socket = new WebSocket(publicationWebSocketUrl(viewerConfig.websocketInvokePath));
  const readyArtifacts = new Set();
  await new Promise((resolve, reject) => {
    let invoked = false;
    socket.addEventListener('message', (event) => {
      const payload = JSON.parse(String(event.data || '{}'));
      if (payload.type === 'ready') { void sendArtifacts().catch(reject); return; }
      if (payload.type === 'artifact' && payload.artifact?.artifact_id) {
        readyArtifacts.add(payload.artifact.artifact_id);
        if (!invoked && readyArtifacts.size >= artifacts.length) {
          invoked = true;
          socket.send(JSON.stringify({ type: 'invoke', input: { prompt } }));
        }
        return;
      }
      if (payload.type === 'error') reject(new Error(payload.error || 'websocket error'));
      applyPublicationEvent(payload.type, payload);
      if (payload.type === 'final' || payload.type === 'timeout') { socket.close(); resolve(); }
    });
    socket.addEventListener('error', () => reject(new Error('websocket connection failed')));
    socket.addEventListener('close', () => {
      if (!invoked && artifacts.length === 0) reject(new Error('websocket closed before invocation'));
    });
    async function sendArtifacts() {
      if (!artifacts.length) {
        invoked = true;
        socket.send(JSON.stringify({ type: 'invoke', input: inputPayload(prompt, []) }));
        return;
      }
      for (const artifact of artifacts) {
        const artifactId = 'artifact_' + Date.now() + '_' + Math.random().toString(16).slice(2);
        socket.send(JSON.stringify({ type: 'artifact_begin', artifact_id: artifactId, name: artifact.name, mime_type: artifact.type, size_bytes: artifact.size_bytes }));
        socket.send(JSON.stringify({ type: 'artifact_chunk', artifact_id: artifactId, data: artifact.base64 }));
        socket.send(JSON.stringify({ type: 'artifact_end', artifact_id: artifactId }));
      }
    }
  });
}

function inputPayload(prompt, artifacts) {
  return artifacts.length ? { prompt, artifacts } : { prompt };
}

function resetForInvocation(prompt) {
  showEmptyOutput('Waiting for the first workflow update.');
  traceKeys.clear();
  document.querySelectorAll('.trace-feed').forEach((feed) => {
    feed.innerHTML = '<div class="trace-empty">No trace activity yet.</div>';
  });
  if (traceStatusEl) traceStatusEl.textContent = 'Waiting';
  renderOptimisticPrompt(prompt);
  setStatus('Queued');
  renderQueueStatus(null);
}

function renderOptimisticPrompt(prompt) {
  if (!prompt) return;
  for (const node of viewerConfig.traceNodes || []) {
    renderTrace({
      workflow_run_id: 'pending',
      workflow_node_run_id: 'pending:' + node.nodeId,
      node_id: node.nodeId,
      level: 'user_prompt',
      sequence: 0,
      timestamp_ms: Date.now(),
      message: prompt,
      data: { source: 'publication_input' },
    });
  }
}

function subscribeHumanHttpEvents(path) {
  let eventStreamSettled = false;
  let reconnectScheduled = false;
  const events = new EventSource(publicationUrl(path));
  const reconnect = () => {
    if (eventStreamSettled || reconnectScheduled) return;
    reconnectScheduled = true;
    events.close();
    setTimeout(() => subscribeHumanHttpEvents(path), 1_000);
  };
  events.addEventListener('queued', (event) => applyPublicationEvent('queued', parseEventData(event)));
  events.addEventListener('status', (event) => applyPublicationEvent('status', parseEventData(event)));
  events.addEventListener('started', (event) => applyPublicationEvent('started', parseEventData(event)));
  events.addEventListener('partial', (event) => applyPublicationEvent('partial', parseEventData(event)));
  events.addEventListener('trace', (event) => applyPublicationEvent('trace', parseEventData(event)));
  events.addEventListener('final', (event) => { eventStreamSettled = true; applyPublicationEvent('final', parseEventData(event)); events.close(); });
  events.addEventListener('timeout', (event) => { applyPublicationEvent('timeout', parseEventData(event)); reconnect(); });
  events.addEventListener('error', () => {
    if (!eventStreamSettled) setStatus('Still running · reconnecting');
    reconnect();
  });
}

async function consumeSseStream(body) {
  const reader = body.pipeThrough(new TextDecoderStream()).getReader();
  let buffer = '';
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += value;
    let index;
    while ((index = buffer.indexOf('\n\n')) >= 0) {
      const frame = buffer.slice(0, index);
      buffer = buffer.slice(index + 2);
      const parsed = parseSseFrame(frame);
      if (parsed) applyPublicationEvent(parsed.event, parsed.data);
    }
  }
}

function parseSseFrame(frame) {
  let event = 'message';
  const data = [];
  for (const line of frame.split(/\r?\n/)) {
    if (line.startsWith('event:')) event = line.slice(6).trim();
    if (line.startsWith('data:')) data.push(line.slice(5).trimStart());
  }
  if (!data.length) return null;
  return { event, data: JSON.parse(data.join('\n')) };
}

function parseEventData(event) {
  return JSON.parse(event.data || '{}');
}

function applyPublicationEvent(type, payload) {
  if (type === 'queued') renderQueueStatus(payload);
  if (type === 'started' || type === 'status') {
    renderRun(payload.workflow_run);
    if (type === 'started' && !isTerminalStatus(payload.workflow_run?.status)) {
      setStatus('Running', false, payload.workflow_run);
    }
  }
  if (type === 'partial') renderOutput(payload.message ?? '', 'progress');
  if (type === 'trace') renderTrace(payload);
  if (type === 'final') {
    const finalMessage = outputMessage(payload.workflow_run?.final_output);
    if (finalMessage !== null) renderOutput(finalMessage, 'final');
    else if (payload.message !== undefined) renderOutput(payload.message, 'final');
    else renderRun(payload.workflow_run);
    setStatus(payload.workflow_run?.status || 'Completed', false, payload.workflow_run);
  }
  if (type === 'timeout') setStatus('Still running', true);
}

function renderRun(run) {
  if (!run) return false;
  latestWorkflowRun = run;
  setStatus(run.status || 'Accepted', false, run);
  if (queueStatusEl) { queueStatusEl.hidden = true; queueStatusEl.textContent = ''; }
  const finalMessage = outputMessage(run.final_output);
  if (finalMessage !== null) {
    renderOutput(finalMessage, 'final');
    return true;
  }
  const intermediate = Array.isArray(run.intermediate_outputs) ? run.intermediate_outputs.at(-1) : null;
  const intermediateMessage = outputMessage(intermediate?.output);
  if (intermediateMessage !== null) {
    renderOutput(intermediateMessage, 'progress');
    return true;
  }
  return false;
}

function outputMessage(output) {
  if (typeof output === 'string') return output;
  if (!output || typeof output !== 'object') return null;
  if (typeof output.message === 'string') return output.message;
  if (output.output && typeof output.output === 'object') return outputMessage(output.output);
  try { return JSON.stringify(output); } catch { return String(output); }
}

function renderQueueStatus(payload) {
  setStatus('Queued');
  if (!queueStatusEl) return;
  const position = payload?.queue_position ?? payload?.position ?? payload?.queued_prompt?.position ?? payload?.response?.queued_prompt?.position;
  const details = [];
  if (typeof position === 'number') details.push('position ' + position);
  if (payload?.queue_ref) details.push(payload.queue_ref);
  queueStatusEl.hidden = details.length === 0;
  queueStatusEl.textContent = details.length ? 'Queued · ' + details.join(' · ') : '';
}

function setStatus(status, warning = false, workflowRun = null) {
  if (statusEl) statusEl.textContent = status;
  const normalized = String(status || '').toLowerCase();
  const terminal = isTerminalStatus(normalized);
  if (runDotEl) runDotEl.dataset.state = warning ? 'warning' : terminal ? 'terminal' : normalized === 'ready' ? 'ready' : 'active';
  if (terminal) postSettledRun(status, workflowRun);
}

function isTerminalStatus(status) {
  const normalized = String(status || '').toLowerCase();
  return ['completed', 'complete', 'done', 'failed', 'cancelled', 'canceled'].some((value) => normalized.includes(value));
}

function postSettledRun(status, workflowRun) {
  if (window.parent === window) return;
  window.parent.postMessage({
    type: 'chariox:publication:settled',
    publicationId: viewerConfig.publicationId,
    status: String(status || ''),
    workflowRun,
  }, '*');
}

function showEmptyOutput(message) {
  htmlOutputEl.hidden = true;
  htmlOutputEl.replaceChildren();
  outputEl.hidden = true;
  outputEl.textContent = '';
  emptyOutputEl.hidden = false;
  emptyOutputEl.querySelector('p').textContent = message;
}

function renderOutput(message, phase) {
  const normalizedMessage = normalizeViewerMessage(message);
  emptyOutputEl.hidden = true;
  htmlOutputEl.hidden = true;
  htmlOutputEl.replaceChildren();
  outputEl.hidden = true;
  outputEl.textContent = '';
  const renderable = renderableOutput(normalizedMessage);
  if (renderable) {
    htmlOutputEl.hidden = false;
    const frame = document.createElement('iframe');
    frame.title = phase === 'final' ? 'Workflow result' : 'Workflow progress';
    frame.setAttribute('sandbox', 'allow-scripts allow-forms allow-popups allow-modals allow-downloads');
    frame.setAttribute('referrerpolicy', 'no-referrer');
    if (renderable.html !== null) frame.srcdoc = renderable.html;
    if (renderable.src !== null) frame.src = publicationAppAssetUrl(renderable.src);
    htmlOutputEl.append(frame);
    return;
  }
  outputEl.hidden = false;
  outputEl.textContent = typeof normalizedMessage === 'string' ? normalizedMessage : JSON.stringify(normalizedMessage, null, 2);
}

function normalizeViewerMessage(message) {
  if (typeof message !== 'string') return outputMessage(message) ?? message;
  try {
    const parsed = JSON.parse(message);
    if (parsed && (parsed.kind === 'html' || parsed.kind === 'response')) return message;
    if (parsed && parsed.output && typeof parsed.output === 'object') {
      const nested = outputMessage(parsed.output);
      return nested === null ? message : normalizeViewerMessage(nested);
    }
    if (parsed && typeof parsed.message === 'string') return normalizeViewerMessage(parsed.message);
  } catch {}
  return message;
}

function renderableOutput(message) {
  if (typeof message !== 'string') return null;
  try {
    const parsed = JSON.parse(message);
    if (parsed && parsed.kind === 'html' && typeof parsed.html === 'string') return { html: parsed.html, src: null };
    if (parsed && parsed.kind === 'response' && parsed.response) {
      const mode = parsed.response.mode;
      if (mode === 'html') {
        const html = typeof parsed.response.html === 'string'
          ? parsed.response.html
          : typeof parsed.response.body === 'string'
            ? parsed.response.body
            : typeof parsed.html === 'string' ? parsed.html : null;
        if (html !== null) return { html, src: null };
      }
      if (mode === 'serve' && typeof parsed.response.entry === 'string') return { html: null, src: parsed.response.entry };
    }
  } catch {}
  return null;
}

function setupTraceRail() {
  document.querySelectorAll('[data-trace-select]').forEach((button) => {
    button.addEventListener('click', () => selectTraceNode(button.dataset.traceSelect));
  });
  const observer = traceRailEl && typeof ResizeObserver === 'function'
    ? new ResizeObserver(([entry]) => {
        traceRailEl.classList.toggle('is-narrow', entry.contentRect.width < 560);
      })
    : null;
  if (traceRailEl) observer?.observe(traceRailEl);
}

function selectTraceNode(nodeId) {
  document.querySelectorAll('[data-trace-select]').forEach((button) => {
    button.setAttribute('aria-pressed', String(button.dataset.traceSelect === nodeId));
  });
  document.querySelectorAll('[data-trace-node]').forEach((pane) => {
    pane.classList.toggle('is-selected', pane.dataset.traceNode === nodeId);
  });
}

function setupRailResize() {
  const separator = document.querySelector('#rail-resizer');
  if (!separator || !rootEl) return;
  const setWidth = (width) => {
    const bounds = rootEl.getBoundingClientRect();
    const separatorWidth = separator.getBoundingClientRect().width;
    const next = Math.max(300, Math.min(bounds.width - separatorWidth - 360, width));
    rootEl.style.setProperty('--trace-width', next + 'px');
    separator.setAttribute('aria-valuenow', String(Math.round(next)));
  };
  separator.addEventListener('pointerdown', (event) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = traceRailEl?.getBoundingClientRect().width || 480;
    document.body.classList.add('is-resizing-rail');
    const move = (moveEvent) => setWidth(startWidth + startX - moveEvent.clientX);
    const done = () => {
      document.body.classList.remove('is-resizing-rail');
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', done);
      window.removeEventListener('pointercancel', done);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', done);
    window.addEventListener('pointercancel', done);
  });
  separator.addEventListener('keydown', (event) => {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    const current = traceRailEl?.getBoundingClientRect().width || 480;
    setWidth(current + (event.key === 'ArrowLeft' ? 32 : -32));
  });
}

function renderTrace(trace) {
  const nodeId = String(trace.node_id || '');
  const pane = document.querySelector('[data-trace-node="' + cssEscape(nodeId) + '"]');
  if (!pane) return;
  const feed = pane.querySelector('.trace-feed');
  const workflowRunId = String(trace.workflow_run_id || 'pending');
  const key = trace.level === 'user_prompt'
    ? ['user_prompt', nodeId, workflowRunId, trace.message].join(':')
    : [trace.workflow_run_id, trace.workflow_node_run_id, trace.level, trace.timestamp_ms, trace.message].join(':');
  if (trace.level === 'user_prompt' && workflowRunId !== 'pending') {
    const pendingKey = ['user_prompt', nodeId, 'pending', trace.message].join(':');
    const pendingItem = Array.from(feed.querySelectorAll('.trace-item'))
      .find((item) => item.dataset.traceKey === pendingKey);
    if (pendingItem) {
      traceKeys.delete(pendingKey);
      traceKeys.add(key);
      pendingItem.dataset.traceKey = key;
      pendingItem.dataset.traceRun = String(trace.workflow_node_run_id || workflowRunId);
      pendingItem.dataset.traceTimestamp = String(Number(trace.timestamp_ms) || 0);
      reorderTraceFeed(feed);
      return;
    }
  }
  if (traceKeys.has(key)) return;
  traceKeys.add(key);
  if (traceStatusEl) traceStatusEl.textContent = 'Live';
  feed.querySelector('.trace-empty')?.remove();
  const item = document.createElement('article');
  item.className = 'trace-item trace-' + String(trace.level || 'event').replace(/[^a-z0-9_-]/gi, '-');
  item.dataset.traceKey = key;
  item.dataset.traceLevel = String(trace.level || 'event');
  item.dataset.traceRun = String(trace.workflow_node_run_id || workflowRunId);
  item.dataset.traceTimestamp = String(Number(trace.timestamp_ms) || 0);
  const label = traceLevelLabel(trace.level);
  const meta = document.createElement('div');
  meta.className = 'trace-meta';
  const title = document.createElement('strong');
  title.textContent = label;
  const timestamp = document.createElement('time');
  const time = Number(trace.timestamp_ms);
  timestamp.textContent = Number.isFinite(time) ? new Date(time).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '';
  meta.append(title, timestamp);
  item.append(meta);
  renderTraceContent(item, trace);
  feed.append(item);
  reorderTraceFeed(feed);
  feed.scrollTop = feed.scrollHeight;
}

function reorderTraceFeed(feed) {
  const items = Array.from(feed.querySelectorAll('.trace-item'));
  const firstTimestampByRun = new Map();
  for (const item of items) {
    const run = item.dataset.traceRun || '';
    const timestamp = Number(item.dataset.traceTimestamp) || 0;
    firstTimestampByRun.set(run, Math.min(firstTimestampByRun.get(run) ?? timestamp, timestamp));
  }
  items.sort((left, right) => {
    const leftLevel = left.dataset.traceLevel || '';
    const rightLevel = right.dataset.traceLevel || '';
    const leftRun = left.dataset.traceRun || '';
    const rightRun = right.dataset.traceRun || '';
    if (leftRun !== rightRun) {
      return (firstTimestampByRun.get(leftRun) ?? 0) - (firstTimestampByRun.get(rightRun) ?? 0);
    }
    if (leftLevel === 'user_prompt' || rightLevel === 'user_prompt') {
      return leftLevel === rightLevel ? 0 : leftLevel === 'user_prompt' ? -1 : 1;
    }
    const leftSummary = leftLevel === 'output_summary';
    const rightSummary = rightLevel === 'output_summary';
    if (leftSummary !== rightSummary) return leftSummary ? 1 : -1;
    return Number(left.dataset.traceTimestamp || 0) - Number(right.dataset.traceTimestamp || 0);
  });
  for (const item of items) feed.append(item);
}

function renderTraceContent(item, trace) {
  const message = traceDisplayMessage(trace);
  if (trace.level === 'tool_use') {
    const code = document.createElement('pre');
    code.className = 'trace-code';
    code.textContent = message;
    item.append(code);
    return;
  }
  const prose = document.createElement('div');
  prose.className = 'trace-prose';
  renderTraceProse(prose, message);
  item.append(prose);
}

function traceDisplayMessage(trace) {
  const fallback = trace.message || JSON.stringify(trace.data ?? trace, null, 2);
  if (trace.level === 'tool_use' || typeof fallback !== 'string') return String(fallback ?? '');
  return naturalTraceMessage(fallback);
}

function naturalTraceMessage(message) {
  const trimmed = message.trim();
  if (!trimmed) return '';
  try {
    const parsed = JSON.parse(trimmed);
    const natural = naturalStructuredMessage(parsed);
    return natural || 'Produced a structured workflow update.';
  } catch {}
  return message;
}

function naturalStructuredMessage(value) {
  if (typeof value === 'string') return naturalTraceMessage(value);
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  if (value.kind === 'html' && typeof value.html === 'string') return 'Generated an interactive HTML workflow update.';
  if (value.kind === 'response' && value.response?.mode === 'html') return 'Generated an interactive HTML workflow update.';
  for (const key of ['summary', 'text', 'content', 'message']) {
    if (typeof value[key] === 'string') return naturalTraceMessage(value[key]);
  }
  if (value.output && typeof value.output === 'object') return naturalStructuredMessage(value.output);
  return null;
}

function renderTraceProse(container, message) {
  const lines = String(message || '').replace(/\r\n/g, '\n').split('\n');
  let paragraph = [];
  let list = null;
  let code = null;
  const flushParagraph = () => {
    if (!paragraph.length) return;
    const element = document.createElement('p');
    appendInlineTraceFormatting(element, paragraph.join(' '));
    container.append(element);
    paragraph = [];
  };
  const flushList = () => { list = null; };
  for (const line of lines) {
    if (line.trim().startsWith(String.fromCharCode(96).repeat(3))) {
      flushParagraph(); flushList();
      if (code) { container.append(code); code = null; }
      else { code = document.createElement('pre'); code.className = 'trace-code'; }
      continue;
    }
    if (code) { code.textContent += (code.textContent ? '\n' : '') + line; continue; }
    const heading = line.match(/^(#{1,3})\s+(.+)$/);
    if (heading) {
      flushParagraph(); flushList();
      const element = document.createElement('h' + Math.min(heading[1].length + 2, 5));
      appendInlineTraceFormatting(element, heading[2]);
      container.append(element);
      continue;
    }
    const bullet = line.match(/^\s*[-*]\s+(.+)$/);
    if (bullet) {
      flushParagraph();
      if (!list || list.tagName !== 'UL') { list = document.createElement('ul'); container.append(list); }
      const item = document.createElement('li');
      appendInlineTraceFormatting(item, bullet[1]);
      list.append(item);
      continue;
    }
    const ordered = line.match(/^\s*\d+[.)]\s+(.+)$/);
    if (ordered) {
      flushParagraph();
      if (!list || list.tagName !== 'OL') { list = document.createElement('ol'); container.append(list); }
      const item = document.createElement('li');
      appendInlineTraceFormatting(item, ordered[1]);
      list.append(item);
      continue;
    }
    if (!line.trim()) { flushParagraph(); flushList(); continue; }
    flushList();
    paragraph.push(line.trim());
  }
  flushParagraph();
  if (code) container.append(code);
}

function appendInlineTraceFormatting(container, text) {
  const pattern = /(\x60[^\x60]+\x60|\*\*[^*]+\*\*|_[^_]+_)/g;
  let cursor = 0;
  for (const match of text.matchAll(pattern)) {
    if (match.index > cursor) container.append(document.createTextNode(text.slice(cursor, match.index)));
    const token = match[0];
    const element = document.createElement(token.charCodeAt(0) === 96 ? 'code' : token.startsWith('**') ? 'strong' : 'em');
    element.textContent = token.startsWith('**') ? token.slice(2, -2) : token.slice(1, -1);
    container.append(element);
    cursor = match.index + token.length;
  }
  if (cursor < text.length) container.append(document.createTextNode(text.slice(cursor)));
}

function traceLevelLabel(level) {
  return ({ user_prompt: 'Prompt', output_summary: 'Summary', assistant_messages: 'Assistant', thinking: 'Thinking', tool_use: 'Tool call' })[level] || 'Trace';
}

function cssEscape(value) {
  return window.CSS?.escape ? window.CSS.escape(value) : value.replace(/["\\]/g, '\\$&');
}

async function readArtifact(file) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = '';
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) binary += String.fromCharCode(...bytes.slice(index, index + chunkSize));
  const base64 = btoa(binary);
  return { name: file.name, type: file.type || 'application/octet-stream', size_bytes: file.size, data_url: 'data:' + (file.type || 'application/octet-stream') + ';base64,' + base64, base64 };
}

function publicationUrl(path) {
  if (!path || !path.startsWith('/')) return path;
  const match = window.location.pathname.match(/^(\/display\/[^/]+)/);
  if (match) return match[1] + path;
  const prefix = publicationIngressPrefix();
  return prefix ? prefix + path : path;
}

function publicationAppAssetUrl(path) {
  const base = publicationUrl(path);
  if (!viewerConfig.invocationRequestId || !base || !base.startsWith('/')) return base;
  const [pathname, query = ''] = base.split('?');
  const params = new URLSearchParams(query);
  params.set('chariox_invocation', viewerConfig.invocationRequestId);
  const serialized = params.toString();
  return serialized ? pathname + '?' + serialized : pathname;
}

function publicationWebSocketUrl(path) {
  const url = new URL(publicationUrl(path), window.location.href);
  url.protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return url.toString();
}

function publicationIngressPrefix() {
  const parts = window.location.pathname.split('/').filter(Boolean);
  if (!parts.length) return '';
  if (parts[0] === '~d' && parts[1] && parts[2]) return '/' + parts.slice(0, 3).join('/');
  if (parts[0] === 'publication-ingress' && parts[1] === '~d' && parts[2] && parts[3]) return '/' + parts.slice(0, 4).join('/');
  if (parts[0] === 'publication-ingress' && parts[1]) return '/' + parts.slice(0, 2).join('/');
  if (['.well-known', 'invoke', 'mcp', 'health'].includes(parts[0])) return '';
  const directRouteRoots = Array.isArray(viewerConfig.directRouteRoots) ? viewerConfig.directRouteRoots : [];
  if (directRouteRoots.includes('*') || directRouteRoots.includes(parts[0])) return '';
  const routeFirst = String(viewerConfig.humanPromptTarget?.prefix || '').split('/').filter(Boolean)[0] || '';
  if (routeFirst && parts[0] === routeFirst) return '';
  return '/' + parts[0];
}

})();
`
}

function publicationDirectRouteRoots(publication: WorkflowPublicationConfig): string[] {
  const roots = [
    publication.route,
    ...(publication.agent_app?.routes ?? []).map((route) => route.path),
  ]
    .map((route) => String(route ?? "").split("/").filter(Boolean)[0])
    .filter((root): root is string => Boolean(root))
  return [...new Set(roots)]
}

function promptTargetParts(route: string) {
  const wildcardIndex = route.indexOf("*")
  if (wildcardIndex >= 0) return { prefix: route.slice(0, wildcardIndex), suffix: route.slice(wildcardIndex + 1) }
  const parameter = route.match(/:[A-Za-z_][A-Za-z0-9_]*/)
  if (parameter?.index !== undefined) {
    return { prefix: route.slice(0, parameter.index), suffix: route.slice(parameter.index + parameter[0].length) }
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
    "  <link rel=\"icon\" href=\"data:,\">",
    `  <title>${escapeHtml(title)}</title>`,
    "  <style>",
    "    :root { color-scheme: dark; --bg: #0c0e0d; --panel: #111411; --panel-2: #171a17; --line: #2b302b; --muted: #8d958b; --text: #ecf0e9; --accent: #f39a62; --green: #84d497; --trace-width: min(44vw, 720px); }",
    "    * { box-sizing: border-box; }",
    "    [hidden] { display: none !important; }",
    "    html, body { height: 100%; }",
    "    body { margin: 0; overflow: hidden; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; background: var(--bg); color: var(--text); }",
    "    button, textarea, input { font: inherit; }",
    "    button { cursor: pointer; }",
    "    .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0,0,0,0); white-space: nowrap; border: 0; }",
    "    .publication-viewer { display: grid; grid-template: minmax(0,1fr) / minmax(0,1fr); width: 100vw; height: 100vh; background: var(--bg); }",
    "    .publication-viewer.has-traces { grid-template-columns: minmax(360px,1fr) 7px minmax(300px,var(--trace-width)); }",
    "    .publication-viewer.has-traces.has-composer { grid-template-rows: minmax(0,1fr) auto; }",
    "    .publication-viewer.is-output-only, .publication-viewer.is-output-only.has-traces, .publication-viewer.is-output-only.has-traces.has-composer { grid-template: minmax(0,1fr) / minmax(0,1fr); }",
    "    .publication-viewer.is-output-only .output-pane { grid-column: 1; grid-row: 1; }",
    "    .publication-viewer.is-output-only .rail-resizer, .publication-viewer.is-output-only .trace-rail, .publication-viewer.is-output-only .invoke-form { display: none; }",
    "    .output-pane { grid-column: 1; grid-row: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: #080a09; }",
    "    .viewer-bar, .trace-bar { min-height: 58px; padding: 11px 16px; border-bottom: 1px solid var(--line); display: flex; align-items: center; justify-content: space-between; gap: 14px; background: color-mix(in srgb, var(--panel) 92%, transparent); }",
    "    .viewer-bar > div:first-child, .trace-bar > div:first-child { display: flex; align-items: baseline; gap: 11px; min-width: 0; }",
    "    .eyebrow { color: var(--accent); font-size: 10px; text-transform: uppercase; letter-spacing: .14em; white-space: nowrap; }",
    "    h1, h2 { margin: 0; font-size: 14px; line-height: 1.2; }",
    "    .run-state { display: flex; align-items: center; gap: 8px; color: var(--muted); font-size: 12px; }",
    "    #run-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--muted); box-shadow: 0 0 0 3px #252925; }",
    "    #run-dot[data-state=active] { background: var(--accent); box-shadow: 0 0 0 3px #4a2c1e; animation: pulse 1.6s ease-in-out infinite; }",
    "    #run-dot[data-state=terminal], #run-dot[data-state=ready] { background: var(--green); box-shadow: 0 0 0 3px #1d3a25; }",
    "    #run-dot[data-state=warning] { background: #f16d6d; box-shadow: 0 0 0 3px #431e1e; }",
    "    #queue-status { position: absolute; top: 58px; left: 16px; z-index: 2; margin: 0; padding: 5px 8px; background: #211b16; border: 1px solid #503520; color: #eeb185; font-size: 11px; }",
    "    .output-surface { min-height: 0; flex: 1; position: relative; overflow: auto; }",
    "    .empty-state { position: absolute; inset: 0; display: grid; place-content: center; text-align: center; color: var(--muted); }",
    "    .empty-state span { color: #c8cec5; font-size: 13px; }",
    "    .empty-state p { margin: 7px 0 0; max-width: 320px; font: 12px/1.5 ui-sans-serif, system-ui, sans-serif; }",
    "    #output { margin: 0; min-height: 100%; padding: 22px; font: 14px/1.58 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; white-space: pre-wrap; overflow-wrap: anywhere; color: #e4e9e0; }",
    "    .html-output { width: 100%; height: 100%; min-height: 0; background: #fff; }",
    "    .html-output iframe { display: block; width: 100%; height: 100%; border: 0; background: #fff; }",
    "    .rail-resizer { grid-column: 2; grid-row: 1 / -1; position: relative; background: var(--line); cursor: col-resize; touch-action: none; z-index: 3; }",
    "    .rail-resizer::after { content: ''; position: absolute; inset: 0 -4px; }",
    "    .rail-resizer:focus-visible, .rail-resizer:hover { background: var(--accent); outline: none; }",
    "    body.is-resizing-rail { cursor: col-resize; user-select: none; }",
    "    body.is-resizing-rail iframe { pointer-events: none; }",
    "    .trace-rail { grid-column: 3; grid-row: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--panel); }",
    "    .trace-bar { flex: 0 0 auto; }",
    "    #trace-status { color: var(--green); font-size: 11px; text-transform: uppercase; letter-spacing: .08em; }",
    "    .trace-selector { display: none; gap: 5px; padding: 7px; border-bottom: 1px solid var(--line); overflow-x: auto; }",
    "    .trace-selector button { border: 1px solid var(--line); border-radius: 3px; padding: 6px 9px; background: #101310; color: var(--muted); font-size: 11px; white-space: nowrap; }",
    "    .trace-selector button[aria-pressed=true] { border-color: var(--accent); color: var(--text); background: #261b15; }",
    "    .trace-grid { min-height: 0; flex: 1; display: grid; grid-template-columns: repeat(2,minmax(250px,1fr)); gap: 7px; padding: 7px; overflow: auto; }",
    "    .trace-agent-pane { min-height: 270px; display: grid; grid-template-rows: auto minmax(0,1fr) auto; border: 1px solid var(--line); background: #0d100e; overflow: hidden; }",
    "    .trace-agent-pane:only-child { grid-column: 1 / -1; }",
    "    .trace-agent-pane > header, .trace-agent-pane > footer { min-height: 35px; padding: 8px 10px; display: flex; align-items: center; justify-content: space-between; gap: 9px; background: var(--panel-2); }",
    "    .trace-agent-pane > header { border-bottom: 1px solid var(--line); font-size: 11px; }",
    "    .trace-agent-pane > header span { color: var(--muted); font-size: 10px; }",
    "    .trace-agent-pane > footer { border-top: 1px solid var(--line); color: var(--muted); font-size: 11px; }",
    "    .trace-agent-pane > footer::before { content: ''; width: 6px; height: 6px; margin-right: 7px; border-radius: 50%; background: var(--green); }",
    "    .trace-feed { min-height: 0; overflow: auto; padding: 8px; display: flex; flex-direction: column; gap: 7px; }",
    "    .trace-empty { margin: auto; color: #666e65; font-size: 11px; }",
    "    .trace-item { border: 1px solid #282d28; background: #141714; }",
    "    .trace-meta { padding: 6px 8px; border-bottom: 1px solid #282d28; display: flex; justify-content: space-between; gap: 10px; color: var(--muted); font-size: 10px; text-transform: uppercase; letter-spacing: .06em; }",
    "    .trace-meta strong { color: #c3c9c0; }",
    "    .trace-user_prompt { border-color: #3b403b; background: #191c19; }",
    "    .trace-user_prompt .trace-meta strong { color: var(--text); }",
    "    .trace-tool_use .trace-meta strong { color: var(--accent); }",
    "    .trace-output_summary .trace-meta strong { color: var(--green); }",
    "    .trace-prose { padding: 10px; font: 12px/1.55 ui-sans-serif, system-ui, sans-serif; color: #dfe4dc; overflow-wrap: anywhere; }",
    "    .trace-prose p { margin: 0 0 8px; }",
    "    .trace-prose p:last-child { margin-bottom: 0; }",
    "    .trace-prose h3, .trace-prose h4, .trace-prose h5 { margin: 2px 0 8px; font: 600 12px/1.35 ui-sans-serif, system-ui, sans-serif; color: var(--text); }",
    "    .trace-prose ul, .trace-prose ol { margin: 0 0 8px; padding-left: 20px; }",
    "    .trace-prose li + li { margin-top: 4px; }",
    "    .trace-prose code { padding: 1px 4px; border: 1px solid #303630; border-radius: 3px; background: #0b0e0c; font: 11px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; color: #f0c2a5; }",
    "    .trace-code { margin: 0; padding: 9px; background: #0b0e0c; font: 11px/1.48 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; white-space: pre-wrap; overflow-wrap: anywhere; color: #d9ded6; }",
    "    .invoke-form { min-width: 0; display: grid; grid-template-columns: minmax(0,1fr) auto auto; gap: 7px; padding: 9px; border-top: 1px solid var(--line); background: #121512; }",
    "    .composer-under-traces { grid-column: 3; grid-row: 2; }",
    "    .composer-under-output { grid-column: 1; grid-row: 2; }",
    "    .publication-viewer:not(.has-traces).has-composer { grid-template-rows: minmax(0,1fr) auto; }",
    "    textarea { min-height: 44px; max-height: 140px; resize: vertical; padding: 11px 12px; border: 1px solid #343a34; border-radius: 4px; outline: none; background: #0b0e0c; color: var(--text); }",
    "    textarea:focus { border-color: var(--accent); }",
    "    .attach-button, .invoke-form button { width: 44px; height: 44px; display: grid; place-items: center; border: 1px solid #343a34; border-radius: 4px; background: #171b17; color: var(--text); }",
    "    .attach-button input { display: none; }",
    "    .attach-button:hover { border-color: #626962; }",
    "    .invoke-form button { border-color: #a55d37; background: #a95f39; font-size: 22px; }",
    "    .invoke-form button:disabled { opacity: .45; }",
    "    .trace-rail.is-narrow .trace-selector { display: flex; }",
    "    .trace-rail.is-narrow .trace-grid { display: block; overflow: hidden; }",
    "    .trace-rail.is-narrow .trace-agent-pane { display: none; height: 100%; min-height: 0; }",
    "    .trace-rail.is-narrow .trace-agent-pane.is-selected { display: grid; }",
    "    @keyframes pulse { 50% { opacity: .45; } }",
    "    @media (max-width: 760px) {",
    "      body { overflow: auto; }",
    "      .publication-viewer, .publication-viewer.has-traces, .publication-viewer.has-traces.has-composer { height: auto; min-height: 100vh; grid-template-columns: 1fr; grid-template-rows: minmax(56vh,1fr) minmax(380px,44vh) auto; }",
    "      .output-pane { grid-column: 1; grid-row: 1; min-height: 56vh; }",
    "      .rail-resizer { display: none; }",
    "      .trace-rail { grid-column: 1; grid-row: 2; min-height: 380px; border-top: 1px solid var(--line); }",
    "      .composer-under-traces, .composer-under-output { grid-column: 1; grid-row: 3; position: sticky; bottom: 0; }",
    "      .trace-selector { display: flex; }",
    "      .trace-grid { display: block; overflow: hidden; }",
    "      .trace-agent-pane { display: none; height: 100%; min-height: 0; }",
    "      .trace-agent-pane.is-selected { display: grid; }",
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
