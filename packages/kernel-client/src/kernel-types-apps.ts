import type { WorkflowPublicationApp, WorkflowPublicationApps } from "./kernel-types-workflow.js"
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
  /** Protocol 363: uninstalled, with its data kept for a reinstall. */
  data_kept: boolean
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

/** Protocol 345 worker control. `dormant` workers start on their next use. */
export type AppWorkerSummary = {
  installation_id: string
  phase: "not_started" | "starting" | "running" | "dormant" | "stopped" | "failed"
  /** False after a user stop; on-demand use does not restart it. */
  enabled: boolean
  failure: string | null
  updated_at_ms: number | null
}

/** Protocol 345: one App event routed to one workflow endpoint and queue. */
export type AppAutomationSummary = {
  automation_id: string
  revision: number
  event_name: string
  event_version: number
  session_id: string
  publication_id: string
  endpoint_id: string
  queue_id: string
  scheduled: boolean
  status: "active" | "paused" | "broken" | "disabled"
}

/** Protocol 353: one inbox route of an installation, with occurrence outcomes. */
export interface AppInboxRouteSummary {
  route_id: string
  event_name: string
  source_event_type: string
  source_event_version: number
  active: boolean
  /** Protocol 358: the generator connection that feeds the route. */
  connection?: { generator_id: string; connection_id: string; connection_scope: string; filter?: unknown }
  pending: number
  delivered: number
  failed: number
  expired: number
}

/** Protocol 359: a generator connection an App may act through. */
export interface AppConnectionSummary {
  generator_id: string
  connection_id: string
  granted_at_ms: number
  actions: string[]
}

/** Protocol 367: the owner's consent to deploy a workflow with the Apps of its
 * publication's pinned App plan. Replaying the same `request_id` reports the answer. */
export type DeploymentAppsConsentStatus = "awaiting_approval" | "approved" | "declined" | "expired"

export type DeploymentAppsConsent = {
  request_id: string
  interaction_id: string
  deployment_id: string
  release_id: string
  package_digest: string
  status: DeploymentAppsConsentStatus
  expires_at_ms: number
}

/** Protocol 367: `DeploymentAppsPreview` — the publication's App plan with each
 * App's signed capabilities; `plan` is null when the workflow uses no App. */
export type DeploymentAppsPreview = {
  publication_id: string
  pinned: boolean
  plan: (Omit<WorkflowPublicationApps, "apps"> & {
    apps: (WorkflowPublicationApp & { capabilities: Record<string, unknown> })[]
  }) | null
}

/** Protocol 400: the exact payload released only to the accepting terminal. */
export type AppHostAction =
  | { kind: "clipboard_write"; text: string }
  | { kind: "open_link"; url: string }

export type AppHostActionAccepted = { operation_id: string; action: AppHostAction }
