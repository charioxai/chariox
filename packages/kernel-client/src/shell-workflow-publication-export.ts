import { mkdir, writeFile } from "node:fs/promises"
import { resolve as resolvePath } from "node:path"

import type {
  AgentInstance,
  ExtensionGrant,
  RuntimeSession,
  WorkflowPublicationDefinition,
} from "./kernel-types.js"
import { sessionWorkflowSchedules } from "./session-workflow-state.js"

export async function writeWorkflowPublicationExportPackage(
  publication: WorkflowPublicationDefinition,
  session: RuntimeSession,
  outputRoot: string,
  kernelUrl?: string,
) {
  await mkdir(outputRoot, { recursive: true })
  await mkdir(resolvePath(outputRoot, "public"), { recursive: true })
  await mkdir(resolvePath(outputRoot, "scripts"), { recursive: true })
  const publicationPackage = workflowPublicationPackage(publication)
  const workflowSnapshot = workflowPublicationSnapshot(publication, session)
  const requirements = workflowPublicationRequirements(workflowSnapshot.agents)
  const bindings = workflowPublicationBindings(workflowSnapshot)
  const config = workflowPublicationGatewayConfig(publication, kernelUrl)
  const files = {
    "publication.json": JSON.stringify(publicationPackage, null, 2) + "\n",
    "workflow.snapshot.json": JSON.stringify(workflowSnapshot, null, 2) + "\n",
    "requirements.json": JSON.stringify(requirements, null, 2) + "\n",
    "bindings.example.json": JSON.stringify(bindings, null, 2) + "\n",
    "publication.config.json": JSON.stringify(config, null, 2) + "\n",
    ".env.example": workflowPublicationEnvTemplate(publication, kernelUrl),
    "run.sh": workflowPublicationLauncherScript(),
    "README.md": workflowPublicationReadme(publication, publicationPackage, config),
    "public/index.html": workflowPublicationIndexHtml(publication),
    "public/app.js": workflowPublicationAppJs(publication),
    "public/styles.css": workflowPublicationStylesCss(),
  }
  const paths: string[] = []
  for (const [name, content] of Object.entries(files)) {
    const filePath = resolvePath(outputRoot, name)
    await writeFile(filePath, content, name === "run.sh" ? { mode: 0o755 } : undefined)
    paths.push(filePath)
  }
  return paths
}

function workflowPublicationPackage(publication: WorkflowPublicationDefinition) {
  const transport = hookTransport(publication)
  const hook: Record<string, unknown> = {
    id: `${publication.id}-hook`,
    publication_id: publication.id,
    transport,
    endpoint_id: publication.endpoint_id,
    queue_ref: publication.queue_ref ?? "default",
    input_schema: publication.input_schema ?? null,
    trace_exposure: publication.trace_exposure ?? null,
    response_mode: "accepted",
  }
  if (transport !== "schedule_only") {
    hook.route = publication.route ?? "/*"
    hook.methods = publication.methods?.length ? publication.methods : ["GET"]
    hook.parser = publication.parser ?? { kind: "json" }
    hook.mode = publication.mode ?? "sync"
  }
  return {
    schema_version: 1,
    package_version: 1,
    publication_id: publication.id,
    kind: publication.kind ?? "ingress",
    alias: publication.alias ?? null,
    source_session_id: publication.session_id,
    workflow_id: publication.workflow_id,
    default_bindings_path: "bindings.local.json",
    hooks: [hook],
    assets: {
      public_dir: "public",
      scripts_dir: "scripts",
    },
  }
}

