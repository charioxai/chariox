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
  properties?: Record<string, { type?: "string" | "number" | "boolean" | "object" | "array" }>
}

export type TlsConfig = {
  enabled?: boolean
  key_file?: string
  cert_file?: string
}

export type WorkflowPublicationConfig = {
  publication_id: string
  session_id: string
  workflow_ref: string
  endpoint_ref: string
  kernel_endpoint?: string
  route?: string
  methods?: Array<"GET" | "POST">
  parser?: ParserConfig
  input_schema?: InputSchema
  tls?: TlsConfig
  mode?: "sync" | "async"
  sync_timeout_ms?: number
  poll_ms?: number
}

export type NormalizedInvocation = {
  publication_id: string
  request_id: string
  caller: Record<string, unknown>
  input: unknown
  mode: "sync" | "async"
}

export type WorkflowRun = {
  id: string
  status: string
  workflow_id?: string
  endpoint_id?: string
  final_output?: { message: string; artifacts?: unknown[] } | null
}

export type WorkflowInvocationResult = {
  accepted: boolean
  queued?: boolean
  workflow_run?: WorkflowRun
  response?: unknown
}

export type GatewayDeps = {
  invokeWorkflow?: (invocation: NormalizedInvocation) => Promise<WorkflowInvocationResult>
}

export type PublicationInvocationOptions = {
  input: unknown
  caller?: Record<string, unknown>
  mode?: "sync" | "async"
  requestIdPrefix?: string
  deps?: Pick<GatewayDeps, "invokeWorkflow">
}

export type KernelLookupClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  close?: () => Promise<void>
}

export type GatewayRequest = {
  method: string
  url: string
  headers: Record<string, string | string[] | undefined>
  body?: unknown
  query?: unknown
  raw: unknown
}
