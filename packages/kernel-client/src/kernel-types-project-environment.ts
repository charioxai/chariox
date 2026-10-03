export type ProjectEnvironmentDefinitionOrigin = "user_authored" | "utility_generated"

export type ProjectEnvironmentDefinitionSource =
  | "commands"
  | "dockerfile"
  | "devcontainer"
  | "setup_script"

export type ProjectEnvironmentSetupStepKind =
  | "package"
  | "system_tool"
  | "compiler"
  | "native_dependency"
  | "command"

export type ProjectEnvironmentInputKind = "recipe" | "lockfile"

export type ProjectEnvironmentPathBase = "preparation_home" | "workspace"

export type ProjectEnvironmentPathEntry = {
  readonly base: ProjectEnvironmentPathBase
  readonly path: string
}

export type ProjectEnvironmentInput = {
  readonly kind: ProjectEnvironmentInputKind
  readonly path: string
  readonly sha256: string
}

export type ProjectEnvironmentSetupStep = {
  readonly kind: ProjectEnvironmentSetupStepKind
  readonly command: string
}

export type ProjectEnvironmentDefinition = {
  readonly schema_version: number
  readonly origin: ProjectEnvironmentDefinitionOrigin
  readonly source: ProjectEnvironmentDefinitionSource
  readonly target_platform: string
  readonly source_path: string | null
  readonly inputs?: readonly ProjectEnvironmentInput[]
  readonly path_entries?: readonly ProjectEnvironmentPathEntry[]
  readonly setup_steps: readonly ProjectEnvironmentSetupStep[]
  readonly validation_commands: readonly string[]
}

export type ProjectEnvironmentSetupUtilityInput = {
  readonly project_id: string
  readonly workspace_id: string
  readonly target_worker_id: string
  readonly target_platform: string
  readonly definition: ProjectEnvironmentDefinition | null
  readonly validation_commands: readonly string[]
}

export type ProjectEnvironmentSetupPhase =
  | "requested"
  | "preparing"
  | "validating"
  | "ready"
  | "failed"
  | "cancelled"

export type ProjectEnvironmentCommandResult = {
  readonly command_digest: string
  readonly exit_code: number
  readonly stdout_bytes: number
  readonly stderr_bytes: number
}

export type ProjectEnvironmentValidation = {
  readonly worker_id: string
  readonly platform: string
  readonly commands: readonly ProjectEnvironmentCommandResult[]
}

export type ProjectEnvironmentSetupStatus = {
  readonly operation_id: string
  readonly project_id: string
  readonly session_id: string
  readonly agent_id: string
  readonly worker_id: string
  readonly platform: string
  readonly phase: ProjectEnvironmentSetupPhase
  readonly attempt: number
  readonly progress_percent: number
  readonly definition_digest: string | null
  readonly validation: ProjectEnvironmentValidation | null
  readonly message: string | null
  readonly failure_code: string | null
  readonly failure_message: string | null
  readonly retryable: boolean
  readonly created_at_ms: number
  readonly updated_at_ms: number
}

// MP-08: metadata only; secret values are absent from every Project manifest projection.
export type ProjectEnvironmentUse = { readonly path: string; readonly line: number }
export type ProjectEnvironmentLocator =
  | { readonly kind: "env_file"; readonly path: string; readonly key: string }
  | { readonly kind: "workspace_environment"; readonly name: string }
  | { readonly kind: "config_file"; readonly path: string }
  | { readonly kind: "vault"; readonly service: string; readonly key: string }
  | { readonly kind: "missing" }
export type ProjectEnvironmentManifestEntry = {
  readonly name: string
  readonly workspace_id: string
  readonly kind: "variable" | "config_file"
  readonly classification: "secret" | "non_secret"
  readonly excluded?: boolean
  readonly uses: readonly ProjectEnvironmentUse[]
  readonly locator: ProjectEnvironmentLocator
  readonly status: "found" | "missing" | "problem"
}
export type ProjectEnvironmentManifest = {
  readonly schema_version: 1
  readonly project_id: string
  readonly evidence_digest: string
  readonly entries: readonly ProjectEnvironmentManifestEntry[]
  readonly private_files?: readonly {readonly workspace_id: string; readonly path: string; readonly bring: boolean; readonly reason: string}[]
  readonly toolchain_hints: readonly string[]
  readonly package_hints: readonly string[]
  readonly service_hints: readonly string[]
}
