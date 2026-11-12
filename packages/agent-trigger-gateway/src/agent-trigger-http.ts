import { parseAndValidateRequest } from "./publication-parser.js"

/** MP-08 / MP-09 / MP-10 / MP-11: the Workflows panel's HTTP trigger options, as the kernel keeps them. */
export type AgentHttpOptions = {
  route: string
  methods: Array<"GET" | "POST">
  parser: { kind: "path_template"; template: string } | { kind: "json" | "query_params" | "webhook" }
  mode: "async" | "sync"
  sync_timeout_ms?: number
  poll_ms?: number
  input_schema?: unknown
}
export type Invocation = { status: "accepted"; prompt_id: string; outcome: "started" | "queued" | "steered" }
  | { status: "offline" | "disabled" | "busy" | "payload_too_large" }
  | { status: "invalid_input"; message: string }
export type TriggerReply = { status: "pending" } | { status: "completed"; text: string } | { status: "failed"; message: string }
export type HttpBody = { body_base64: string; content_type: string }

/** What a gateway asks the kernel for one firing; the local gateway and the hosted ingress both answer it. */
export type HttpTriggerKernel = {
  options(): Promise<{ enabled: boolean; options: AgentHttpOptions }>
  invoke(input: unknown, http: HttpBody | null): Promise<Invocation>
  reply(promptId: string): Promise<TriggerReply>
}

/** A request below the trigger's base address; `path` starts with `/`. */
export type HttpTriggerRequest = {
  method: string
  path: string
  query: Record<string, unknown>
  search: string
  headers: Record<string, string | string[] | undefined>
  body: unknown
  raw: Buffer
  contentType: string
}
export type HttpTriggerResponse = { status: number; headers: Record<string, string>; body: unknown }

/** The path under the base that takes requests: the route, or its path prompt template. */
export function invocationPath(options: AgentHttpOptions): string {
  return options.parser.kind === "path_template" ? options.parser.template : options.route
}

export function matchesInvocationPath(path: string, options: AgentHttpOptions): boolean {
  const parts = (value: string) => value.split("/").filter(Boolean)
  const want = parts(invocationPath(options)), got = parts(path)
  return want.length === got.length && want.every((part, index) => part.startsWith(":") ? got[index]!.length > 0 : part === got[index])
}

const json = (status: number, body: unknown, headers: Record<string, string> = {}): HttpTriggerResponse =>
  ({ status, headers: { "cache-control": "no-store", ...headers }, body })

const refusals = { offline: [503, "agent offline"], disabled: [403, "trigger disabled"], busy: [429, "agent busy; retry shortly"], payload_too_large: [413, "payload too large"] } as const

/** One HTTP firing: method, route and parser as configured, then an async or sync response. */
/** A gateway's rate limit. It counts requests that reach the kernel; an offline agent gives the request back. */
export type RateLimit = { admit(): boolean; refund(): void }
const unlimited: RateLimit = { admit: () => true, refund: () => {} }

export async function invokeHttpTrigger(request: HttpTriggerRequest, kernel: HttpTriggerKernel, base: string, limit: RateLimit = unlimited,
  sleep: (ms: number) => Promise<void> = ms => new Promise(resolve => setTimeout(resolve, ms))): Promise<HttpTriggerResponse> {
  const { enabled, options } = await kernel.options()
  if (!matchesInvocationPath(request.path, options)) return json(404, { error: "trigger not found" })
  if (!(options.methods as string[]).includes(request.method)) return json(405, { error: `use ${options.methods.join(" or ")}` }, { allow: options.methods.join(", ") })
  if (!enabled) return json(403, { error: "trigger disabled" })
  let input: unknown
  try {
    input = await parseAndValidateRequest({ method: request.method, url: request.path + request.search, headers: request.headers, body: request.body, query: request.query, raw: request.raw }, { parser: options.parser })
  } catch (error) {
    return json(400, { error: error instanceof Error ? error.message : "invalid request" })
  }
  if (!limit.admit()) return json(429, { error: "too many requests; retry shortly" }, { "retry-after": "10" })
  const http = request.method === "POST" ? { body_base64: request.raw.toString("base64"), content_type: request.contentType } : null
  const result = await kernel.invoke(input, http)
  if (result.status === "offline") limit.refund()
  return answer(result, options, kernel, request, base, sleep)
}

/** The body of a request as the parsers read it: JSON, form or plain text; GET has none. */
export function decodeBody(method: string, raw: Buffer, contentType: string): { body: unknown } | HttpTriggerResponse {
  if (method !== "POST") return { body: null }
  const type = contentType.split(";")[0]!.trim().toLowerCase()
  if (type === "application/json") {
    try { return { body: raw.length ? JSON.parse(raw.toString("utf8")) : {} } } catch { return json(400, { error: "invalid request body" }) }
  }
  if (type === "application/x-www-form-urlencoded") return { body: Object.fromEntries(new URLSearchParams(raw.toString("utf8"))) }
  if (type === "text/plain" || !type) return { body: raw.toString("utf8") }
  return json(415, { error: "Use JSON, form or plain text" })
}

/** The trigger page's own message box: its text is the input, whatever the route and parser are. */
export async function sendPageMessage(prompt: string, kernel: HttpTriggerKernel): Promise<HttpTriggerResponse> {
  const result = await kernel.invoke({ prompt }, null)
  return result.status === "accepted" ? json(202, { firing_id: result.prompt_id, prompt_id: result.prompt_id, outcome: result.outcome }) : refusal(result)
}

function refusal(result: Exclude<Invocation, { status: "accepted" }>): HttpTriggerResponse {
  if (result.status === "invalid_input") return json(400, { error: result.message })
  const [status, error] = refusals[result.status]
  return json(status, { error }, result.status === "offline" ? { "retry-after": "5" } : result.status === "busy" ? { "retry-after": "10" } : {})
}

async function answer(result: Invocation, options: AgentHttpOptions, kernel: HttpTriggerKernel, request: HttpTriggerRequest, base: string,
  sleep: (ms: number) => Promise<void>): Promise<HttpTriggerResponse> {
  if (result.status !== "accepted") return refusal(result)
  const firing = { firing_id: result.prompt_id, prompt_id: result.prompt_id, outcome: result.outcome, status_url: base }
  if (options.mode === "async") {
    if (String(request.headers.accept).includes("text/html")) return { status: 303, headers: { "cache-control": "no-store", location: `${base}#firing-${encodeURIComponent(result.prompt_id)}` }, body: "" }
    return json(202, firing)
  }
  const timeout = options.sync_timeout_ms!, poll = options.poll_ms!
  const deadline = Date.now() + timeout
  for (;;) {
    const reply = await kernel.reply(result.prompt_id)
    if (reply.status === "completed") return json(200, { ...firing, status: "completed", reply: reply.text })
    if (reply.status === "failed") return json(502, { ...firing, status: "failed", error: reply.message })
    const left = deadline - Date.now()
    if (left <= 0) return json(202, { ...firing, status: "timeout", error: `no reply within ${timeout} ms; the agent keeps working` })
    await sleep(Math.min(poll, left))
  }
}
