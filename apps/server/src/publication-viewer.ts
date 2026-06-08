import type {
  WorkflowInvocationResult,
  WorkflowPublicationConfig,
} from "./publication-types.js"
import {
  collectPublicationTraceEvents,
  createPublicationTraceStreamState,
} from "./publication-trace-events.js"

type ViewerApp = {
  get: (path: string, handler: (_request: unknown, reply: ViewerReply) => unknown) => unknown
}

type ViewerReply = {
  code: (code: number) => ViewerReply
  type: (contentType: string) => ViewerReply
}

export const PUBLICATION_VIEWER_FORM_INVOKE_PATH = "/.well-known/arroba/publication/human-http/invoke"
const API_SSE_INVOKE_PATH = "/invoke"
const WEBSOCKET_INVOKE_PATH = "/.well-known/arroba/publication/ws"

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
) {
  const workflowRunId = result.workflow_run?.id ?? null
  const eventsUrl = workflowRunId
    ? `/.well-known/arroba/publication/runs/${encodeURIComponent(workflowRunId)}/events`
    : result.queued && invocationRequestId
      ? `/.well-known/arroba/publication/invocations/${encodeURIComponent(invocationRequestId)}/events`
      : null
  return publicationViewerPage(publication, { result, eventsUrl })
}

export function publicationViewerPage(
  publication: WorkflowPublicationConfig,
  options: {
    result?: WorkflowInvocationResult
    eventsUrl?: string | null
  } = {},
) {
  const transport = viewerTransport(publication) ?? "human_http"
  const hasInitialRun = Boolean(options.result?.workflow_run || options.eventsUrl)
  const config = {
    transport,
    title: "Workflow Run",
    showForm: !hasInitialRun,
    initialResult: options.result ?? null,
    initialTraces: options.result?.workflow_run
      ? collectPublicationTraceEvents(publication, options.result.workflow_run, createPublicationTraceStreamState())
      : [],
    eventsUrl: options.eventsUrl ?? null,
    apiSseInvokePath: API_SSE_INVOKE_PATH,
    websocketInvokePath: WEBSOCKET_INVOKE_PATH,
    humanFormInvokePath: PUBLICATION_VIEWER_FORM_INVOKE_PATH,
    humanPromptTarget: promptTargetParts(publication.route ?? "/*"),
  }
  return htmlDocument(
    "Workflow Run",
    [
      "<main class=\"split-viewer\">",
      "  <section class=\"output-pane\">",
      "    <header class=\"pane-header\">",
      "      <h1>Workflow Run</h1>",
      "      <p id=\"status\">Ready</p>",
      "    </header>",
      "    <form id=\"invoke-form\" class=\"invoke-form\">",
      "      <textarea name=\"prompt\" rows=\"7\" autofocus></textarea>",
      "      <div class=\"actions\">",
      "        <input type=\"file\" name=\"artifact\" multiple>",
      "        <button type=\"submit\">Run</button>",
      "      </div>",
      "    </form>",
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
      `window.__arrobaPublicationViewerConfig = ${safeJson(config)};`,
      viewerScript(),
      "</script>",
    ].join("\n"),
  )
}

function viewerTransport(publication: WorkflowPublicationConfig) {
  const transport = publication.transport ?? "human_http"
  return transport === "human_http" || transport === "api_sse_json" || transport === "websocket_json"
    ? transport
    : null
}

