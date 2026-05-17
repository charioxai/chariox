import type {
  WorkflowPublicationSenderCredential,
  WorkflowPublicationTrustedSender,
} from "@arroba/kernel-client/kernel-types"

export type ParserKind =
  | "json"
  | "query_params"
  | "headers"
  | "regex"
  | "path_template"
  | "webhook"
  | "custom_command"

export type ConnectorKind = "http" | "slack" | "discord" | "telegram" | "whatsapp" | "signal"

export type PrincipalRef = {
  id: string
  type?: "user" | "team" | "sender" | "service" | "anonymous"
  teams?: string[]
  display_name?: string
  allowed_connectors?: ConnectorKind[]
}

export type ExternalIdentityBinding = {
  connector: ConnectorKind
  external_id: string
  principal: PrincipalRef
}

export type BearerPrincipal = {
  token_env: string
  principal: PrincipalRef
}

export type ApiKeyPrincipal = {
  header: string
  key_env: string
  principal: PrincipalRef
}

export type ConnectorConfig =
  | { kind: "http"; principal?: PrincipalRef }
  | { kind: "slack"; signing_secret_env?: string }
  | { kind: "discord"; public_key_env?: string }
  | { kind: "telegram"; webhook_secret_env?: string }
  | { kind: "whatsapp"; app_secret_env?: string; verify_token_env?: string }
  | { kind: "signal"; webhook_secret_env?: string }

export type PairedSendersAuthConfig = {
  enabled?: boolean
  header?: string
  transport?: ConnectorKind
  pair_endpoint?: boolean
}

export type ArrobaAuthConfig = {
  mode: "arroba"
  allow_anonymous?: boolean
  anonymous_principal?: PrincipalRef
  paired_senders?: PairedSendersAuthConfig
  bearer_tokens?: BearerPrincipal[]
  api_keys?: ApiKeyPrincipal[]
  external_identities?: ExternalIdentityBinding[]
  connectors?: ConnectorConfig[]
}

export type AuthConfig =
  | { mode: "anonymous" }
  | { mode: "bearer"; token_env: string; principal?: PrincipalRef }
  | { mode: "api_key"; header: string; key_env: string; principal?: PrincipalRef }
  | ArrobaAuthConfig

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
  auth?: AuthConfig
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
  redeemPublicationPairCode?: (
    publication: WorkflowPublicationConfig,
    pairCode: string,
    displayName?: string | null,
  ) => Promise<WorkflowPublicationSenderCredential>
  authenticatePublicationSender?: (
    publication: WorkflowPublicationConfig,
    credential: string,
    transport: ConnectorKind,
  ) => Promise<WorkflowPublicationTrustedSender>
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

export type VerifiedExternalIdentity = {
  connector: ConnectorKind
  external_id: string
  metadata?: Record<string, unknown>
}

export type GatewayRequest = {
  method: string
  url: string
  headers: Record<string, string | string[] | undefined>
  body?: unknown
  query?: unknown
  raw: unknown
}
