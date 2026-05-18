import { spawn } from "node:child_process"

import type {
  GatewayRequest,
  InputSchema,
  ParserConfig,
  WorkflowPublicationConfig,
} from "./publication-types.js"

export async function parseAndValidateRequest(
  request: GatewayRequest,
  publication: WorkflowPublicationConfig,
): Promise<unknown> {
  const parsed = await parseRequest(request, publication.parser ?? { kind: "json" })
  validateInput(parsed, publication.input_schema)
  return parsed
}

export function validateInput(value: unknown, schema: InputSchema | undefined) {
  if (!schema) return
  if (schema.type === "object" && (!value || typeof value !== "object" || Array.isArray(value))) {
    throw new Error("input schema expected object")
  }
  const object = value as Record<string, unknown>
  for (const field of schema.required ?? []) {
    if (object[field] === undefined || object[field] === null) {
      throw new Error(`input schema missing required field ${field}`)
    }
  }
  for (const [field, spec] of Object.entries(schema.properties ?? {})) {
    if (object[field] === undefined || !spec.type) continue
    if (!matchesJsonType(object[field], spec.type)) {
      throw new Error(`input schema field ${field} expected ${spec.type}`)
    }
  }
}

export function isParseErrorPayload(value: unknown): value is { __arroba_parse_error: string } {
  return !!value
    && typeof value === "object"
    && "__arroba_parse_error" in value
    && typeof (value as { __arroba_parse_error?: unknown }).__arroba_parse_error === "string"
}

async function parseRequest(request: GatewayRequest, config: ParserConfig): Promise<unknown> {
  switch (config.kind) {
    case "json":
      return request.body ?? {}
    case "query_params":
      return request.query ?? {}
    case "headers":
      return request.headers
    case "webhook":
      return { headers: request.headers, body: request.body ?? {}, query: request.query ?? {} }
    case "regex":
      return parseRegex(sourceValue(request, config.source ?? "path"), config)
    case "path_template":
      return parsePathTemplate(String(request.url.split("?")[0] ?? "/"), config)
    case "custom_command":
      return await parseCustomCommand(request, config)
  }
}

function parseRegex(source: string, config: ParserConfig) {
  if (!config.pattern) throw new Error("regex parser requires pattern")
  const match = new RegExp(config.pattern).exec(source)
  if (!match) throw new Error("request did not match regex parser")
  return { ...(match.groups ?? {}) }
}

function parsePathTemplate(pathname: string, config: ParserConfig) {
  if (!config.template) throw new Error("path_template parser requires template")
  const templateParts = config.template.split("/").filter(Boolean)
  const pathParts = pathname.split("/").filter(Boolean)
  if (templateParts.length !== pathParts.length) {
    throw new Error("request path did not match template")
  }
  const output: Record<string, string> = {}
  for (let index = 0; index < templateParts.length; index += 1) {
    const template = templateParts[index] ?? ""
    const value = decodeURIComponent(pathParts[index] ?? "")
    if (template.startsWith(":")) output[template.slice(1)] = value
    else if (template !== value) throw new Error("request path did not match template")
  }
  return output
}

async function parseCustomCommand(request: GatewayRequest, config: ParserConfig) {
  if (!config.command) throw new Error("custom_command parser requires command")
  const envelope = JSON.stringify({
    method: request.method,
    url: request.url,
    headers: request.headers,
    query: request.query ?? {},
    body: request.body ?? {},
  })
  const output = await runCommand(config.command, config.args ?? [], envelope)
  return JSON.parse(output || "{}")
}

function matchesJsonType(value: unknown, type: string) {
  if (type === "array") return Array.isArray(value)
  if (type === "object") return value != null && typeof value === "object" && !Array.isArray(value)
  return typeof value === type
}

function sourceValue(request: GatewayRequest, source: ParserConfig["source"]) {
  if (source === "body") return typeof request.body === "string" ? request.body : JSON.stringify(request.body ?? {})
  if (source === "query") return JSON.stringify(request.query ?? {})
  if (source === "headers") return JSON.stringify(request.headers)
  if (source === "request") return JSON.stringify({ method: request.method, url: request.url, body: request.body ?? {}, query: request.query ?? {} })
  return String(request.url.split("?")[0] ?? "/")
}

async function runCommand(command: string, args: string[], input: string): Promise<string> {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ["pipe", "pipe", "pipe"] })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => { stdout += chunk.toString() })
    child.stderr.on("data", (chunk) => { stderr += chunk.toString() })
    child.on("error", reject)
    child.on("close", (code) => {
      if (code === 0) resolve(stdout)
      else reject(new Error(`custom parser exited ${code}: ${stderr}`))
    })
    child.stdin.end(input)
  })
}
