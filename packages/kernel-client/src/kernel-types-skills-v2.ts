// MP-08 / MP-10 / MP-11: Skills v2 shared wire contract (533/141).
import type { PromptAttachmentPart, PromptQueueItem } from "./kernel-types.js"
export const SKILLS_MIN_LOCAL_PROTOCOL = 533
export const SKILLS_MIN_PEER_PROTOCOL = 141
export type SkillScope =
  | { kind: "chariox" }
  | { kind: "project"; project_id: string }

export type SkillState =
  | "auto"
  | "manual"
  | "off"

export type SkillFileManifest = {
  path: string
  sha256: string
  bytes: number
  executable: boolean
}

export type SkillProvenance = {
  kind: string
  source_machine: string | null
  locator: string | null
  source_revision: string | null
  provider: string | null
  account_profile: string | null
  installed_at_ms: number
  updated_at_ms: number
}

export type SkillRecord = {
  id: string
  name: string
  scope: SkillScope
  package_id: string | null
  description: string
  short_description: string | null
  default_state: SkillState
  state_override: SkillState | null
  digest: string
  declared_version: string | null
  files: (SkillFileManifest)[]
  provenance: SkillProvenance
  compatibility_notes: (string)[]
}

export type SkillPackageRecord = {
  id: string
  name: string
  scope: SkillScope
  custom: boolean
  source_locator: string | null
  source_revision: string | null
  upstream_members: Record<string, string>
}

export type SkillCatalog = {
  schema: number
  revision: number
  scope: SkillScope
  skills: (SkillRecord)[]
  packages: (SkillPackageRecord)[]
  chariox_overrides: Record<string, SkillState>
}

export type SkillSelection =
  | { source: "chariox"; skill_id: string; name: string; catalog_revision: number }
  | { source: "native"; entry_id: string; name: string; provider: string; account_profile: string; execution_id: string; inventory_revision: string }

export type NativeSkillExposure =
  | "confirmed"
  | "unverified"
  | "disabled"

export type NativeSkillEntry = {
  selection: SkillSelection
  description: string
  digest: string | null
  exposure: NativeSkillExposure
  default_state: SkillState
  complete_manifest: boolean
  supports_load_ack: boolean
  discovery_error: string | null
}

export type NativeSkillProvenance = {
  source_machine: string | null
  locator: string | null
  provider_version: string | null
}

export type NativeSkillDetails = {
  entry: NativeSkillEntry
  provenance: NativeSkillProvenance
  body: string | null
  files: (SkillFileManifest)[]
  declared_version: string | null
  compatibility_notes: (string)[]
  unavailable_reason: string | null
}

export type SkillDelivery =
  | { kind: "chariox" }
  | { kind: "provided_by"; provider: string; entry_id: string }
  | { kind: "conflict"; provider: string; entry_id: string; native_digest: string }
  | { kind: "unverified"; provider: string; entry_id: string }

export type SkillProjection = {
  skill: SkillRecord
  effective_state: SkillState
  project_override: SkillState | null
  delivery: SkillDelivery
  also_in_provider: string | null
}

export type SkillLoadReason =
  | "explicit"
  | "model"
  | "confirmed_native"

export type SkillLoadRecord = {
  skill_id: string
  digest: string
  session_id: string
  agent_id: string
  run_id: string
  prompt_id: string
  reason: SkillLoadReason
  context_generation: string
  delivered_at_ms: number
}

export type RepositorySkill = { path: string; name: string; description: string }
export type RepositorySkillSkip = { path: string; reason: string }
export type RepositorySkillMember = { path: string; skill_id: string; name: string; digest: string }
export type RepositorySkillUpdate = { member: RepositorySkillMember; digest: string }
export type SkillInstallSource =
  | { kind: "folder"; path: string }
  | { kind: "content"; skill_md: string }
  | { kind: "url"; url: string; pinned_revision: string | null }
  | { kind: "native"; selection: SkillSelection }
  | { kind: "repository"; url: string; revision: string; path: string }

export type SkillCollisionAction =
  | { action: "reject" }
  | { action: "rename"; name: string }
  | { action: "replace"; skill_id: string }

export type SkillCatalogAction =
  | { action: "install"; source: SkillInstallSource; collision: SkillCollisionAction; package_id: string | null }
  | { action: "update"; skill_id: string; source: SkillInstallSource; expected_digest: string }
  | { action: "remove"; skill_id: string }
  | { action: "set_state"; skill_id: string; state: SkillState | null }
  | { action: "create_package"; package_id: string; name: string }
  | { action: "rename_package"; package_id: string; name: string }
  | { action: "regroup"; skill_id: string; package_id: string | null }
  | { action: "remove_package"; package_id: string; remove_skills: boolean }
  | { action: "set_package_state"; package_id: string; state: SkillState | null }
  | { action: "move"; skill_id: string; destination: SkillScope }

