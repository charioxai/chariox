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
