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
  source_session_id?: string
  workflow_ref: string
  endpoint_ref: string
  hook_id?: string
  queue_ref?: string
  kernel_endpoint?: string
  transport?: string
  route?: string
  methods?: Array<"GET" | "POST">
  parser?: ParserConfig
  input_schema?: InputSchema
  tls?: TlsConfig
  mode?: "sync" | "async"
  sync_timeout_ms?: number
  poll_ms?: number
}

export type PublicationHookConfig = {
  id: string
  publication_id?: string
  transport: string
  endpoint_id: string
  queue_ref?: string
  route?: string
  methods?: string[]
  parser?: ParserConfig
  input_schema?: InputSchema | null
  mode?: "sync" | "async"
  response_mode?: string
}

export type WorkflowPublicationPackage = {
  schema_version: number
  package_version?: number
  publication_id: string
  alias?: string | null
  source_session_id?: string
  workflow_id: string
  default_bindings_path?: string
  hooks: PublicationHookConfig[]
}

export type WorkflowPublicationSnapshot = KernelWorkflowPublicationSnapshot

export type PublicationProviderModelProfile = {
  provider: string
  model?: string | null
  effort?: string | null
}

export type PublicationProviderModelOverride = {
  agent_id: string
  node_ids?: string[]
  captured: PublicationProviderModelProfile
  replacement?: PublicationProviderModelProfile | null
}

export type WorkflowPublicationBindings = {
  schema_version: number
  provider_model_overrides?: PublicationProviderModelOverride[]
}

export type PublicationNamedRequirement = {
  name: string
}

export type PublicationCredentialRequirement = {
  name: string
  used_by?: string
}

export type WorkflowPublicationRequirements = {
  schema_version: number
  mcps?: PublicationNamedRequirement[]
  skills?: PublicationNamedRequirement[]
  scripts?: PublicationNamedRequirement[]
  connectors?: PublicationNamedRequirement[]
  credentials?: PublicationCredentialRequirement[]
}

export type NormalizedInvocation = {
  publication_id: string
  request_id: string
  caller: Record<string, unknown>
  input: unknown
  mode: "sync" | "async"
}

export type WorkflowPublicationInvocationEnvelope = {
  publication_id: string
  hook_id?: string | null
  invocation_id: string
  transport: string
  endpoint_id: string
  queue_ref?: string | null
  input: unknown
  artifacts: unknown[]
  mode: "sync" | "async"
  caller: Record<string, unknown>
}

export type WorkflowRun = {
  id: string
  status: string
  workflow_id?: string
  endpoint_id?: string
  invocation_prompt?: string | null
  publication_invocation?: WorkflowPublicationInvocationEnvelope | null
  intermediate_outputs?: Array<{
    id: string
    source_node_run_id?: string
    output: { message: string }
    valid: boolean
    warning?: string | null
    timestamp_ms?: number
  }>
  final_output?: { message: string; artifacts?: unknown[] } | null
  created_at_ms?: number
  completed_at_ms?: number | null
}

export type WorkflowInvocationResult = {
  accepted: boolean
  queued?: boolean
  workflow_run?: WorkflowRun
  response?: unknown
}

export type GatewayDeps = {
  invokeWorkflow?: (invocation: NormalizedInvocation) => Promise<WorkflowInvocationResult>
  getPublicationStatusDetails?: (publication: WorkflowPublicationConfig) => Promise<Record<string, unknown>>
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
import type { WorkflowPublicationSnapshot as KernelWorkflowPublicationSnapshot } from "@arroba/kernel-client/kernel-types"
