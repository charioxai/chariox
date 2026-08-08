import type { ExtensionGrant } from "./kernel-types-extensions.js"
import type { RuntimeSession } from "./kernel-types-session.js"
import type { AgentInstance } from "./kernel-types-runtime.js"

export type WorkflowDefinition = {
  id: string
  alias: string | null
  prompt?: string | null
  controlled_by_metaagent_id?: string | null
  created_at_ms?: number
  revision?: number
  flush_agent_context_before_run?: boolean
  max_concurrent?: number
  run_output_schema_ref?: string | null
  schemas?: WorkflowSchemaDefinition[]
  canvas_layout?: WorkflowCanvasLayout | null
  nodes?: WorkflowNodeDefinition[]
  edges?: WorkflowEdgeDefinition[]
  endpoints?: WorkflowEndpointDefinition[]
}

export type WorkflowSchemaDefinition = {
  id: string
  alias?: string | null
  description?: string | null
  schema: unknown
}

export type WorkflowCodeDefinition = {
  schema_version?: number
  parameters_schema?: unknown | null
  workflow: WorkflowCodeWorkflow
  schemas?: WorkflowCodeSchemaDefinition[]
  nodes?: WorkflowCodeNodeDefinition[]
  edges?: WorkflowCodeEdgeDefinition[]
  endpoints?: WorkflowCodeEndpointDefinition[]
  queues?: WorkflowCodeQueueDefinition[]
  schedules?: WorkflowCodeScheduleDefinition[]
  /** Legacy workflow-code import shape. New definitions should use schedules. */
  watchdogs?: WorkflowCodeWatchdogDefinition[]
}

export type WorkflowCodeWorkflow = {
  alias?: string | null
  prompt?: string | null
  flush_agent_context_before_run?: boolean | null
  max_concurrent?: number | null
  run_output_schema?: string | null
  intermediate_output_schema?: string | null
}

export type WorkflowCodeSchemaDefinition = {
  handle: string
  alias?: string | null
  description?: string | null
  schema: unknown
}

export type WorkflowCodeNodeDefinition = {
  handle: string
  agent: WorkflowCodeAgentBinding
  public_label?: string | null
  instructions?: string | null
  can_complete_workflow_run?: boolean | null
  can_emit_intermediate_run_output?: boolean | null
  wait_for_all_inputs?: boolean | null
  intermediate_output_schema?: string | null
  max_turns?: number | null
  extensions?: ExtensionGrant[]
  canvas?: WorkflowCodeCanvasPoint | null
}

export type WorkflowCodeAgentBinding =
  | ({ kind: "create" } & WorkflowCodeAgentCreate)
  | ({ kind: "existing" } & WorkflowCodeExistingAgent)

export type WorkflowCodeAgentCreate = {
  alias?: string | null
  provider: string
  model?: string | null
  effort?: string | null
  account_profile?: string | null
}

export type WorkflowCodeExistingAgent = {
  agent_ref: string
}

export type WorkflowCodeEdgeDefinition = {
  handle: string
  from_node: string
  to_node: string
  source_side?: "top" | "right" | "bottom" | "left" | null
  target_side?: "top" | "right" | "bottom" | "left" | null
  handoff_schema?: string | null
  validation_policy?: "warn" | "halt" | null
  canvas?: WorkflowCodeCanvasEdge | null
}

export type WorkflowCodeEndpointDefinition = {
  handle: string
  entry_node: string
  alias?: string | null
  canvas?: WorkflowCodeCanvasPoint | null
}

export type WorkflowCodeQueueDefinition = {
  handle: string
  alias: string
  priority?: number
  enabled?: boolean
}

export type WorkflowCodeScheduleDefinition = {
  handle: string
  endpoint: string
  queue?: string | null
  enabled?: boolean | null
  trigger: WorkflowScheduleTrigger
  invocation_prompt: string
  overlap_policy: "skip" | "queue"
  max_runs?: number | null
}

export type WorkflowCodeWatchdogDefinition = {
  handle: string
  endpoint: string
  queue?: string | null
  enabled?: boolean | null
  interval_seconds: number
  invocation_prompt: string
  policy: "skip" | "queue"
  max_wakeups?: number | null
}

export type WorkflowCodeCanvasPoint = {
  x: number
  y: number
}

export type WorkflowCodeCanvasEdge = {
  points?: WorkflowCodeCanvasPoint[]
}

export type WorkflowCodeValidationReport = {
  ok: boolean
  diagnostics?: WorkflowCodeValidationDiagnostic[]
}