function workflowPublicationSnapshot(publication: WorkflowPublicationDefinition, session: RuntimeSession) {
  const workflow = (session.workflows ?? []).find((candidate) => candidate.id === publication.workflow_id)
  if (!workflow) {
    throw new Error(`workflow ${publication.workflow_id} was not found in session ${session.id}`)
  }
  const endpoint = workflow.endpoints?.find((candidate) => candidate.id === publication.endpoint_id)
  if (!endpoint) {
    throw new Error(`endpoint ${publication.endpoint_id} was not found in workflow ${workflow.id}`)
  }
  const nodeAgentIds = new Set((workflow.nodes ?? []).map((node) => node.agent_id))
  const agents = session.agents.filter((agent) => nodeAgentIds.has(agent.id))
  const missingAgentIds = [...nodeAgentIds].filter((agentId) => !agents.some((agent) => agent.id === agentId))
  if (missingAgentIds.length > 0) {
    throw new Error(`workflow publication snapshot is missing agents: ${missingAgentIds.join(", ")}`)
  }
  return {
    schema_version: 1,
    captured_at_ms: Date.now(),
    source_session: {
      id: session.id,
      alias: session.alias ?? null,
      workspace_id: session.workspace_id,
      worktree_id: session.worktree_id,
    },
    workflow,
    endpoint,
    queues: (session.workflow_prompt_queues ?? []).filter((queue) => queue.workflow_id === workflow.id),
    schedules: sessionWorkflowSchedules(session).filter((schedule) => schedule.workflow_id === workflow.id),
    agents,
  }
}

function workflowPublicationRequirements(agents: AgentInstance[]) {
  const grants = agents.flatMap((agent) => agent.extension_grants ?? [])
  return {
    schema_version: 1,
    mcps: extensionRequirements(grants, "mcp"),
    skills: extensionRequirements(grants, "skill"),
    scripts: extensionRequirements(grants, "script"),
    connectors: extensionRequirements(grants, "connector"),
    credentials: credentialRequirements(grants),
  }
}

function workflowPublicationBindings(snapshot: ReturnType<typeof workflowPublicationSnapshot>) {
  return {
    schema_version: 1,
    provider_model_overrides: snapshot.agents.map((agent) => ({
      agent_id: agent.id,
      node_ids: (snapshot.workflow.nodes ?? [])
        .filter((node) => node.agent_id === agent.id)
        .map((node) => node.id),
      captured: {
        provider: agent.provider,
        model: agent.model,
        effort: agent.effort ?? null,
      },
      replacement: null,
    })),
  }
}

function extensionRequirements(grants: ExtensionGrant[], kind: ExtensionGrant["kind"]) {
  return uniqueByName(grants.filter((grant) => grant.kind === kind).map((grant) => ({ name: grant.name })))
}

function credentialRequirements(grants: ExtensionGrant[]) {
  return uniqueByName(grants
    .filter((grant) => typeof grant.credential === "string" && grant.credential.trim().length > 0)
    .map((grant) => ({ name: grant.credential as string, used_by: grant.name })))
}

function uniqueByName<T extends { name: string }>(items: T[]) {
  const seen = new Set<string>()
  return items.filter((item) => {
    if (seen.has(item.name)) return false
    seen.add(item.name)
    return true
  })
}

function hookTransport(publication: WorkflowPublicationDefinition) {
  if (publication.kind === "schedule_only") return "schedule_only"
  const transport = publication.transport as { kind?: unknown } | null | undefined
  return typeof transport?.kind === "string" ? transport.kind : "human_http"
}

function workflowPublicationGatewayConfig(
  publication: WorkflowPublicationDefinition,
  kernelUrl?: string,
) {
  const transport = hookTransport(publication)
  const config: Record<string, unknown> = {
    publication_id: publication.id,
    session_id: publication.session_id,
    workflow_ref: publication.workflow_id,
    endpoint_ref: publication.endpoint_id,
  }
  if (transport !== "schedule_only") {
    config.route = publication.route ?? "/*"
    config.parser = publication.parser ?? { kind: "json" }
    config.mode = publication.mode === "async" ? "async" : "sync"
  }
  if (kernelUrl) config.kernel_endpoint = kernelUrl
  if (transport !== "schedule_only" && publication.methods?.length) config.methods = publication.methods
  if (publication.transport != null) config.transport = publication.transport
  if (publication.input_schema != null) config.input_schema = publication.input_schema
  if (publication.trace_exposure != null) config.trace_exposure = publication.trace_exposure
  return config
}

