import type { WorkflowPublicationApp, WorkflowPublicationApps } from "./kernel-types-workflow.js"
export type AppReleaseSummary = {
  version: string
  publisher_id: string
  package_digest: string
  schema_version: number
  /** The signed manifest's `displayName`; absent when the App declares none. */
  display_name?: string
  /** The signed manifest's icon as a PNG or SVG `data:` URL; absent when none. */
  icon?: string
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

export type AppAcquisitionSource = "local_file" | "dev_push" | "register" | "library" | "direct_feed"
export type AppInstallationSummary = {
  channel: "dev" | "release"
  dev_room_id: string | null
  installation_id: string
  app_id: string
  /** Outgoing events declared by the active verified release. */
  events?: { name: string; label: string }[]
  /** Opaque decimal generation, never convert to a JavaScript number. */
  generation: string
  active_release: AppReleaseSummary | null
  /** `display_name` and `icon` of the active (or retained) release, for list rows. */
  display_name?: string
  icon?: string
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

export type AppRequestErrorCode = "invalid_request" | "unauthorized" | "not_found" | "busy" | "limit_exceeded" | "conflict" | "digest_mismatch" | "storage_unavailable" | "receipt_expired" | "reserved_publisher"

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
  /** Protocol 381: `queued` is approved and waiting to start, usually for a free App worker slot. */
  phase: "preparing" | "awaiting_approval" | "queued" | "starting" | "committed" | "cancelled" | "failed"
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
  /** Protocol 407: quarantine requires an explicit start. */
  phase: "not_started" | "starting" | "running" | "dormant" | "stopped" | "failed" | "quarantined"
  /** False after a user stop; on-demand use does not restart it. */
  enabled: boolean
  failure: string | null
  updated_at_ms: number | null
}

/** Protocol 345: one App event routed to one workflow endpoint and queue. */
export type AppAutomationSummary = {
  target_kind: "workflow_endpoint" | "agent";
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

/** An App plan with each App's signed capabilities. */
export type DeploymentAppsPlan = Omit<WorkflowPublicationApps, "apps"> & {
  apps: (WorkflowPublicationApp & { capabilities: Record<string, unknown> })[]
}

/** Protocol 367: `DeploymentAppsPreview` — the App plan a new release would
 * package (the owner's current App set); `plan` is null when the workflow uses
 * no App and `pinned` says a release was prepared. Protocol 377 adds
 * `release_plan`, the plan of the release asked for by package digest. */
export type DeploymentAppsPreview = {
  publication_id: string
  pinned: boolean
  plan: DeploymentAppsPlan | null
  release_plan?: DeploymentAppsPlan | null
}

/** Protocol 409: the exact payload released only to the accepting terminal. */
export type AppHostAction =
  | { kind: "clipboard_write"; text: string }
  | { kind: "open_link"; url: string }

export type AppHostActionAccepted = { operation_id: string; action: AppHostAction }
/** Protocol 410: owner-scoped restore result. Authority is never restored. */
export interface AppDataSnapshotRestored {
  installation_id: string
  generation: string
  snapshot_id: string
}

/** Protocol 418. Kernel identity; neither an installation grant nor a session. */
export type UserAppView = {
  view_id: string
  installation_id: string
  generation: string
  origin: string
  browser?: { tab_id: string; generation: number }
  access?: import("./kernel-types-kernel-browser.js").UserDomainWindowAccess
}

export type AppFrontendBundle = {
  entry: string
  assets: { path: string; content_type: string; body_base64: string }[]
  content_security_policy: string
  iframe_sandbox: string
}

/** Read-only catalog; runtime registrations never grant signed authority. */
export type AppViewCatalog = {
  actions: AppViewAction[]
  registrations: unknown[]
  state: { ok: true; value: unknown } | { ok: false; code: string; message: string }
  warnings: { code: string; message: string }[]
}
export type AppViewAction = {
  name: string; description: string; inputSchema: unknown; outputSchema?: unknown
  effect: "read" | "write" | "approval" | "person_confirmed"; approvalReason?: string
  /** `person_confirmed` only: the App's own view that previews the exact input on the person's card. */
  confirmationView?: string
}
/** target_id is the logical Room Tab ID, also accepted by InspectAppView. */
export type AppViewInspection = {
  installation_id: string; target_id: string; generation: string; development: boolean; catalog_json: string
}

/** MP-08/09/10/11: read-only App-view delivery; no backend authority. */
export type ClientAppViewRequest =
  | { Open: { installation_id: string } }
  | { Read: { view_id: string; offset: number; max_bytes: number } }
  | { Check: { view_id: string } }
  | { Close: { view_id: string } }

export type ClientAppViewDescriptor = {
  view_id: string
  installation_id: string
  generation: string
  package_digest: string
  bundle: { manifest: string; inventory: string; signature: unknown }
  trust: { publisherId: string; keyId: string; contentDigest: string; publicKey: string }
}

export type ClientAppViewResponse =
  | { Opened: ClientAppViewDescriptor }
  | { Chunk: { view_id: string; offset: number; body_base64: string; eof: boolean } }
  | { Valid: { view_id: string; generation: string } }
  | { Closed: { view_id: string } }
