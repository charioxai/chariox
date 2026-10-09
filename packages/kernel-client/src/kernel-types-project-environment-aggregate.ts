import type { ProjectEnvironmentManifestEntry, ProjectEnvironmentManifest, ProjectEnvironmentLocator, ProjectEnvironmentDefinitionOrigin, ProjectEnvironmentDefinitionSource, ProjectEnvironmentInput, ProjectEnvironmentPathEntry, ProjectEnvironmentSetupStepKind } from "./kernel-types-project-environment.js"
import type { ProjectEnvironmentReview } from "./project-environment-review.js"

// MP-02 / MP-03 / MP-08: protocol 471 value-free Project Environment contract.

export type EnvironmentLineage = { readonly project_id: string; readonly environment_id: string }

export type RequirementScope =
  | { readonly kind: "project" }
  | { readonly kind: "folder"; readonly folder_id: string }

export type RequirementOrigin =
  | { readonly kind: "detected"; readonly folder_id: string; readonly relative_path: string; readonly line: number | null; readonly evidence_digest: string }
  | { readonly kind: "detected_metadata"; readonly source: string; readonly reference: string }
  | { readonly kind: "user_added"; readonly user_id: string; readonly revision: number }
  | { readonly kind: "migrated"; readonly source: string; readonly digest: string }

export type EnvironmentFileKind =
  | "file"
  | "folder"

export type EnvironmentFile = { readonly relative_path: string; readonly kind: EnvironmentFileKind; readonly content_digest: string | null; readonly folder_id: string; readonly user_selected: boolean; readonly git_ignored: boolean | null; readonly byte_count: number | null; readonly credential_filter_verdict: EnvironmentCredentialFilterVerdict; readonly transfer_inclusion: EnvironmentTransferInclusion; readonly reason: string | null; readonly secret_looking: boolean }

export type EnvironmentInstallScope =
  | "project"
  | "user"
  | "system"

export type EnvironmentPlatform = { readonly os: string; readonly architecture: string }

export type EnvironmentProbe =
  | { readonly kind: "executable"; readonly name: string; readonly arguments: readonly string[]; readonly timeout_ms: number }
  | { readonly kind: "tcp"; readonly host: string; readonly port: number; readonly timeout_ms: number }
  | { readonly kind: "http"; readonly url: string; readonly expected_status: number; readonly timeout_ms: number }

export type EnvironmentVaultReference = { readonly service: string; readonly key: string }

export type EnvironmentProvider =
  | "codex"
  | "claude"
  | "opencode"

export type AgentToolKind =
  | "mcp"
  | "skill"
  | "instruction_file"

export type EnvironmentCommand = { readonly executable: string; readonly arguments: readonly string[]; readonly timeout_ms: number; readonly output_limit_bytes: number }

export type EnvironmentExecutionBoundary =
  | "disposable_worker"
  | "project_folder"

export type LegacySetupReference = { readonly definition_digest: string; readonly origin: ProjectEnvironmentDefinitionOrigin; readonly source: ProjectEnvironmentDefinitionSource; readonly target_platform: string; readonly source_path: string | null; readonly inputs: readonly ProjectEnvironmentInput[]; readonly path_entries: readonly ProjectEnvironmentPathEntry[]; readonly setup_step_kinds: readonly ProjectEnvironmentSetupStepKind[]; readonly setup_command_digests: readonly string[]; readonly validation_command_digests: readonly string[] }

export type RequirementSpec =
  | { readonly kind: "files"; readonly folder_id: string; readonly entries: readonly EnvironmentFile[] }
  | { readonly kind: "software"; readonly identity: string; readonly version_constraint: string | null; readonly platform: EnvironmentPlatform | null; readonly install_scope: EnvironmentInstallScope; readonly install_source: string | null; readonly detect_only: boolean }
  | { readonly kind: "services"; readonly identity: string; readonly target_binding: EnvironmentTargetBinding | null; readonly probe: EnvironmentProbe | null; readonly health_expectation: EnvironmentHealthExpectation | null; readonly depends_on: readonly string[] }
  | { readonly kind: "variables"; readonly name: string; readonly locator: ProjectEnvironmentLocator }
  | { readonly kind: "secrets"; readonly name: string; readonly vault: EnvironmentVaultReference | null }
  | { readonly kind: "accounts"; readonly provider: EnvironmentProvider; readonly linked_profile_ref: string | null; readonly required_capabilities: readonly string[] }
  | { readonly kind: "agent_tools"; readonly tool_kind: AgentToolKind; readonly registry_ref: string; readonly package_digest: string | null; readonly scope: RequirementScope; readonly runtime_requirements: readonly string[]; readonly vault_refs: readonly EnvironmentVaultReference[]; readonly compatible_providers: readonly EnvironmentProvider[] }
  | { readonly kind: "chariox_apps"; readonly app_id: string; readonly version: string | null; readonly content_digest: string | null; readonly required_grants: readonly string[] }
  | { readonly kind: "setup_checks"; readonly commands: readonly EnvironmentCommand[]; readonly probes: readonly EnvironmentProbe[]; readonly boundary: EnvironmentExecutionBoundary; readonly source_path: string | null; readonly source_digest: string | null; readonly legacy: LegacySetupReference | null }
  | { readonly kind: "machine_needs"; readonly platforms: readonly EnvironmentPlatform[]; readonly minimum_cpus: number | null; readonly minimum_memory_bytes: number | null; readonly minimum_free_disk_bytes: number | null; readonly gpu: string | null }

