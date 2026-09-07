export type AppReleaseSummary = {
  version: string
  publisher_id: string
  package_digest: string
  schema_version: number
}

export type AppInstallationSummary = {
  installation_id: string
  app_id: string
  /** Opaque decimal generation, never convert to a JavaScript number. */
  generation: string
  active_release: AppReleaseSummary | null
  pending_generation: string | null
  admission_paused: boolean
}

export type AppUpdateSummary = {
  base_generation: string
  generation: string
  release: AppReleaseSummary
  phase: "staged" | "quiescing" | "prepared" | "committed" | "aborted"
  decision: "pending" | "approved" | "declined"
  created_at_ms: number
  updated_at_ms: number
}

export type AppRequestErrorCode = "invalid_request" | "unauthorized" | "not_found" | "busy" | "storage_unavailable"
