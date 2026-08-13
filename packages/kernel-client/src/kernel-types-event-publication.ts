export type EventGeneratorParty = {
  id: string
  name: string
  url?: string | null
}

export type EventGeneratorCatalogSummary = {
  schema_version: number
  generator_id: string
  version: string
  name: string
  summary: string
  provider: string
  publisher: EventGeneratorParty
  operator: EventGeneratorParty
  verification: "chariox" | "official_provider" | "verified_community" | "community" | "self_hosted" | string
  manifest_digest: string
  protocol_version: number
  categories: string[]
  installed_count: number
  recommended: boolean
  availability: "available" | "development_preview" | "coming_soon" | string
}

export type EventGeneratorEventDefinition = {
  event_type: string
  version: number
  name: string
  description: string
  filter_schema: unknown
  required_scopes?: string[]
}

export type EventGeneratorCatalogDetail = EventGeneratorCatalogSummary & {
  authorization: unknown
  events: EventGeneratorEventDefinition[]
  signature: unknown
  deprecation?: unknown | null
}

export type EventCatalogCategory = {
  id: string
  name: string
  service_count: number
}

export type EventCatalogFacetValue = {
  value: string
  count: number
}

export type EventCatalogFacet = {
  id: string
  values: EventCatalogFacetValue[]
}

export type EventGeneratorCatalogPage = {
  services: EventGeneratorCatalogSummary[]
  next_cursor?: string | null
  categories: EventCatalogCategory[]
  facets: EventCatalogFacet[]
  stale: boolean
}

export type EventGeneratorEventPage = {
  events: EventGeneratorEventDefinition[]
  next_cursor?: string | null
}

export type EventGeneratorAuthorizationFlow = {
  generator_id: string
  status: "ready" | "user_action_required" | string
  connection_id?: string | null
  authorization_url?: string | null
  user_code?: string | null
  expires_at_ms?: number | null
}

export type EventGeneratorResource = {
  id: string
  name: string
  kind: string
  connection_scope: string
}

export type EventGeneratorResourcePage = {
  resources: EventGeneratorResource[]
  next_cursor?: string | null
}

export type EventConnectionStatus =
  | "pending"
  | "ready"
  | "expired"
  | "revoked"
  | "unavailable"
  | "error"

export type EventConnectionLifecycleState =
  | "not_installed"
  | "authorization_required"
  | "connecting"
  | "connected"
  | "connected_restricted"
  | "degraded"
  | "reauthorization_required"
  | "provider_unreachable"
  | "aegs_unavailable"
  | "unused"
  | "disconnecting"
  | "disconnected"

export type EventConnectionScope = {
  id: string
  label: string
  granted: boolean
  required: boolean
}

export type EventConnectedResource = {
  id: string
  name: string
  kind: string
}

export type EventConnection = {
  generator_id: string
  connection_id: string
  status: EventConnectionStatus
  lifecycle_state: EventConnectionLifecycleState
  scopes?: EventConnectionScope[]
  resources?: EventConnectedResource[]
  attached_trigger_count: number
  metadata?: unknown
  expires_at_ms?: number | null
  created_at_ms: number
  updated_at_ms: number
  last_validated_at_ms?: number | null
  last_successful_health_check_at_ms?: number | null
  last_accepted_event_at_ms?: number | null
  problem_code?: string | null
  problem_message?: string | null
  recovery_action?: string | null
  test_event_supported: boolean
}

export type EventConnectionTestResult = {
  occurrence_id: string
  accepted: boolean
  message?: string | null
}

export type EventConnectionAuthorization = {
  authorization_id: string
  generator_id: string
  connection_id?: string | null
  status: string
  authorization_url?: string | null
  user_code?: string | null
  expires_at_ms?: number | null
  created_at_ms: number
}

export type EventConnectionPage = {
  connections: EventConnection[]
  next_cursor?: string | null
}

export type WorkflowEventBindingDependency = {
  session_id: string
  publication_id: string
  binding_id: string
  status: "active" | "paused" | "conflict" | "tombstoned"
}

export type EventDeliveryStatus = {
  configured: boolean
  connected: boolean
  aeds_url?: string | null
  last_connected_at_ms?: number | null
  last_error?: string | null
  active_route_count: number
}