export type Requirement = { readonly requirement_id: string; readonly title: string; readonly scope: RequirementScope; readonly origins: readonly RequirementOrigin[]; readonly spec: RequirementSpec; readonly depends_on: readonly string[]; readonly platform_variants: readonly EnvironmentPlatform[]; readonly required: boolean; readonly legacy_entry: ProjectEnvironmentManifestEntry | null }

export type EnvironmentGitReference = { readonly url: string | null; readonly branch: string | null; readonly commit: string | null }

export type EnvironmentFolder = { readonly folder_id: string; readonly portable_folder_key: string; readonly label: string; readonly local_workspace_binding: string; readonly optional_git: EnvironmentGitReference | null; readonly requirements: readonly Requirement[] }

export type EnvironmentProposal = { readonly proposal_id: string; readonly requirement: Requirement }

export type EnvironmentObservationStatus =
  | "ready"
  | "missing"
  | "outdated"
  | "needs_your_input"
  | "conflict"

export type EnvironmentTargetBinding = { readonly machine_id: string; readonly target_instance_generation: string; readonly slice_ref: string | null }

export type EnvironmentMeasuredFacts = { readonly platform: EnvironmentPlatform | null; readonly version: string | null; readonly content_digest: string | null; readonly present: boolean | null; readonly cpus: number | null; readonly memory_bytes: number | null; readonly free_disk_bytes: number | null; readonly gpu: string | null }

export type MachineObservation = { readonly environment_id: string; readonly requirement_id: string; readonly target: EnvironmentTargetBinding; readonly revision_digest: string; readonly status: EnvironmentObservationStatus; readonly checked_at_ms: number | null; readonly measured_facts: EnvironmentMeasuredFacts; readonly reason_code: string; readonly safe_summary: string; readonly check_operation_id: string | null; readonly stale: boolean }

export type EnvironmentOperationKind =
  | "detect"
  | "apply"
  | "check"
  | "export"
  | "import"

export type EnvironmentOperationPhase =
  | "requested"
  | "preparing"
  | "awaiting_input"
  | "running"
  | "validating"
  | "ready"
  | "failed"
  | "cancelled"

export type EnvironmentItemResult = { readonly requirement_id: string; readonly status: EnvironmentObservationStatus; readonly reason_code: string; readonly safe_summary: string; readonly receipt_ids: readonly string[] }

export type EnvironmentReceipt = { readonly receipt_id: string; readonly requirement_id: string | null; readonly target: EnvironmentTargetBinding; readonly revision_digest: string; readonly artifact_digest: string; readonly recorded_at_ms: number }

export type EnvironmentCancellation = { readonly requested_at_ms: number; readonly settled_at_ms: number | null }

export type EnvironmentOperation = { readonly operation_id: string; readonly attempt: number; readonly local_project_id: string; readonly revision_digest: string; readonly target: EnvironmentTargetBinding; readonly kind: EnvironmentOperationKind; readonly phase: EnvironmentOperationPhase; readonly selected_items: readonly string[]; readonly per_item_opt_ins: readonly EnvironmentItemOptIn[]; readonly per_item_results: readonly EnvironmentItemResult[]; readonly created_at_ms: number; readonly updated_at_ms: number; readonly cancellation: EnvironmentCancellation | null; readonly receipts: readonly EnvironmentReceipt[]; readonly recovery_state: EnvironmentRecoveryState }

export type ProjectEnvironment = { readonly schema_version: number; readonly delivered_capabilities: DeliveredEnvironmentCapabilities; readonly local_project_id: string; readonly lineage: EnvironmentLineage; readonly revision: number; readonly content_digest: string; readonly parent_revision_digest: string | null; readonly evidence_digest: string | null; readonly project_requirements: readonly Requirement[]; readonly folders: readonly EnvironmentFolder[]; readonly proposals: readonly EnvironmentProposal[]; readonly reviewed_at_ms: number | null; readonly reviewed_by: string | null; readonly observations: readonly MachineObservation[]; readonly operations: readonly EnvironmentOperation[]; readonly legacy_manifest: ProjectEnvironmentManifest | null; readonly legacy_reviewed_manifest: ProjectEnvironmentManifest | null; readonly legacy_review: ProjectEnvironmentReview | null }

export type EnvironmentCredentialFilterVerdict =
  | "not_checked"
  | "clear"
  | "needs_vault"
  | "rejected"