export type WorkflowCodeValidationDiagnostic = {
  severity: "error" | "warning"
  code: string
  message: string
  handle?: string | null
  source_span?: WorkflowCodeSourceSpan | null
}

export type WorkflowCodeSourceSpan = {
  start_line: number
  start_column: number
  end_line: number
  end_column: number
}

export type WorkflowCodeCompileResult = {
  definition: WorkflowCodeDefinition
  validation: WorkflowCodeValidationReport
  logs: string
}

export type WorkflowCodeApplyReport = {
  workflow_id: string
  schema_refs?: Record<string, string>
  node_ids?: Record<string, string>
  agent_ids?: Record<string, string>
  edge_ids?: Record<string, string>
  endpoint_ids?: Record<string, string>
  queue_ids?: Record<string, string>
  schedule_ids?: Record<string, string>
  /** Legacy apply report field. New reports use schedule_ids. */
  watchdog_ids?: Record<string, string>
  canvas_layout_applied: boolean
  warnings?: WorkflowCodeApplyWarning[]
}

export type WorkflowCodeApplyWarning = {
  code: string
  message: string
  handle?: string | null
}

export type WorkflowCodeProviderRebinding = {
  node: string
  provider: string
  model?: string | null
  effort?: string | null
  account_profile?: string | null
}

export type WorkflowCodeAgentRebinding = {
  node: string
  agent_ref: string
}

export type WorkflowCodeCompileAndApplyResult = {
  compile: WorkflowCodeCompileResult
  apply: WorkflowCodeApplyReport
}

export type WorkflowCodeRunResult = {
  apply: WorkflowCodeCompileAndApplyResult
  invocation: WorkflowCodeRunInvocation
}

export type WorkflowCodeRunInvocation =
  | {
      kind: "started"
      workflow_run: WorkflowRun
      workflow: WorkflowDefinition
      endpoint: WorkflowEndpointDefinition
    }
  | {
      kind: "enqueued"
      queued_prompt: WorkflowQueuedPrompt
      workflow: WorkflowDefinition
      endpoint: WorkflowEndpointDefinition
    }

export type WorkflowCodeLanguage = "java_script" | "type_script"

export type WorkflowCodeArtifactMetadata = {
  name: string
  language: WorkflowCodeLanguage
  path: string
  source_sha256: string
  source_bytes: number
  validation: WorkflowCodeValidationReport
  provenance: WorkflowCodeArtifactProvenance
  history?: WorkflowCodeArtifactHistoryEntry[]
  created_at_ms: number
  updated_at_ms: number
}

export type WorkflowCodeArtifactActor = {
  user_id: string
  metaagent_id?: string | null
}

export type WorkflowCodeArtifactProvenance = {
  created_by: WorkflowCodeArtifactActor
  updated_by: WorkflowCodeArtifactActor
}

export type WorkflowCodeArtifactHistoryEntry = {
  action: WorkflowCodeArtifactHistoryAction
  at_ms: number
  actor: WorkflowCodeArtifactActor
  source_sha256: string
  validation_ok?: boolean | null
  workflow_id?: string | null
  warnings?: WorkflowCodeApplyWarning[]
}

export type WorkflowCodeArtifactHistoryAction =
  | "created"
  | "updated"
  | "imported"
  | "applied"
  | "run"

export type WorkflowCodeArtifact = {
  metadata: WorkflowCodeArtifactMetadata
  source: string
  definition: WorkflowCodeDefinition
}

export type WorkflowCodeArtifactPackage = {
  package_version: number
  name: string
  language: WorkflowCodeLanguage
  source: string
  source_sha256: string
  source_bytes: number
  definition_sha256: string
  definition: WorkflowCodeDefinition
  validation: WorkflowCodeValidationReport
  exported_at_ms: number
}

export type WorkflowCodeSourceExportFormat = "inline" | "directory"
export type WorkflowCodeSourceExportAgentMode = "portable_generated" | "existing_agents"

export type WorkflowCodePackageExportTarget =
  | { kind: "artifact"; name: string }
  | { kind: "workflow"; workflow_ref: string }

export type WorkflowCodeSourceExportTarget =
  | { kind: "artifact"; name: string }
  | { kind: "workflow"; workflow_ref: string }

export type WorkflowCodeSourceExportFile = {
  path: string
  contents: string
  sha256: string
}

export type WorkflowRegistrySourceScope = "workspace" | "user" | "builtin"
export type WorkflowRegistrySourceKind = "single_file" | "source_directory"

