import { spawn } from "node:child_process"

export type ParserKind =
  | "json"
  | "query_params"
  | "headers"
  | "regex"
  | "path_template"
  | "webhook"
  | "custom_command"

export type ParserConfig = {
  kind: ParserKind
  source?: "body" | "path" | "query" | "headers" | "request"
  pattern?: string
  template?: string
  command?: string
  args?: string[]
}

export type InputSchema = {
  type?: "object"
  required?: string[]
  properties?: Record<string, { type?: "string" | "integer" | "number" | "boolean" | "object" | "array" }>
  additionalProperties?: boolean
}

export type GatewayRequest = {
  method: string
  url: string
  headers: Record<string, string | string[] | undefined>
  body?: unknown
  query?: unknown
  raw: unknown
}

/** One parser for workflow publications and agent HTTP triggers. */
export async function parseAndValidateRequest(
  request: GatewayRequest,
  publication: { readonly parser?: ParserConfig; readonly input_schema?: InputSchema },
): Promise<unknown> {
  if (request.method.toUpperCase() === "GET" && new URL(request.url, "http://publication.local").searchParams.has("prompt")) {
    if (Buffer.byteLength(request.url, "utf8") > 8192) throw new Error("GET URL exceeds 8 KiB; send POST JSON")
    const params = new URL(request.url, "http://publication.local").searchParams
    const values: Record<string, string> = Object.create(null)
    for (const [key, value] of params) {
      if (Object.hasOwn(values, key)) throw new Error(`GET parameter ${key} must occur once`)
      values[key] = value
    }
    const data: Record<string, unknown> = { ...values }
    delete data.prompt
    if (publication.input_schema) {
      for (const [key, value] of Object.entries(data)) {
        const type = publication.input_schema.properties?.[key]?.type
        if (type === "integer") {
          if (!/^(0|-[1-9][0-9]*|[1-9][0-9]*)$/.test(String(value)) || !Number.isSafeInteger(Number(value))) throw new Error(`GET parameter ${key} must be a safe integer`)
          data[key] = Number(value)
        } else if (type === "boolean") {
          if (value !== "true" && value !== "false") throw new Error(`GET parameter ${key} must be true or false`)
          data[key] = value === "true"
        } else if (type && type !== "string") throw new Error("GET schemas support scalar fields; send POST JSON")
      }
      validateInput(data, publication.input_schema)
      if (publication.input_schema.additionalProperties === false && Object.keys(data).some(key => !Object.hasOwn(publication.input_schema!.properties ?? {}, key))) throw new Error("GET contains an undeclared parameter")
    }
    // Preserve query lexemes for authoritative kernel coercion.
    return values
  }
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

export function isParseErrorPayload(value: unknown): value is { __chariox_parse_error: string } {
  return !!value
    && typeof value === "object"
    && "__chariox_parse_error" in value
    && typeof (value as { __chariox_parse_error?: unknown }).__chariox_parse_error === "string"
}

async function parseRequest(request: GatewayRequest, config: ParserConfig): Promise<unknown> {
  switch (config.kind) {
    case "json":
      return request.body ?? {}
    case "query_params":
      return plainObject(request.query)
    case "headers":
      return plainObject(request.headers)
    case "webhook":
      return {
        headers: plainObject(request.headers),
        body: request.body ?? {},
        query: plainObject(request.query),
      }
    case "regex":
      return parseRegex(sourceValue(request, config.source ?? "path"), config, config.source ?? "path")
    case "path_template":
      return parsePathTemplate(String(request.url.split("?")[0] ?? "/"), config)
    case "custom_command":
      return await parseCustomCommand(request, config)
  }
}

function plainObject(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {}
  return { ...(value as Record<string, unknown>) }
}

function parseRegex(source: string, config: ParserConfig, sourceKind: ParserConfig["source"]) {
  if (!config.pattern) throw new Error("regex parser requires pattern")
  const match = new RegExp(config.pattern).exec(source)
  if (!match) throw new Error("request did not match regex parser")
  if (sourceKind !== "path") return { ...(match.groups ?? {}) }
  return Object.fromEntries(Object.entries(match.groups ?? {}).map(([key, value]) => [
    key,
    decodeURIComponent(value),
  ]))
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
  if (type === "integer") return typeof value === "number" && Number.isSafeInteger(value)
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