function viewerScript() {
  return String.raw`
(() => {
const viewerConfig = window.__arrobaPublicationViewerConfig || {};
const formEl = document.querySelector('#invoke-form');
const statusEl = document.querySelector('#status');
const outputEl = document.querySelector('#output');
const htmlOutputEl = document.querySelector('#html-output');
const traceStatusEl = document.querySelector('#trace-status');
const traceFeedEl = document.querySelector('#trace-feed');
const partialOutputs = [];

if (!viewerConfig.showForm && formEl) formEl.hidden = true;
renderRun(viewerConfig.initialResult?.workflow_run);
for (const trace of viewerConfig.initialTraces || []) renderTrace(trace);
if (viewerConfig.eventsUrl) subscribeHumanHttpEvents(viewerConfig.eventsUrl);
if (!viewerConfig.eventsUrl && viewerConfig.initialResult?.queued) statusEl.textContent = 'Queued';

formEl?.addEventListener('submit', async (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  const data = new FormData(form);
  const prompt = String(data.get('prompt') ?? '').trim();
  const files = data.getAll('artifact').filter((item) => item instanceof File && item.size > 0);
  if (!prompt && files.length === 0) return;
  const button = form.querySelector('button[type="submit"]');
  if (button) button.disabled = true;
  statusEl.textContent = 'Submitting';
  try {
    const artifacts = await Promise.all(files.map(readArtifact));
    if (viewerConfig.transport === 'human_http') {
      await invokeHumanHttp(prompt, artifacts);
    } else if (viewerConfig.transport === 'api_sse_json') {
      await invokeApiSse(prompt, artifacts);
    } else if (viewerConfig.transport === 'websocket_json') {
      await invokeWebSocket(prompt, artifacts);
    }
  } catch (error) {
    statusEl.textContent = error instanceof Error ? error.message : String(error);
  } finally {
    if (button) button.disabled = false;
  }
});

async function invokeHumanHttp(prompt, artifacts) {
  if (!artifacts.length) {
    const encoded = encodeURIComponent(prompt);
    window.location.href = viewerConfig.humanPromptTarget.prefix + encoded + viewerConfig.humanPromptTarget.suffix;
    return;
  }
  const response = await fetch(publicationUrl(viewerConfig.humanFormInvokePath), {
    method: 'POST',
    headers: { accept: 'text/html', 'content-type': 'application/json' },
    body: JSON.stringify({ prompt, artifacts }),
  });
  const html = await response.text();
  document.open();
  document.write(html);
  document.close();
}

async function invokeApiSse(prompt, artifacts) {
  resetForInvocation();
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
  resetForInvocation();
  const socket = new WebSocket(publicationWebSocketUrl(viewerConfig.websocketInvokePath));
  const readyArtifacts = new Set();
  await new Promise((resolve, reject) => {
    let invoked = false;
    socket.addEventListener('message', (event) => {
      const payload = JSON.parse(String(event.data || '{}'));
      if (payload.type === 'ready') {
        void sendArtifacts().catch(reject);
        return;
      }
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
      if (payload.type === 'final' || payload.type === 'timeout') {
        socket.close();
        resolve();
      }
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
        socket.send(JSON.stringify({
          type: 'artifact_begin',
          artifact_id: artifactId,
          name: artifact.name,
          mime_type: artifact.type,
          size_bytes: artifact.size_bytes,
        }));
        socket.send(JSON.stringify({ type: 'artifact_chunk', artifact_id: artifactId, data: artifact.base64 }));
        socket.send(JSON.stringify({ type: 'artifact_end', artifact_id: artifactId }));
      }
    }
  });
}

function inputPayload(prompt, artifacts) {
  return artifacts.length ? { prompt, artifacts } : { prompt };
}

function resetForInvocation() {
  if (formEl) formEl.hidden = true;
  outputEl.hidden = false;
  outputEl.textContent = '';
  htmlOutputEl.hidden = true;
  htmlOutputEl.innerHTML = '';
  partialOutputs.length = 0;
  traceFeedEl.innerHTML = '';
  traceStatusEl.textContent = 'No exposed traces';
  statusEl.textContent = 'Queued';
}

function subscribeHumanHttpEvents(path) {
  const events = new EventSource(publicationUrl(path));
  events.addEventListener('queued', (event) => applyPublicationEvent('queued', parseEventData(event)));
  events.addEventListener('status', (event) => applyPublicationEvent('status', parseEventData(event)));
  events.addEventListener('started', (event) => applyPublicationEvent('started', parseEventData(event)));
  events.addEventListener('partial', (event) => applyPublicationEvent('partial', parseEventData(event)));
  events.addEventListener('trace', (event) => applyPublicationEvent('trace', parseEventData(event)));
  events.addEventListener('final', (event) => { applyPublicationEvent('final', parseEventData(event)); events.close(); });
  events.addEventListener('timeout', (event) => { applyPublicationEvent('timeout', parseEventData(event)); events.close(); });
  events.addEventListener('error', () => { statusEl.textContent = 'Connection interrupted'; });
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
  if (type === 'queued') statusEl.textContent = 'Queued';
  if (type === 'started') renderRun(payload.workflow_run);
  if (type === 'status') renderRun(payload.workflow_run);
  if (type === 'partial') renderPartialOutput(payload.message || '');
  if (type === 'trace') renderTrace(payload);
  if (type === 'final') {
    if (payload.workflow_run) renderRun(payload.workflow_run);
    else if (typeof payload.message === 'string') renderFinalOutput(payload.message);
    statusEl.textContent = payload.workflow_run?.status || 'Completed';
  }
  if (type === 'timeout') statusEl.textContent = 'Still running';
}

function renderRun(run) {
  if (!run) return;
  statusEl.textContent = run.status || 'accepted';
  if (run.final_output) renderFinalOutput(run.final_output.message);
}

function renderPartialOutput(message) {
  if (typeof message !== 'string' || !message) return;
  partialOutputs.push(message);
  if (!htmlOutputEl.hidden) return;
  outputEl.textContent = partialOutputs.join('\n\n');
}

function renderFinalOutput(message) {
  if (typeof message !== 'string') {
    outputEl.textContent = JSON.stringify(message, null, 2);
    return;
  }
  const renderable = renderableOutput(message);
  if (renderable !== null) {
    outputEl.hidden = true;
    htmlOutputEl.hidden = false;
    htmlOutputEl.innerHTML = '';
    const frame = document.createElement('iframe');
    frame.setAttribute('sandbox', 'allow-scripts allow-forms allow-popups allow-modals');
    if (renderable.html !== null) frame.srcdoc = renderable.html;
    if (renderable.src !== null) frame.src = renderable.src;
    htmlOutputEl.append(frame);
    return;
  }
  htmlOutputEl.hidden = true;
  htmlOutputEl.innerHTML = '';
  outputEl.hidden = false;
  outputEl.textContent = message;
}

function renderableOutput(message) {
  try {
    const parsed = JSON.parse(message);
    if (parsed && parsed.kind === 'html' && typeof parsed.html === 'string') {
      return { html: parsed.html, src: null };
    }
    if (parsed && parsed.kind === 'response' && parsed.response) {
      const mode = parsed.response.mode;
      if (mode === 'html') {
        const html = typeof parsed.response.html === 'string'
          ? parsed.response.html
          : typeof parsed.response.body === 'string'
            ? parsed.response.body
            : typeof parsed.html === 'string'
              ? parsed.html
              : null;
        if (html !== null) return { html, src: null };
      }
      if (mode === 'serve' && typeof parsed.response.entry === 'string') {
        return { html: null, src: parsed.response.entry };
      }
    }
  } catch {
  }
  return null;
}

function renderTrace(trace) {
  if (!traceFeedEl) return;
  traceStatusEl.textContent = 'Live traces';
  const item = document.createElement('article');
  item.className = 'trace-item';
  const alias = trace.agent_alias || trace.agent_id || 'agent';
  const label = trace.node_label || trace.node_id || 'node';
  item.innerHTML = '<div class="trace-meta"><span>' + escapeText(alias) + '</span><span>' + escapeText(label) + '</span><span>' + escapeText(trace.level || '') + '</span></div><pre></pre>';
  item.querySelector('pre').textContent = trace.message || JSON.stringify(trace.data ?? trace, null, 2);
  traceFeedEl.append(item);
  traceFeedEl.scrollTop = traceFeedEl.scrollHeight;
}

async function readArtifact(file) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = '';
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.slice(index, index + chunkSize));
  }
  const base64 = btoa(binary);
  return {
    name: file.name,
    type: file.type || 'application/octet-stream',
    size_bytes: file.size,
    data_url: 'data:' + (file.type || 'application/octet-stream') + ';base64,' + base64,
    base64,
  };
}

function publicationUrl(path) {
  if (!path || !path.startsWith('/')) return path;
  const match = window.location.pathname.match(/^(\/display\/[^/]+)/);
  if (match) return match[1] + path;
  const prefix = publicationIngressPrefix();
  return prefix ? prefix + path : path;
}

function publicationWebSocketUrl(path) {
  const url = new URL(publicationUrl(path), window.location.href);
  url.protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return url.toString();
}

function publicationIngressPrefix() {
  const parts = window.location.pathname.split('/').filter(Boolean);
  if (!parts.length) return '';
  if (parts[0] === 'publication-ingress' && parts[1]) return '/' + parts[0] + '/' + parts[1];
  const routeFirst = String(viewerConfig.humanPromptTarget?.prefix || '').split('/').filter(Boolean)[0] || '';
  if (routeFirst && parts[0] === routeFirst) return '';
  if (!routeFirst && ['.well-known', 'invoke', 'mcp', 'health'].includes(parts[0])) return '';
  if (viewerConfig.transport !== 'human_http' && ['.well-known', 'invoke', 'mcp', 'health'].includes(parts[0])) return '';
  return '/' + parts[0];
}

function escapeText(value) {
  return String(value).replace(/[&<>\"]/g, (ch) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '\"': '&quot;' }[ch]));
}
})();
`
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
    "    .split-viewer { display: grid; grid-template-columns: minmax(0, 1fr) minmax(280px, 34%); gap: 0; max-width: none; width: 100vw; min-height: 100vh; margin: 0; padding: 0; }",
    "    .output-pane, .trace-pane { min-width: 0; min-height: 100vh; padding: 20px; overflow: auto; }",
    "    .output-pane { background: #ffffff; border-right: 1px solid #d7dbd2; }",
    "    .trace-pane { background: #f1f3ed; }",
    "    .pane-header { display: flex; align-items: baseline; justify-content: space-between; gap: 16px; margin-bottom: 14px; }",
    "    h1, h2 { margin: 0; font-size: 18px; line-height: 1.2; letter-spacing: 0; }",
    "    h2 { font-size: 14px; text-transform: uppercase; color: #4d564b; }",
    "    #status, #trace-status { margin: 0; color: #586069; font-size: 13px; }",
    "    .invoke-form { border: 1px solid #d0d0c8; background: #fafbf8; border-radius: 8px; padding: 14px; margin-bottom: 16px; }",
    "    textarea { box-sizing: border-box; width: 100%; padding: 12px; border: 1px solid #bbb; border-radius: 6px; font: inherit; resize: vertical; background: #fff; }",
    "    .actions { display: flex; flex-wrap: wrap; gap: 12px; align-items: center; margin-top: 12px; }",
    "    button { padding: 9px 14px; border: 0; border-radius: 6px; background: #1f2328; color: #fff; font: inherit; }",
    "    button:disabled { opacity: .55; }",
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