function workflowPublicationEnvTemplate(publication: WorkflowPublicationDefinition, kernelUrl?: string) {
  return [
    "# Copy this file to .env or export these variables before running run.sh.",
    "HOST=0.0.0.0",
    "PORT=3000",
    `ARROBA_KERNEL_URL=${kernelUrl ?? "ws://127.0.0.1:43118"}`,
    "ARROBA_PUBLICATION_PACKAGE=./publication.json",
    `ARROBA_PUBLICATION_SESSION_ID=${publication.session_id}`,
    `ARROBA_PUBLICATION_ID=${publication.id}`,
    "",
    "# Optional HTTPS/TLS. When both files are set, the gateway serves HTTPS.",
    "# ARROBA_PUBLICATION_TLS_KEY_FILE=./tls.key",
    "# ARROBA_PUBLICATION_TLS_CERT_FILE=./tls.crt",
    "# ARROBA_PUBLICATION_TLS_ENABLED=true",
    "",
  ].join("\n")
}

function workflowPublicationLauncherScript() {
  return [
    "#!/usr/bin/env bash",
    "set -euo pipefail",
    "DIR=\"$(cd \"$(dirname \"${BASH_SOURCE[0]}\")\" && pwd)\"",
    "if [ -f \"$DIR/.env\" ]; then",
    "  set -a",
    "  . \"$DIR/.env\"",
    "  set +a",
    "fi",
    "export ARROBA_PUBLICATION_PACKAGE=\"${ARROBA_PUBLICATION_PACKAGE:-$DIR/publication.json}\"",
    "exec arroba-workflow-gateway",
    "",
  ].join("\n")
}

function workflowPublicationReadme(
  publication: WorkflowPublicationDefinition,
  publicationPackage: ReturnType<typeof workflowPublicationPackage>,
  config: Record<string, unknown>,
) {
  const scheduleOnly = hookTransport(publication) === "schedule_only"
  const route = String(config.route ?? "/*")
  const examplePath = route.includes("*") ? route.replace("*", "example") : route
  const methods = Array.isArray(config.methods) && config.methods.length ? config.methods.map(String) : ["GET", "POST"]
  const primaryMethod = methods[0] ?? "GET"
  const body = primaryMethod === "GET"
    ? ""
    : " \\\n  -H 'content-type: application/json' \\\n  -d '{\"input\":\"hello\"}'"
  const webSocketPath = publication.transport === "websocket_json"
    ? route
    : "/.well-known/arroba/publication/ws"
  return [
    `# Workflow Publication ${publication.alias ?? publication.id}`,
    "",
    "This directory is an Arroba workflow-gateway package. It runs only when an Arroba kernel is reachable.",
    "",
    "## Files",
    "",
    "- `publication.json`: published workflow package metadata",
    "- `workflow.snapshot.json`: captured workflow, endpoint, queues, and agents",
    "- `requirements.json`: required extensions and credentials",
    "- `bindings.example.json`: provider/model override template",
    "- `publication.config.json`: legacy gateway config for existing scripts",
    "- `.env.example`: environment template",
    "- `run.sh`: launcher for `arroba-workflow-gateway`",
    "- `public/`: editable browser assets",
    "",
    "## Run",
    "",
    "```bash",
    "cp .env.example .env",
    "./run.sh",
    "```",
    "",
    scheduleOnly ? "## Observe" : "## Invoke",
    "",
    "## Hooks",
    "",
    "```json",
    JSON.stringify(publicationPackage.hooks, null, 2),
    "```",
    "",
    ...(scheduleOnly ? [
      "This is a schedule-only publication. It has no HTTP, WebSocket, MCP, or API ingress route.",
      "Start the publication runtime to observe scheduled runs and exposed traces.",
      "",
    ] : [
      "```bash",
      "BASE_URL=http://127.0.0.1:3000",
      `curl -sS -X ${primaryMethod} "$BASE_URL${examplePath}"${body}`,
      "```",
      "",
      "## WebSocket",
      "",
      "The gateway also accepts WebSocket clients at:",
      "",
      "```text",
      `ws://127.0.0.1:3000${webSocketPath}`,
      `wss://127.0.0.1:3000${webSocketPath}`,
      "```",
      "",
      "Send `{\"type\":\"invoke\",\"input\":{}}` to invoke the publication.",
      "",
    ]),
    "## Local IPC",
    "",
    "Local scripts can invoke the publication without starting the HTTP gateway:",
    "",
    "```bash",
    "arroba-workflow-call --package ./publication.json --input '{\"input\":\"hello\"}'",
    "```",
    "",
  ].join("\n")
}