export type WorkflowRegistrySourceInput =
  | {
      kind: "single_file"
      source: string
      source_path?: string | null
    }
  | {
      kind: "source_directory"
      files: WorkflowCodeSourceExportFile[]
    }

export type WorkflowRegistryValidationSummary = {
  ok: boolean
  diagnostics?: string[]
}

export type WorkflowRegistryEntrySummary = {
  endpoints?: string[]
  queues?: string[]
  nodes?: string[]
  default_endpoint?: string | null
}

export type WorkflowRegistryEntryMetadata = {
  name: string
  source_scope: WorkflowRegistrySourceScope
  source_kind: WorkflowRegistrySourceKind
  source_path: string
  source_sha256: string
  source_bytes: number
  definition_sha256?: string | null
  created_at_ms: number
  updated_at_ms: number
  validation: WorkflowRegistryValidationSummary
  summary?: WorkflowRegistryEntrySummary | null
  parameters_schema?: unknown | null
}

export type WorkflowCodeSourceExport = {
  name: string
  language: WorkflowCodeLanguage
  format: WorkflowCodeSourceExportFormat
  source_path: string
  source: string
  source_sha256: string
  source_bytes: number
  definition_sha256: string
  files?: WorkflowCodeSourceExportFile[]
}

export type WorkflowCodeValidatedResponse = {
  WorkflowCodeValidated: {
    result: WorkflowCodeCompileResult
  }
}

export type WorkflowCodeAppliedResponse = {
  WorkflowCodeApplied: {
    result: WorkflowCodeCompileAndApplyResult
    session: RuntimeSession
  }
}

export type WorkflowCodeRunResponse = {
  WorkflowCodeRun: {
    result: WorkflowCodeRunResult
    session: RuntimeSession
  }
}

export type WorkflowCodeArtifactCreatedResponse = {
  WorkflowCodeArtifactCreated: {
    artifact: WorkflowCodeArtifact
  }
}

export type WorkflowCodeArtifactUpdatedResponse = {
  WorkflowCodeArtifactUpdated: {
    artifact: WorkflowCodeArtifact
  }
}

export type WorkflowCodeArtifactResponse = {
  WorkflowCodeArtifact: {
    artifact: WorkflowCodeArtifact
  }
}

export type WorkflowCodeArtifactsListedResponse = {
  WorkflowCodeArtifactsListed: {
    artifacts: WorkflowCodeArtifactMetadata[]
  }
}

export type WorkflowCodeArtifactDeletedResponse = {
  WorkflowCodeArtifactDeleted: {
    name: string
    path: string
  }
}

export type WorkflowCodeArtifactExportedResponse = {
  WorkflowCodeArtifactExported: {
    package: WorkflowCodeArtifactPackage
  }
}

export type WorkflowCodeArtifactImportedResponse = {
  WorkflowCodeArtifactImported: {
    artifact: WorkflowCodeArtifact
  }
}

export type WorkflowCodePackageExportedResponse = {
  WorkflowCodePackageExported: {
    package: WorkflowCodeArtifactPackage
  }
}

export type WorkflowCodePackageImportedResponse = {
  WorkflowCodePackageImported: {
    artifact: WorkflowCodeArtifact
  }
}

export type WorkflowCodeSourceExportedResponse = {
  WorkflowCodeSourceExported: {
    export: WorkflowCodeSourceExport
  }
}

export type WorkflowRegistryListedResponse = {
  WorkflowRegistryListed: {
    entries: WorkflowRegistryEntryMetadata[]
  }
}

export type WorkflowRegistryEntryResponse = {
  WorkflowRegistryEntry: {
    entry: WorkflowRegistryEntryMetadata
  }
}

export type WorkflowRegistryEntryAddedResponse = {
  WorkflowRegistryEntryAdded: {
    entry: WorkflowRegistryEntryMetadata
  }
}

export type WorkflowRegistryEntryDeletedResponse = {
  WorkflowRegistryEntryDeleted: {
    name: string
    path: string
  }
}

export type WorkflowRegistryEntryLoadedResponse = {
  WorkflowRegistryEntryLoaded: {
    entry: WorkflowRegistryEntryMetadata
    result: WorkflowCodeCompileAndApplyResult
    session: RuntimeSession
  }
}

export type WorkflowRegistryEntryRunResponse = {
  WorkflowRegistryEntryRun: {
    entry: WorkflowRegistryEntryMetadata
    result: WorkflowCodeRunResult
    session: RuntimeSession
  }
}