export type SkillCatalogMutation = {
  scope: SkillScope
  expected_revision: number
  actions: (SkillCatalogAction)[]
}

export type SkillReviewReceipt = {
  id: string
  expires_at_ms: number
  mutation_sha256: string
  sources: (SkillReviewedSource)[]
}

export type SkillReviewedSource = {
  action_index: number
  skill_id: string
  digest: string
  source_revision: string | null
}

export type SkillsV2Request =
  | { action: "catalog"; project_id: string | null; agent_id: string | null }
  | { action: "get"; skill_id: string; project_id: string | null }
  | { action: "native_inventory"; agent_id: string; refresh: boolean }
  | { action: "native_details"; agent_id: string; selection: SkillSelection }
  | { action: "loads"; session_id: string; agent_id: string | null }
  | { action: "mutate"; mutation: SkillCatalogMutation; review_receipt_id: string }
  | { action: "preview"; mutation: SkillCatalogMutation }
  | { action: "resolve_repository"; url: string }
  | { action: "resolve_repository_update"; package_id: string }
  | { action: "list"; session_id: string; agent_id: string; cursor: string | null; limit: number }
  | { action: "load"; session_id: string; agent_id: string; skill_id: string }
  | { action: "submit"; session_id: string; attachment_id: string; target_agent_id: string; prompt: string; attachments: (PromptAttachmentPart)[]; selections: (SkillSelection)[] }

export type SkillsV2Response =
  | { kind: "catalog"; revision: number; skills: (SkillProjection)[]; packages: (SkillPackageRecord)[]; native: (NativeSkillEntry)[]; metadata_budget_percent: number }
  | { kind: "detail"; skill: SkillProjection; body: string }
  | { kind: "native_inventory"; entries: (NativeSkillEntry)[]; inventory_revision: string }
  | { kind: "native_details"; details: NativeSkillDetails }
  | { kind: "loads"; records: (SkillLoadRecord)[] }
  | { kind: "preview"; expected_revision: number; affected: (SkillRecord)[]; collisions: (string)[]; review_receipt: SkillReviewReceipt }
  | { kind: "repository"; locator: string; revision: string; skills: (RepositorySkill)[]; skipped: (RepositorySkillSkip)[] }
  | { kind: "repository_update"; package_id: string; locator: string; from_revision: string; revision: string; changed: RepositorySkillUpdate[]; added: RepositorySkill[]; removed: RepositorySkillMember[]; unchanged: RepositorySkillMember[]; skipped: RepositorySkillSkip[]; mutation: SkillCatalogMutation }
  | { kind: "published"; revision: number; changed_skill_ids: (string)[]; removed_skill_ids: (string)[] }
  | { kind: "page"; revision: number; skills: (SkillProjection)[]; next_cursor: string | null }
  | { kind: "loaded"; skill_id: string; digest: string; body: string; root: string; files: (SkillFileManifest)[] }
  | { kind: "submitted"; outcome: ({ Started: { prompt: PromptQueueItem } } | { Queued: { prompt: PromptQueueItem } }) }

export type SkillCatalogEvent =
  | { kind: "changed"; revision: number; scope: SkillScope; changed_skill_ids: (string)[]; removed_skill_ids: (string)[] }
  | { kind: "loaded"; record: SkillLoadRecord }

export type SkillTransferManifest = {
  catalog_revision: number
  project_id: string | null
  skills: (SkillTransferEntry)[]
}

export type SkillTransferEntry = {
  skill_id: string
  digest: string
  scope: SkillScope
  state: SkillState
}

export type SkillEnvironmentSnapshot = {
  schema: number
  source_revision: number
  project: SkillCatalog
  chariox_dependencies: (SkillRecord)[]
  dependency_packages: (SkillPackageRecord)[]
}

export type SkillTurnContext = {
  catalog_revision: number
  context_generation: string
  metadata: string
  metadata_tokens_upper_bound: number
  metadata_budget_tokens: number
  omitted_auto_count: number
  loaded: (SkillResolvedBody)[]
  native_selections: (SkillSelection)[]
}

export type SkillResolvedBody = {
  skill_id: string
  name: string
  digest: string
  root: string
  body: string
  files: (SkillFileManifest)[]
  reason: SkillLoadReason
  delivery: SkillDelivery
}