function workflowPublicationIndexHtml(publication: WorkflowPublicationDefinition) {
  return [
    "<!doctype html>",
    "<html lang=\"en\">",
    "<head>",
    "  <meta charset=\"utf-8\">",
    "  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    `  <title>${escapeHtml(publication.alias ?? publication.id)}</title>`,
    "  <link rel=\"stylesheet\" href=\"./styles.css\">",
    "</head>",
    "<body>",
    "  <main>",
    `    <h1>${escapeHtml(publication.alias ?? publication.id)}</h1>`,
    "    <form id=\"invoke-form\">",
    "      <textarea name=\"prompt\" rows=\"5\" autofocus></textarea>",
    "      <button type=\"submit\">Run</button>",
    "    </form>",
    "    <pre id=\"output\"></pre>",
    "  </main>",
    "  <script src=\"./app.js\" type=\"module\"></script>",
    "</body>",
    "</html>",
    "",
  ].join("\n")
}

function workflowPublicationAppJs(publication: WorkflowPublicationDefinition) {
  const route = publication.route ?? "/*"
  return [
    `const routePattern = ${JSON.stringify(route)}`,
    "const form = document.querySelector('#invoke-form')",
    "const output = document.querySelector('#output')",
    "function invocationUrl(prompt) {",
    "  const encoded = encodeURIComponent(prompt)",
    "  const wildcardIndex = routePattern.indexOf('*')",
    "  if (wildcardIndex >= 0) {",
    "    const path = routePattern.slice(0, wildcardIndex) + encoded + routePattern.slice(wildcardIndex + 1)",
    "    return new URL(path.startsWith('/') ? path : `/${path}`, window.location.origin).toString()",
    "  }",
    "  const url = new URL(routePattern.startsWith('/') ? routePattern : `/${routePattern}`, window.location.origin)",
    "  url.searchParams.set('prompt', prompt)",
    "  return url.toString()",
    "}",
    "form?.addEventListener('submit', (event) => {",
    "  event.preventDefault()",
    "  const data = new FormData(form)",
    "  const prompt = String(data.get('prompt') ?? '').trim()",
    "  if (!prompt) return",
    "  output.textContent = 'Opening workflow invocation...'",
    "  window.location.href = invocationUrl(prompt)",
    "})",
    "",
  ].join("\n")
}

function workflowPublicationStylesCss() {
  return [
    "body {",
    "  margin: 0;",
    "  font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif;",
    "  background: #f7f7f4;",
    "  color: #202124;",
    "}",
    "main {",
    "  max-width: 760px;",
    "  margin: 0 auto;",
    "  padding: 32px 20px;",
    "}",
    "textarea {",
    "  box-sizing: border-box;",
    "  width: 100%;",
    "  padding: 12px;",
    "  border: 1px solid #b9b9b2;",
    "  border-radius: 6px;",
    "  font: inherit;",
    "}",
    "button {",
    "  margin-top: 12px;",
    "  padding: 10px 14px;",
    "  border: 0;",
    "  border-radius: 6px;",
    "  background: #202124;",
    "  color: white;",
    "  font: inherit;",
    "}",
    "pre {",
    "  white-space: pre-wrap;",
    "}",
    "",
  ].join("\n")
}

function escapeHtml(value: string) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\"/g, "&quot;")
}