export type WorkflowCanvasPoint = {
  x: number
  y: number
}

export type WorkflowCanvasLayout = {
  version?: number | null
  revision: number
  coordinate_space: string
  nodes?: Record<string, WorkflowCanvasPoint>
  endpoints?: Record<string, WorkflowCanvasPoint>
  exits?: Record<string, WorkflowCanvasPoint>
  edges?: Record<string, { waypoints?: WorkflowCanvasPoint[] }>
}

export type WorkflowDesignWorkflow = {
  id: string
  alias?: string | null
  prompt?: string | null
  flush_agent_context_before_run?: boolean | null
  max_concurrent?: number | null
  run_output_schema_ref?: string | null
  schemas?: WorkflowSchemaDefinition[]
}

export type WorkflowDesignWorkflowPatch = {
  alias?: string | null
  prompt?: string | null
  flush_agent_context_before_run?: boolean | null
  max_concurrent?: number | null
  run_output_schema_ref?: string | null
}

export type WorkflowDesignSchemaPatch = {
  alias?: string | null
  description?: string | null
  schema?: unknown
}

export type WorkflowDesignNode = {
  id: string
  agent_id: string
  label?: string | null
  instructions?: string | null
  can_complete_workflow_run?: boolean | null
  can_emit_intermediate_run_output?: boolean | null
  wait_for_all_inputs?: boolean | null
  intermediate_output_schema_ref?: string | null
  max_turns?: number | null
}

export type WorkflowDesignNodePatch = {
  label?: string | null
  instructions?: string | null
  can_complete_workflow_run?: boolean | null
  can_emit_intermediate_run_output?: boolean | null
  wait_for_all_inputs?: boolean | null
  intermediate_output_schema_ref?: string | null
  max_turns?: number | null
}

export type WorkflowDesignEdge = {
  id: string
  from_node_id: string
  to_node_id: string
  source_side?: "top" | "right" | "bottom" | "left" | null
  target_side?: "top" | "right" | "bottom" | "left" | null
  handoff_schema_ref?: string | null
  validation_policy?: "warn" | "halt" | null
}

export type WorkflowDesignEdgePatch = {
  handoff_schema_ref?: string | null
  validation_policy?: "warn" | "halt" | null
}

export type WorkflowDesignEndpoint = {
  id: string
  alias?: string | null
  entry_node_id: string
}

export type WorkflowDesignEndpointPatch = {
  alias?: string | null
  entry_node_id?: string | null
}

export type WorkflowDesignOp =
  | { kind: "workflow_create"; workflow: WorkflowDesignWorkflow }
  | { kind: "workflow_update"; workflow_id: string; patch: WorkflowDesignWorkflowPatch }
  | { kind: "workflow_remove"; workflow_id: string }
  | { kind: "schema_add"; workflow_id: string; schema: WorkflowSchemaDefinition }
  | { kind: "schema_update"; workflow_id: string; schema_id: string; patch: WorkflowDesignSchemaPatch }
  | { kind: "schema_remove"; workflow_id: string; schema_id: string }
  | { kind: "node_add"; workflow_id: string; node: WorkflowDesignNode; position?: WorkflowCanvasPoint | null }
  | { kind: "node_update"; workflow_id: string; node_id: string; patch: WorkflowDesignNodePatch }
  | { kind: "node_move"; workflow_id: string; node_id: string; position: WorkflowCanvasPoint }
  | { kind: "node_remove"; workflow_id: string; node_id: string }
  | { kind: "edge_add"; workflow_id: string; edge: WorkflowDesignEdge }
  | { kind: "edge_update"; workflow_id: string; edge_id: string; patch: WorkflowDesignEdgePatch }
  | { kind: "edge_remove"; workflow_id: string; edge_id: string }
  | { kind: "endpoint_add"; workflow_id: string; endpoint: WorkflowDesignEndpoint; position?: WorkflowCanvasPoint | null }
  | { kind: "endpoint_update"; workflow_id: string; endpoint_id: string; patch: WorkflowDesignEndpointPatch }
  | { kind: "endpoint_move"; workflow_id: string; endpoint_id: string; position: WorkflowCanvasPoint }
  | { kind: "endpoint_remove"; workflow_id: string; endpoint_id: string }

export type WorkflowDesignOpForwarded = {
  session_id: string
  origin_client_id: string
  op_id: string
  kernel_sequence: number
  op: WorkflowDesignOp
}

