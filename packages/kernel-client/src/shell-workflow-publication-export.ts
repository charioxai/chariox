import { mkdir, writeFile } from "node:fs/promises"
import { resolve as resolvePath } from "node:path"

import type { WorkflowPublicationDefinition } from "./kernel-types.js"

export async function writeWorkflowPublicationExportPackage(
  publication: WorkflowPublicationDefinition,
  outputRoot: string,
  kernelUrl?: string,
) {
  await mkdir(outputRoot, { recursive: true })
  const config = workflowPublicationGatewayConfig(publication, kernelUrl)
  const files = {
    "publication.config.json": JSON.stringify(config, null, 2) + "\n",
    ".env.example": workflowPublicationEnvTemplate(publication, kernelUrl),
    "run.sh": workflowPublicationLauncherScript(),
    "README.md": workflowPublicationReadme(publication, config),
  }
  const paths: string[] = []
  for (const [name, content] of Object.entries(files)) {
    const filePath = resolvePath(outputRoot, name)
    await writeFile(filePath, content, name === "run.sh" ? { mode: 0o755 } : undefined)
    paths.push(filePath)
  }
  return paths
}

function workflowPublicationGatewayConfig(
  publication: WorkflowPublicationDefinition,
  kernelUrl?: string,
) {
  const config: Record<string, unknown> = {
    publication_id: publication.id,
    session_id: publication.session_id,
    workflow_ref: publication.workflow_id,
    endpoint_ref: publication.endpoint_id,
    route: publication.route ?? "/*",
    parser: publication.parser ?? { kind: "json" },
    mode: publication.mode === "async" ? "async" : "sync",
  }
  if (kernelUrl) config.kernel_endpoint = kernelUrl
  if (publication.methods?.length) config.methods = publication.methods
  if (publication.transport != null) config.transport = publication.transport
  if (publication.input_schema != null) config.input_schema = publication.input_schema
  return config
}

function workflowPublicationEnvTemplate(publication: WorkflowPublicationDefinition, kernelUrl?: string) {
  return [
    "# Copy this file to .env or export these variables before running run.sh.",
    "HOST=0.0.0.0",
    "PORT=3000",
    `ARROBA_KERNEL_URL=${kernelUrl ?? "ws://127.0.0.1:43118"}`,
    "ARROBA_PUBLICATION_CONFIG=./publication.config.json",
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
    "export ARROBA_PUBLICATION_CONFIG=\"${ARROBA_PUBLICATION_CONFIG:-$DIR/publication.config.json}\"",
    "exec arroba-workflow-gateway",
    "",
  ].join("\n")
}

function workflowPublicationReadme(
  publication: WorkflowPublicationDefinition,
  config: Record<string, unknown>,
) {
  const route = String(config.route ?? "/*")
  const examplePath = route.includes("*") ? route.replace("*", "example") : route
  const methods = Array.isArray(config.methods) && config.methods.length ? config.methods.map(String) : ["GET", "POST"]
  const primaryMethod = methods[0] ?? "GET"
  const body = primaryMethod === "GET"
    ? ""
    : " \\\n  -H 'content-type: application/json' \\\n  -d '{\"input\":\"hello\"}'"
  return [
    `# Workflow Publication ${publication.alias ?? publication.id}`,
    "",
    "This directory is an Arroba workflow-gateway package. It runs only when an Arroba kernel is reachable.",
    "",
    "## Files",
    "",
    "- `publication.config.json`: gateway publication config",
    "- `.env.example`: environment template",
    "- `run.sh`: launcher for `arroba-workflow-gateway`",
    "",
    "## Run",
    "",
    "```bash",
    "cp .env.example .env",
    "./run.sh",
    "```",
    "",
    "## Invoke",
    "",
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
    "ws://127.0.0.1:3000/.well-known/arroba/publication/ws",
    "wss://127.0.0.1:3000/.well-known/arroba/publication/ws",
    "```",
    "",
    "Send `{\"type\":\"invoke\",\"input\":{}}` to invoke the publication.",
    "",
    "## Local IPC",
    "",
    "Local scripts can invoke the publication without starting the HTTP gateway:",
    "",
    "```bash",
    "arroba-workflow-call --config ./publication.config.json --input '{\"input\":\"hello\"}'",
    "```",
    "",
  ].join("\n")
}