export type EnvironmentTransferInclusion =
  | "include"
  | "exclude"
  | "review_required"

export type EnvironmentHealthExpectation =
  | { readonly kind: "reachable" }
  | { readonly kind: "http_status"; readonly status: number }
  | { readonly kind: "command_exit"; readonly exit_code: number }

export type EnvironmentCapability =
  | "get"
  | "detect"
  | "preview_diff"
  | "save"
  | "plan"
  | "apply"
  | "check"
  | "get_operation"
  | "cancel_operation"
  | "retry_operation"
  | "export"
  | "preview_import"
  | "commit_import"

export type DeliveredEnvironmentCapabilities = { readonly enabled_environment_operations: readonly EnvironmentCapability[]; readonly supported_schema: number }

export type EnvironmentOptInDecision =
  | "accepted"
  | "excluded"

export type EnvironmentItemOptIn = { readonly requirement_id: string; readonly decision: EnvironmentOptInDecision; readonly recorded_at_ms: number; readonly user_id: string }

export type EnvironmentRecoveryState =
  | { readonly kind: "pending" }
  | { readonly kind: "resumable"; readonly checkpoint_digest: string; readonly last_settled_requirement_id: string | null }
  | { readonly kind: "settled" }

export type EnvironmentFolderSpecification = { readonly folder_id: string; readonly portable_folder_key: string; readonly label: string; readonly optional_git: EnvironmentGitReference | null; readonly requirements: readonly Requirement[] }

export type EnvironmentRevisionDraft = { readonly project_requirements: readonly Requirement[]; readonly folders: readonly EnvironmentFolderSpecification[] }

export type EnvironmentSealedPayloadDescriptor = { readonly artifact_id: string; readonly content_digest: string; readonly byte_count: number; readonly target_kernel_id: string; readonly target_key_thumbprint: string }

export type EnvironmentExportSource = { readonly kernel_id: string; readonly project_label: string; readonly exported_at_ms: number }

export type EnvironmentPortableFolder = { readonly folder_id: string; readonly portable_folder_key: string; readonly label: string }

export type EnvironmentExportEnvelope = { readonly schema_version: number; readonly lineage: EnvironmentLineage; readonly source_revision_digest: string; readonly folder_map: readonly EnvironmentPortableFolder[]; readonly requirement_spec: EnvironmentRevisionDraft; readonly selected_file_manifest: readonly EnvironmentFile[]; readonly artifact_digests: readonly string[]; readonly source_metadata: EnvironmentExportSource; readonly sealed_target_payload_descriptor: EnvironmentSealedPayloadDescriptor | null }

export type EnvironmentDiffKind =
  | "added"
  | "changed"
  | "removed"
  | "unchanged"
  | "conflict"

export type EnvironmentRequirementDiff = { readonly requirement_id: string; readonly kind: EnvironmentDiffKind; readonly before: Requirement | null; readonly after: Requirement | null }

export type EnvironmentRevisionDiff = { readonly project_id: string; readonly expected_revision: number; readonly source_digest: string; readonly target_digest: string; readonly requirements: readonly EnvironmentRequirementDiff[]; readonly folder_map: readonly EnvironmentFolderBinding[] }

export type EnvironmentFolderBinding = { readonly folder_id: string; readonly local_workspace_binding: string }

export type EnvironmentImportChoice =
  | { readonly kind: "new" }
  | { readonly kind: "update"; readonly project_id: string; readonly expected_revision: number }
  | { readonly kind: "replace"; readonly project_id: string; readonly expected_revision: number }

export type EnvironmentImportFileDiff = { readonly folder_id: string; readonly relative_path: string; readonly kind: EnvironmentDiffKind; readonly before_digest: string | null; readonly after_digest: string | null; readonly byte_count: number; readonly credential_filter_verdict: EnvironmentCredentialFilterVerdict }

export type EnvironmentImportPreview = { readonly preview_id: string; readonly lineage: EnvironmentLineage; readonly source_digest: string; readonly matching_project_ids: readonly string[]; readonly choice: EnvironmentImportChoice; readonly folder_map: readonly EnvironmentFolderBinding[]; readonly revision_diff: EnvironmentRevisionDiff | null; readonly file_diff: readonly EnvironmentImportFileDiff[]; readonly platform_gaps: readonly string[]; readonly omitted_items: readonly string[]; readonly expires_at_ms: number }

export type EnvironmentApplyPlan = { readonly plan_id: string; readonly project_id: string; readonly revision_digest: string; readonly target: EnvironmentTargetBinding; readonly selected_items: readonly string[]; readonly required_opt_ins: readonly string[]; readonly conflicts: readonly EnvironmentItemResult[]; readonly expires_at_ms: number }

export type EnvironmentProviderAuthorizationLink = { readonly provider: EnvironmentProvider; readonly linked_profile_ref: string | null; readonly authorization_url: string; readonly expires_at_ms: number }