export type WorkflowEndpointDefinition = {
  id: string
  owner_user_id?: string
  alias: string | null
  entry_node_id: string
}

export type WorkflowPublicationDefinition = {
  id: string
  session_id: string
  workflow_id: string
  endpoint_id: string
  queue_ref?: string | null
  alias?: string | null
  enabled: boolean
  kind?: "ingress" | "schedule_only" | string | null
  route?: string | null
  methods?: string[]
  transport?: unknown | null
  parser?: unknown | null
  input_schema?: unknown | null
  trace_exposure?: PublicationTraceExposurePolicy | null
  mode?: string | null
  sync_timeout_ms?: number | null
  poll_ms?: number | null
  source_workflow_revision?: number | null
  source_snapshot_digest?: string | null
  creation_operation_key?: string | null
  creation_request_digest?: string | null
  status?: string | null
  open_url?: string | null
  viewer_url?: string | null
  deployment?: unknown | null
  runtime_last_heartbeat_at_ms?: number | null
  runtime_last_error?: string | null
  runtime?: unknown | null
  schedule_count?: number | null
  schedules?: unknown[]
  watchdog_count?: number | null
  watchdogs?: unknown[]
  latest_run?: unknown | null
  recent_runs?: unknown[]
  latest_output?: unknown | null
  runtime_logs?: WorkflowPublicationRuntimeLogEntry[]
  created_by_user_id: string
  created_at_ms: number
  updated_at_ms: number
}

export type WorkflowPublicationRuntimeLogEntry = {
  at_ms: number
  level: string
  message: string
}

export type WorkflowPublicationPackageFile = {
  path: string
  content_base64: string
  executable?: boolean
}

export type WorkflowPublicationPackageExportedResponse = {
  WorkflowPublicationPackageExported: {
    publication: WorkflowPublicationDefinition
    package_version: number
    package_digest: string
    package_archive_base64: string
    package_files: WorkflowPublicationPackageFile[]
  }
}

export type WorkflowPublicationRuntimeAction = "start" | "stop" | "restart" | "inspect"

export type WorkflowPublicationRuntimeControlledResponse = {
  WorkflowPublicationRuntimeControlled: {
    publication: WorkflowPublicationDefinition
    action: WorkflowPublicationRuntimeAction
    status: string
    local_url?: string | null
    open_url?: string | null
    viewer_url?: string | null
    process_id?: number | null
    message?: string | null
  }
}

export type WorkflowPublicationEndpointRegisteredResponse = {
  WorkflowPublicationEndpointRegistered: {
    publication: WorkflowPublicationDefinition
    open_url: string
    viewer_url: string
    access: string
    expires_at_ms?: number | null
  }
}

export type PublicationTraceLevel =
  | "output_summary"
  | "assistant_messages"
  | "thinking"
  | "tool_use"

export type PublicationTraceExposurePolicy = {
  nodes?: Record<string, PublicationTraceLevel[]>
}

export type WorkflowPublicationSnapshot = {
  schema_version: number
  captured_at_ms?: number | null
  source_session?: WorkflowPublicationSourceSessionSnapshot | null
  workflow: WorkflowDefinition
  endpoint?: WorkflowEndpointDefinition | null
  queues?: WorkflowPromptQueueDefinition[]
  schedules?: WorkflowScheduleDefinition[]
  watchdogs?: WorkflowWatchdogDefinition[]
  agents?: AgentInstance[]
}

export type WorkflowPublicationSourceSessionSnapshot = {
  id?: string | null
  alias?: string | null
  workspace_id: string
  worktree_id: string
}

export type WorkflowScheduleTrigger =
  | { kind: "interval"; every_seconds: number }
  | { kind: "cron"; expression: string; timezone: string }

export type WorkflowScheduleDefinition = {
  id: string
  workflow_id: string
  endpoint_id: string
  queue_id?: string | null
  enabled: boolean
  trigger: WorkflowScheduleTrigger
  invocation_prompt: string
  overlap_policy: "skip" | "queue"
  max_runs?: number | null
  runs_started: number
  last_scheduled_for_ms?: number | null
  next_run_at_ms: number
  last_run_at_ms?: number | null
  last_status?: string | null
  last_error?: string | null
  last_workflow_run_id?: string | null
  pending_run?: boolean
  created_at_ms: number
  updated_at_ms: number
  interval_seconds?: number
  policy?: "skip" | "queue"
  max_wakeups?: number | null
  wakeups_executed?: number
}

