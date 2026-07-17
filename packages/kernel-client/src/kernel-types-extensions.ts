export type ArrobaMcpServerConfig = {
  name: string
  transport: Record<string, unknown>
  enabled?: boolean
  required?: boolean
  startup_timeout_sec?: number | null
  tool_timeout_sec?: number | null
  enabled_tools?: string[] | null
  disabled_tools?: string[] | null
  tools?: Record<string, unknown>
}

export type McpImportSkip = {
  name: string
  reason: string
}

export type McpImportOutcome = {
  imported: ArrobaMcpServerConfig[]
  skipped: McpImportSkip[]
}

export type ArrobaSkillMetadata = {
  name: string
  description: string
  short_description?: string | null
  path: string
}

export type ArrobaEnvironmentConfig = {
  name: string
  runtime: Record<string, unknown>
}

export type ArrobaScriptMetadata = {
  name: string
  runtime: "python" | "typescript" | string
  path: string
  description: string
  input_schema: Record<string, unknown>
  definition_hash: string
  timeout_sec?: number | null
}

export type ArrobaConnectorDefinition = {
  kind: "connector" | string
  name: string
  description: string
  adapter: string
  credential?: { required?: boolean } | null
  timeout_ms?: number | null
  max_response_bytes?: number | null
  operations: Array<Record<string, unknown>>
}

export type ArrobaConnectorAdapterDefinition = {
  kind: "connector_adapter" | string
  name: string
  version?: string | null
  adapter_protocol: string
  command: string
  args?: string[]
  description?: string | null
  source?: "user" | "bundled" | string | null
  manifest_path?: string | null
}

export type ArrobaCredentialConfig = {
  id: string
  description?: string | null
  source: Record<string, unknown>
  allowed_hosts?: string[]
  allowed_uses?: ("http" | "pty" | "connector" | "browser" | "mcp" | string)[]
  injection: Record<string, unknown>
}

export type ExtensionKind = "mcp" | "skill" | "script" | "connector"

export type ExtensionSource = "home" | "worker"

export type ExtensionCatalogSource = ExtensionSource | "all"

export type ExtensionGrant = {
  /** Missing on legacy persisted grants; clients must interpret it as home. */
  source?: ExtensionSource
  kind: ExtensionKind
  name: string
  environment?: string | null
  credential?: string | null
  max_safety?: "read" | "write" | "destructive" | string | null
}

export type AgentExtensionCatalogEntry = {
  source: ExtensionSource
  resolved_kernel_id: string
  kind: ExtensionKind
  name: string
  description?: string | null
  definition_hash?: string | null
  environments: string[]
  credentials: string[]
  credential_required: boolean
  max_safety: string[]
}

export type AgentExtensionCatalog = {
  agent_id: string
  home_kernel_id: string
  worker_kernel_id?: string | null
  worker_available: boolean
  worker_error?: string | null
  entries: AgentExtensionCatalogEntry[]
}

export type SkillImportSkip = {
  name: string
  path: string
  reason: string
}

export type SkillImportOutcome = {
  imported: ArrobaSkillMetadata[]
  skipped: SkillImportSkip[]
}

export type ProviderCapabilityImportDuplicate = {
  provider: string
  source: string
  hash?: string | null
  reason: string
}

export type ProviderCapabilityImportEntry = {
  kind: string
  name: string
  provider: string
  source: string
  hash?: string | null
  action: string
  reason: string
  duplicates?: ProviderCapabilityImportDuplicate[]
}

export type ProviderCapabilityImportSummary = {
  candidates: number
  imported: number
  updated: number
  already_installed: number
  deduped: number
  skipped: number
  errors: number
}

export type ProviderCapabilityImportReport = {
  dry_run: boolean
  providers: string[]
  summary: ProviderCapabilityImportSummary
  mcps: ProviderCapabilityImportEntry[]
  skills: ProviderCapabilityImportEntry[]
}
