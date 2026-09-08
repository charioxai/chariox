export type AppReleaseSummary = {
  version: string
  publisher_id: string
  package_digest: string
  schema_version: number
}

export type AppPublisherEnrollmentSummary = {
  request_id: string
  phase: "pending" | "approved" | "denied" | "cancelled" | "failed"
  publisher_id: string
  key_id: string
  key_fingerprint: string
  /** Historical approval; later revocation can supersede this revision. */
  approved_revision: string | null
  interaction_id: string | null
  failure: string | null
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

export type AppRequestErrorCode = "invalid_request" | "unauthorized" | "not_found" | "busy" | "limit_exceeded" | "conflict" | "digest_mismatch" | "storage_unavailable"

export type AppPackageUploadSummary = {
  handle: string
  phase: "receiving" | "finalized" | "aborted"
  expected_size: number
  accepted_bytes: number
  sha256: string
  expires_at_ms: number
}

export type AppInstallOperationSummary = {
  request_id: string
  phase: "preparing" | "awaiting_approval" | "starting" | "committed" | "cancelled" | "failed"
  installation_id: string | null
  /** Opaque decimal generation. Present only after verified staging. */
  generation: string | null
  package_digest: string
  interaction_id: string | null
  failure: string | null
}