export type WorkflowWatchdogDefinition = WorkflowScheduleDefinition

export type WorkflowPromptQueueDefinition = {
  id: string
  workflow_id: string
  alias: string
  priority: number
  enabled: boolean
  created_at_ms: number
  updated_at_ms: number
}

export type WorkflowQueuedPrompt = {
  id: string
  queue_id: string
  workflow_id: string
  endpoint_id: string
  prompt?: string | null
  publication_invocation?: WorkflowPublicationInvocationEnvelope | null
  source: "manual" | "scheduled" | "watchdog"
  schedule_id?: string | null
  watchdog_id?: string | null
  status: "queued" | "dispatching" | "running" | "completed" | "cancelled"
  created_at_ms: number
  updated_at_ms: number
  dispatched_at_ms?: number | null
  workflow_run_id?: string | null
}

export type WorkflowPublicationInvocationEnvelope = {
  publication_id: string
  hook_id?: string | null
  invocation_id: string
  transport: string
  endpoint_id: string
  queue_ref?: string | null
  input?: unknown
  artifacts?: unknown[]
  mode?: string | null
  caller?: unknown
}

export type WorkflowNodeDefinition = {
  id: string
  agent_id: string
  owner_user_id?: string
  created_by_user_id?: string
  public_label?: string
  instructions?: string | null
  can_complete_workflow_run?: boolean
  can_emit_intermediate_run_output?: boolean
  wait_for_all_inputs?: boolean
  intermediate_output_schema_ref?: string | null
  max_turns?: number | null
}

export type WorkflowEdgeDefinition = {
  id: string
  from_node_id: string
  to_node_id: string
  created_by_user_id?: string
  source_side?: "top" | "right" | "bottom" | "left" | null
  target_side?: "top" | "right" | "bottom" | "left" | null
  handoff_schema_ref?: string | null
  validation_policy?: "warn" | "halt" | null
}

export type WorkflowMessage = {
  id: string
  source_node_run_id: string | null
  source_node_iteration_index?: number | null
  edge_id?: string | null
  target_node_id: string
  message_type: string
  summary: string
  handoff_payload: string
  created_at_ms: number
}

export type WorkflowNodeRun = {
  id: string
  node_id: string
  agent_id: string
  iteration_index?: number
  status: string
  summary: string | null
  completion?: {
    summary: string
    output?: {
      message: string
    } | null
  } | null
  turn_envelope?: {
    delivery_token: string
    state: string
    rendered_prompt?: string | null
    mailbox_content?: string | null
    handoff_payloads_json?: string | null
    runtime_tool_calls?: {
      tool_name: string
      arguments_json: string
      result_json?: string | null
      ok: boolean
      timestamp_ms: number
    }[]
    prepared_at_ms: number
    dispatched_at_ms?: number | null
    acknowledged_at_ms?: number | null
    validated_completed_at_ms?: number | null
  } | null
  thinking_traces?: {
    id: string
    message: string
    timestamp_ms: number
  }[]
  created_at_ms: number
  started_at_ms: number | null
  completed_at_ms: number | null
}

export type WorkflowFailureEvent = {
  kind: string
  source_node_run_id: string
  edge_ids: string[]
  message: string
  timestamp_ms: number
}

export type WorkflowRun = {
  id: string
  workflow_id: string
  endpoint_id: string
  entry_node_id: string
  status: string
  invocation_prompt: string | null
  publication_invocation?: WorkflowPublicationInvocationEnvelope | null
  active_node_run_id: string | null
  node_runs: WorkflowNodeRun[]
  messages: WorkflowMessage[]
  failure_events?: WorkflowFailureEvent[]
  intermediate_outputs?: {
    id: string
    source_node_run_id: string
    output: {
      message: string
    }
    valid: boolean
    warning?: string | null
    timestamp_ms: number
  }[]
  final_output?: {
    message: string
    artifacts?: unknown[]
  } | null
  final_output_valid?: boolean | null
  final_output_warning?: string | null
  completed_by_node_run_id?: string | null
  created_at_ms: number
  started_at_ms: number | null
  completed_at_ms: number | null
}

export type WorkflowConsoleEntry = {
  timestamp_ms: number
  source_node_run_id?: string | null
  source_agent_id?: string | null
  text: string
}

export type WorkflowConsole = {
  workflow_id: string
  entries?: WorkflowConsoleEntry[]
}

export type ReadDirectoryTreeResult = {
  session_id: string
  root_path: string
  entries: unknown[]
}
