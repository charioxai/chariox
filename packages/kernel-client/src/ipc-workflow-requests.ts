import type {
  WorkflowCodeArtifactPackage,
  WorkflowCodeAgentRebinding,
  WorkflowCodeLanguage,
  WorkflowCodePackageExportTarget,
  WorkflowCodeProviderRebinding,
  WorkflowCodeSourceExportAgentMode,
  WorkflowCodeSourceExportFormat,
  WorkflowCodeSourceExportTarget,
  WorkflowDesignOp,
  WorkflowPublicationSnapshot,
  WorkflowRegistrySourceInput,
  WorkflowRegistrySourceScope,
  WorkflowScheduleTrigger,
} from "./kernel-types.js"

export function createWorkflowRequest(sessionId: string, alias?: string | null) {
  return {
    CreateWorkflow: {
      session_id: sessionId,
      alias: alias ?? null,
    },
  }
}

export type ValidateWorkflowCodeRequest = {
  ValidateWorkflowCode: {
    session_id: string
    node_path: string
    source: string
    provider_rebindings?: WorkflowCodeProviderRebinding[]
    agent_rebindings?: WorkflowCodeAgentRebinding[]
  }
}

export function validateWorkflowCodeRequest(
  sessionId: string,
  nodePath: string,
  source: string,
  providerRebindings: WorkflowCodeProviderRebinding[] = [],
  agentRebindings: WorkflowCodeAgentRebinding[] = [],
): ValidateWorkflowCodeRequest {
  return {
    ValidateWorkflowCode: {
      session_id: sessionId,
      node_path: nodePath,
      source,
      ...(providerRebindings.length > 0 ? { provider_rebindings: providerRebindings } : {}),
      ...(agentRebindings.length > 0 ? { agent_rebindings: agentRebindings } : {}),
    },
  }
}

export type ApplyWorkflowCodeRequest = {
  ApplyWorkflowCode: {
    session_id: string
    node_path: string
    source: string
    provider_rebindings?: WorkflowCodeProviderRebinding[]
    agent_rebindings?: WorkflowCodeAgentRebinding[]
  }
}

export function applyWorkflowCodeRequest(
  sessionId: string,
  nodePath: string,
  source: string,
  providerRebindings: WorkflowCodeProviderRebinding[] = [],
  agentRebindings: WorkflowCodeAgentRebinding[] = [],
): ApplyWorkflowCodeRequest {
  return {
    ApplyWorkflowCode: {
      session_id: sessionId,
      node_path: nodePath,
      source,
      ...(providerRebindings.length > 0 ? { provider_rebindings: providerRebindings } : {}),
      ...(agentRebindings.length > 0 ? { agent_rebindings: agentRebindings } : {}),
    },
  }
}

export type RunWorkflowCodeRequest = {
  RunWorkflowCode: {
    session_id: string
    node_path: string
    source: string
    provider_rebindings?: WorkflowCodeProviderRebinding[]
    agent_rebindings?: WorkflowCodeAgentRebinding[]
    endpoint?: string | null
    queue_ref?: string | null
    prompt: string
  }
}

export function runWorkflowCodeRequest(
  sessionId: string,
  nodePath: string,
  source: string,
  prompt: string,
  options: {
    providerRebindings?: WorkflowCodeProviderRebinding[]
    agentRebindings?: WorkflowCodeAgentRebinding[]
    endpoint?: string | null
    queueRef?: string | null
  } = {},
): RunWorkflowCodeRequest {
  const providerRebindings = options.providerRebindings ?? []
  const agentRebindings = options.agentRebindings ?? []
  return {
    RunWorkflowCode: {
      session_id: sessionId,
      node_path: nodePath,
      source,
      ...(providerRebindings.length > 0 ? { provider_rebindings: providerRebindings } : {}),
      ...(agentRebindings.length > 0 ? { agent_rebindings: agentRebindings } : {}),
      ...(options.endpoint !== undefined ? { endpoint: options.endpoint } : {}),
      ...(options.queueRef !== undefined ? { queue_ref: options.queueRef } : {}),
      prompt,
    },
  }
}

export type ApplyWorkflowCodeArtifactRequest = {
  ApplyWorkflowCodeArtifact: {
    session_id: string
    name: string
    provider_rebindings?: WorkflowCodeProviderRebinding[]
    agent_rebindings?: WorkflowCodeAgentRebinding[]
  }
}

export function applyWorkflowCodeArtifactRequest(
  sessionId: string,
  name: string,
  providerRebindings: WorkflowCodeProviderRebinding[] = [],
  agentRebindings: WorkflowCodeAgentRebinding[] = [],
): ApplyWorkflowCodeArtifactRequest {
  return {
    ApplyWorkflowCodeArtifact: {
      session_id: sessionId,
      name,
      ...(providerRebindings.length > 0 ? { provider_rebindings: providerRebindings } : {}),
      ...(agentRebindings.length > 0 ? { agent_rebindings: agentRebindings } : {}),
    },
  }
}

export type RunWorkflowCodeArtifactRequest = {
  RunWorkflowCodeArtifact: {
    session_id: string
    name: string
    provider_rebindings?: WorkflowCodeProviderRebinding[]
    agent_rebindings?: WorkflowCodeAgentRebinding[]
    endpoint?: string | null
    queue_ref?: string | null
    prompt: string
  }
}

export function runWorkflowCodeArtifactRequest(
  sessionId: string,
  name: string,
  prompt: string,
  options: {
    providerRebindings?: WorkflowCodeProviderRebinding[]
    agentRebindings?: WorkflowCodeAgentRebinding[]
    endpoint?: string | null
    queueRef?: string | null
  } = {},
): RunWorkflowCodeArtifactRequest {
  const providerRebindings = options.providerRebindings ?? []
  const agentRebindings = options.agentRebindings ?? []
  return {
    RunWorkflowCodeArtifact: {
      session_id: sessionId,
      name,
      ...(providerRebindings.length > 0 ? { provider_rebindings: providerRebindings } : {}),
      ...(agentRebindings.length > 0 ? { agent_rebindings: agentRebindings } : {}),
      ...(options.endpoint !== undefined ? { endpoint: options.endpoint } : {}),
      ...(options.queueRef !== undefined ? { queue_ref: options.queueRef } : {}),
      prompt,
    },
  }
}

export type ListWorkflowRegistryRequest = {
  ListWorkflowRegistry: {
    session_id: string
  }
}

export function listWorkflowRegistryRequest(sessionId: string): ListWorkflowRegistryRequest {
  return {
    ListWorkflowRegistry: {
      session_id: sessionId,
    },
  }
}

export type GetWorkflowRegistryEntryRequest = {
  GetWorkflowRegistryEntry: {
    session_id: string
    name: string
  }
}

export function getWorkflowRegistryEntryRequest(
  sessionId: string,
  name: string,
): GetWorkflowRegistryEntryRequest {
  return {
    GetWorkflowRegistryEntry: {
      session_id: sessionId,
      name,
    },
  }
}

export type AddWorkflowRegistryEntryRequest = {
  AddWorkflowRegistryEntry: {
    session_id: string
    name: string
    scope?: WorkflowRegistrySourceScope
    source: WorkflowRegistrySourceInput
    node_path: string
  }
}

export function addWorkflowRegistryEntryRequest(
  sessionId: string,
  name: string,
  source: WorkflowRegistrySourceInput,
  nodePath: string,
  scope?: WorkflowRegistrySourceScope,
): AddWorkflowRegistryEntryRequest {
  return {
    AddWorkflowRegistryEntry: {
      session_id: sessionId,
      name,
      ...(scope !== undefined ? { scope } : {}),
      source,
      node_path: nodePath,
    },
  }
}

export type AddWorkflowRegistryEntryFromWorkflowRequest = {
  AddWorkflowRegistryEntryFromWorkflow: {
    session_id: string
    name: string
    workflow_ref: string
    scope?: WorkflowRegistrySourceScope
    agent_mode?: WorkflowCodeSourceExportAgentMode
  }
}

export function addWorkflowRegistryEntryFromWorkflowRequest(
  sessionId: string,
  name: string,
  workflowRef: string,
  options: {
    scope?: WorkflowRegistrySourceScope
    agentMode?: WorkflowCodeSourceExportAgentMode
  } = {},
): AddWorkflowRegistryEntryFromWorkflowRequest {
  return {
    AddWorkflowRegistryEntryFromWorkflow: {
      session_id: sessionId,
      name,
      workflow_ref: workflowRef,
      ...(options.scope !== undefined ? { scope: options.scope } : {}),
      ...(options.agentMode !== undefined ? { agent_mode: options.agentMode } : {}),
    },
  }
}

export type DeleteWorkflowRegistryEntryRequest = {
  DeleteWorkflowRegistryEntry: {
    session_id: string
    name: string
    scope?: WorkflowRegistrySourceScope
  }
}

export function deleteWorkflowRegistryEntryRequest(
  sessionId: string,
  name: string,
  scope?: WorkflowRegistrySourceScope,
): DeleteWorkflowRegistryEntryRequest {
  return {
    DeleteWorkflowRegistryEntry: {
      session_id: sessionId,
      name,
      ...(scope !== undefined ? { scope } : {}),
    },
  }
}

export type LoadWorkflowRegistryEntryRequest = {
  LoadWorkflowRegistryEntry: {
    session_id: string
    name: string
    parameters?: Record<string, unknown>
    provider_rebindings?: WorkflowCodeProviderRebinding[]
    agent_rebindings?: WorkflowCodeAgentRebinding[]
  }
}

export function loadWorkflowRegistryEntryRequest(
  sessionId: string,
  name: string,
  optionsOrProviderRebindings:
    | WorkflowCodeProviderRebinding[]
    | {
        parameters?: Record<string, unknown>
        providerRebindings?: WorkflowCodeProviderRebinding[]
        agentRebindings?: WorkflowCodeAgentRebinding[]
      } = {},
): LoadWorkflowRegistryEntryRequest {
  const options = Array.isArray(optionsOrProviderRebindings)
    ? { providerRebindings: optionsOrProviderRebindings }
    : optionsOrProviderRebindings
  const providerRebindings = options.providerRebindings ?? []
  const agentRebindings = options.agentRebindings ?? []
  return {
    LoadWorkflowRegistryEntry: {
      session_id: sessionId,
      name,
      ...(options.parameters && Object.keys(options.parameters).length > 0
        ? { parameters: options.parameters }
        : {}),
      ...(providerRebindings.length > 0 ? { provider_rebindings: providerRebindings } : {}),
      ...(agentRebindings.length > 0 ? { agent_rebindings: agentRebindings } : {}),
    },
  }
}

export type RunWorkflowRegistryEntryRequest = {
  RunWorkflowRegistryEntry: {
    session_id: string
    name: string
    parameters?: Record<string, unknown>
    provider_rebindings?: WorkflowCodeProviderRebinding[]
    agent_rebindings?: WorkflowCodeAgentRebinding[]
    endpoint?: string | null
    queue_ref?: string | null
    prompt: string
  }
}

export function runWorkflowRegistryEntryRequest(
  sessionId: string,
  name: string,
  prompt: string,
  options: {
    parameters?: Record<string, unknown>
    providerRebindings?: WorkflowCodeProviderRebinding[]
    agentRebindings?: WorkflowCodeAgentRebinding[]
    endpoint?: string | null
    queueRef?: string | null
  } = {},
): RunWorkflowRegistryEntryRequest {
  const providerRebindings = options.providerRebindings ?? []
  const agentRebindings = options.agentRebindings ?? []
  return {
    RunWorkflowRegistryEntry: {
      session_id: sessionId,
      name,
      ...(options.parameters && Object.keys(options.parameters).length > 0
        ? { parameters: options.parameters }
        : {}),
      ...(providerRebindings.length > 0 ? { provider_rebindings: providerRebindings } : {}),
      ...(agentRebindings.length > 0 ? { agent_rebindings: agentRebindings } : {}),
      ...(options.endpoint !== undefined ? { endpoint: options.endpoint } : {}),
      ...(options.queueRef !== undefined ? { queue_ref: options.queueRef } : {}),
      prompt,
    },
  }
}

export type CreateWorkflowCodeArtifactRequest = {
  CreateWorkflowCodeArtifact: {
    session_id: string
    name: string
    language: WorkflowCodeLanguage
    node_path: string
    source: string
  }
}

export function createWorkflowCodeArtifactRequest(
  sessionId: string,
  name: string,
  nodePath: string,
  source: string,
  language: WorkflowCodeLanguage = "java_script",
): CreateWorkflowCodeArtifactRequest {
  return {
    CreateWorkflowCodeArtifact: {
      session_id: sessionId,
      name,
      language,
      node_path: nodePath,
      source,
    },
  }
}

export type UpdateWorkflowCodeArtifactRequest = {
  UpdateWorkflowCodeArtifact: {
    session_id: string
    name: string
    language: WorkflowCodeLanguage
    node_path: string
    source: string
  }
}

export function updateWorkflowCodeArtifactRequest(
  sessionId: string,
  name: string,
  nodePath: string,
  source: string,
  language: WorkflowCodeLanguage = "java_script",
): UpdateWorkflowCodeArtifactRequest {
  return {
    UpdateWorkflowCodeArtifact: {
      session_id: sessionId,
      name,
      language,
      node_path: nodePath,
      source,
    },
  }
}

export type GetWorkflowCodeArtifactRequest = {
  GetWorkflowCodeArtifact: {
    session_id: string
    name: string
  }
}

export function getWorkflowCodeArtifactRequest(
  sessionId: string,
  name: string,
): GetWorkflowCodeArtifactRequest {
  return {
    GetWorkflowCodeArtifact: {
      session_id: sessionId,
      name,
    },
  }
}

export type ListWorkflowCodeArtifactsRequest = {
  ListWorkflowCodeArtifacts: {
    session_id: string
  }
}

export function listWorkflowCodeArtifactsRequest(
  sessionId: string,
): ListWorkflowCodeArtifactsRequest {
  return {
    ListWorkflowCodeArtifacts: {
      session_id: sessionId,
    },
  }
}

export type DeleteWorkflowCodeArtifactRequest = {
  DeleteWorkflowCodeArtifact: {
    session_id: string
    name: string
  }
}

export function deleteWorkflowCodeArtifactRequest(
  sessionId: string,
  name: string,
): DeleteWorkflowCodeArtifactRequest {
  return {
    DeleteWorkflowCodeArtifact: {
      session_id: sessionId,
      name,
    },
  }
}

export type ExportWorkflowCodeArtifactRequest = {
  ExportWorkflowCodeArtifact: {
    session_id: string
    name: string
  }
}

export function exportWorkflowCodeArtifactRequest(
  sessionId: string,
  name: string,
): ExportWorkflowCodeArtifactRequest {
  return {
    ExportWorkflowCodeArtifact: {
      session_id: sessionId,
      name,
    },
  }
}

export type ExportWorkflowCodePackageRequest = {
  ExportWorkflowCodePackage: {
    session_id: string
    name: string
    target?: WorkflowCodePackageExportTarget
    agent_mode?: WorkflowCodeSourceExportAgentMode
  }
}

export function exportWorkflowCodePackageRequest(
  sessionId: string,
  name: string,
  target?: WorkflowCodePackageExportTarget,
  agentMode: WorkflowCodeSourceExportAgentMode = "portable_generated",
): ExportWorkflowCodePackageRequest {
  return {
    ExportWorkflowCodePackage: {
      session_id: sessionId,
      name,
      ...(target ? { target, agent_mode: agentMode } : {}),
    },
  }
}

export type ImportWorkflowCodeArtifactRequest = {
  ImportWorkflowCodeArtifact: {
    session_id: string
    package: WorkflowCodeArtifactPackage
    name?: string
    overwrite?: boolean
    node_path: string
  }
}

export function importWorkflowCodeArtifactRequest(
  sessionId: string,
  workflowCodePackage: WorkflowCodeArtifactPackage,
  nodePath: string,
  options: {
    name?: string
    overwrite?: boolean
  } = {},
): ImportWorkflowCodeArtifactRequest {
  return {
    ImportWorkflowCodeArtifact: {
      session_id: sessionId,
      package: workflowCodePackage,
      ...(options.name !== undefined ? { name: options.name } : {}),
      ...(options.overwrite !== undefined ? { overwrite: options.overwrite } : {}),
      node_path: nodePath,
    },
  }
}

export type ImportWorkflowCodePackageRequest = {
  ImportWorkflowCodePackage: {
    session_id: string
    package: WorkflowCodeArtifactPackage
    name?: string
    overwrite?: boolean
    node_path: string
  }
}

export function importWorkflowCodePackageRequest(
  sessionId: string,
  workflowCodePackage: WorkflowCodeArtifactPackage,
  nodePath: string,
  options: {
    name?: string
    overwrite?: boolean
  } = {},
): ImportWorkflowCodePackageRequest {
  return {
    ImportWorkflowCodePackage: {
      session_id: sessionId,
      package: workflowCodePackage,
      ...(options.name !== undefined ? { name: options.name } : {}),
      ...(options.overwrite !== undefined ? { overwrite: options.overwrite } : {}),
      node_path: nodePath,
    },
  }
}

export type ExportWorkflowCodeSourceRequest = {
  ExportWorkflowCodeSource: {
    session_id: string
    target: WorkflowCodeSourceExportTarget
    format?: WorkflowCodeSourceExportFormat
    agent_mode?: WorkflowCodeSourceExportAgentMode
  }
}

export function exportWorkflowCodeSourceRequest(
  sessionId: string,
  target: WorkflowCodeSourceExportTarget,
  format: WorkflowCodeSourceExportFormat = "inline",
  agentMode: WorkflowCodeSourceExportAgentMode = "portable_generated",
): ExportWorkflowCodeSourceRequest {
  return {
    ExportWorkflowCodeSource: {
      session_id: sessionId,
      target,
      format,
      agent_mode: agentMode,
    },
  }
}

export function applyWorkflowDesignOpRequest(
  sessionId: string,
  originClientId: string,
  opId: string,
  op: WorkflowDesignOp,
) {
  return {
    ApplyWorkflowDesignOp: {
      session_id: sessionId,
      origin_client_id: originClientId,
      op_id: opId,
      op,
    },
  }
}

export function aliasWorkflowRequest(sessionId: string, workflowId: string, alias: string) {
  return {
    AliasWorkflow: {
      session_id: sessionId,
      workflow_ref: workflowId,
      alias,
    },
  }
}

export function listWorkflowsRequest(sessionId: string) {
  return {
    ListWorkflows: {
      session_id: sessionId,
    },
  }
}

export function resolveWorkflowRequest(sessionId: string, workflowRef: string) {
  return {
    ResolveWorkflow: {
      session_id: sessionId,
      workflow_ref: workflowRef,
    },
  }
}

export type CreateWorkflowPublicationOptions = {
  alias?: string | null
  queueRef?: string | null
  route?: string | null
  methods?: string[]
  transport?: unknown | null
  parser?: unknown | null
  inputSchema?: unknown | null
  traceExposure?: unknown | null
  mode?: string | null
  syncTimeoutMs?: number | null
  pollMs?: number | null
  localPort?: number | null
}

export function createWorkflowPublicationRequest(
  sessionId: string,
  workflowRef: string,
  endpointRef: string,
  options: CreateWorkflowPublicationOptions = {},
) {
  return {
    CreateWorkflowPublication: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      endpoint_ref: endpointRef,
      queue_ref: options.queueRef ?? null,
      alias: options.alias ?? null,
      route: options.route ?? null,
      methods: options.methods ?? [],
      transport: options.transport ?? null,
      parser: options.parser ?? null,
      input_schema: options.inputSchema ?? null,
      trace_exposure: options.traceExposure ?? null,
      mode: options.mode ?? null,
      sync_timeout_ms: options.syncTimeoutMs ?? null,
      poll_ms: options.pollMs ?? null,
      local_port: options.localPort ?? null,
    },
  }
}

export function listWorkflowPublicationsRequest(sessionId: string) {
  return {
    ListWorkflowPublications: {
      session_id: sessionId,
    },
  }
}

export function getWorkflowPublicationRequest(sessionId: string, publicationRef: string) {
  return {
    GetWorkflowPublication: {
      session_id: sessionId,
      publication_ref: publicationRef,
    },
  }
}

export function exportWorkflowPublicationPackageRequest(
  sessionId: string,
  publicationRef: string,
  options: {
    kernelUrl?: string | null
    agentApp?: Record<string, unknown> | null
    agentAppAssetsDir?: string | null
  } = {},
) {
  return {
    ExportWorkflowPublicationPackage: {
      session_id: sessionId,
      publication_ref: publicationRef,
      kernel_url: options.kernelUrl ?? null,
      agent_app: options.agentApp ?? null,
      agent_app_assets_dir: options.agentAppAssetsDir ?? null,
    },
  }
}

export function disableWorkflowPublicationRequest(sessionId: string, publicationRef: string) {
  return {
    DisableWorkflowPublication: {
      session_id: sessionId,
      publication_ref: publicationRef,
    },
  }
}

export function registerWorkflowPublicationEndpointRequest(
  sessionId: string,
  publicationRef: string,
  localUrl: string,
  options: {
    runtimeSessionId?: string | null
    ttlMs?: number | null
  } = {},
) {
  return {
    RegisterWorkflowPublicationEndpoint: {
      session_id: sessionId,
      publication_ref: publicationRef,
      local_url: localUrl,
      runtime_session_id: options.runtimeSessionId ?? null,
      ttl_ms: options.ttlMs ?? null,
    },
  }
}

export function materializeWorkflowPublicationRequest(
  publicationId: string,
  snapshot: WorkflowPublicationSnapshot,
) {
  return {
    MaterializeWorkflowPublication: {
      publication_id: publicationId,
      snapshot,
    },
  }
}

export function createWorkflowEndpointRequest(
  sessionId: string,
  workflowRef: string,
  entryNodeId: string,
  alias?: string | null,
) {
  return {
    CreateWorkflowEndpoint: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      entry_node_id: entryNodeId,
      alias: alias ?? null,
    },
  }
}

export function aliasWorkflowEndpointRequest(
  sessionId: string,
  workflowRef: string,
  endpointRef: string,
  alias: string,
) {
  return {
    AliasWorkflowEndpoint: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      endpoint_ref: endpointRef,
      alias,
    },
  }
}

export function bindWorkflowEndpointRequest(
  sessionId: string,
  workflowRef: string,
  endpointRef: string,
  entryNodeId: string,
) {
  return {
    BindWorkflowEndpoint: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      endpoint_ref: endpointRef,
      entry_node_id: entryNodeId,
    },
  }
}

export function addWorkflowNodeRequest(sessionId: string, workflowRef: string, agentId: string) {
  return {
    AddWorkflowNode: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      agent_id: agentId,
    },
  }
}

export function removeWorkflowNodeRequest(sessionId: string, workflowRef: string, nodeId: string) {
  return {
    RemoveWorkflowNode: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
    },
  }
}

export function updateWorkflowNodeInstructionsRequest(
  sessionId: string,
  workflowRef: string,
  nodeId: string,
  instructions: string | null,
) {
  return {
    UpdateWorkflowNodeInstructions: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      instructions,
    },
  }
}

export function setWorkflowNodeCanCompleteRunRequest(
  sessionId: string,
  workflowRef: string,
  nodeId: string,
  canCompleteWorkflowRun: boolean,
) {
  return {
    SetWorkflowNodeCanCompleteRun: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      can_complete_workflow_run: canCompleteWorkflowRun,
    },
  }
}

export function setWorkflowNodeMaxTurnsRequest(
  sessionId: string,
  workflowRef: string,
  nodeId: string,
  maxTurns: number | null,
) {
  return {
    SetWorkflowNodeMaxTurns: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      max_turns: maxTurns,
    },
  }
}

export function setWorkflowNodeCanEmitIntermediateOutputRequest(
  sessionId: string,
  workflowRef: string,
  nodeId: string,
  canEmitIntermediateWorkflowRunOutput: boolean,
) {
  return {
    SetWorkflowNodeCanEmitIntermediateOutput: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      can_emit_intermediate_workflow_run_output: canEmitIntermediateWorkflowRunOutput,
    },
  }
}

export function setWorkflowNodeWaitForAllInputsRequest(
  sessionId: string,
  workflowRef: string,
  nodeId: string,
  waitForAllInputs: boolean,
) {
  return {
    SetWorkflowNodeWaitForAllInputs: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      wait_for_all_inputs: waitForAllInputs,
    },
  }
}

export function setWorkflowNodeIntermediateOutputSchemaRequest(
  sessionId: string,
  workflowRef: string,
  nodeId: string,
  intermediateOutputSchemaRef: string | null,
) {
  return {
    SetWorkflowNodeIntermediateOutputSchema: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      intermediate_output_schema_ref: intermediateOutputSchemaRef,
    },
  }
}

export function addWorkflowEdgeRequest(
  sessionId: string,
  workflowRef: string,
  fromNodeId: string,
  toNodeId: string,
  handoffSchemaRef?: string | null,
  sourceSide?: "top" | "right" | "bottom" | "left" | null,
  targetSide?: "top" | "right" | "bottom" | "left" | null,
) {
  return {
    AddWorkflowEdge: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      from_node_id: fromNodeId,
      to_node_id: toNodeId,
      ...(handoffSchemaRef ? { handoff_schema_ref: handoffSchemaRef } : {}),
      ...(sourceSide ? { source_side: sourceSide } : {}),
      ...(targetSide ? { target_side: targetSide } : {}),
    },
  }
}

export function removeWorkflowEdgeRequest(sessionId: string, workflowRef: string, edgeId: string) {
  return {
    RemoveWorkflowEdge: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      edge_id: edgeId,
    },
  }
}

export type WorkflowCanvasLayoutPatch =
  | { kind: "node_position"; node_id: string; x: number; y: number }
  | { kind: "endpoint_position"; endpoint_id: string; x: number; y: number }
  | { kind: "edge_waypoints"; edge_id: string; waypoints: readonly { readonly x: number; readonly y: number }[] }

export function updateWorkflowCanvasLayoutRequest(
  sessionId: string,
  workflowRef: string,
  patches: readonly WorkflowCanvasLayoutPatch[],
  baseLayoutRevision?: number | null,
) {
  return {
    UpdateWorkflowCanvasLayout: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      base_layout_revision: baseLayoutRevision ?? null,
      patches,
    },
  }
}

export function invokeWorkflowEndpointRequest(
  sessionId: string,
  workflowRef: string,
  endpointRef: string,
  prompt?: string | null,
  queueRef?: string | null,
  publicationInvocation?: Record<string, unknown> | null,
) {
  const payload: Record<string, unknown> = {
    session_id: sessionId,
    workflow_ref: workflowRef,
    endpoint_ref: endpointRef,
    queue_ref: queueRef ?? null,
    prompt: prompt ?? null,
  }
  if (publicationInvocation) {
    payload.publication_invocation = publicationInvocation
  }
  return {
    InvokeWorkflowEndpoint: {
      ...payload,
    },
  }
}

export function createWorkflowWatchdogRequest(
  sessionId: string,
  workflowRef: string,
  endpointRef: string,
  intervalSeconds: number,
  invocationPrompt: string,
  policy: "skip" | "queue",
  maxWakeups?: number | null,
) {
  return {
    CreateWorkflowWatchdog: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      endpoint_ref: endpointRef,
      interval_seconds: intervalSeconds,
      invocation_prompt: invocationPrompt,
      policy,
      max_wakeups_configured: maxWakeups !== undefined,
      max_wakeups: maxWakeups ?? null,
    },
  }
}

export function createWorkflowScheduleRequest(
  sessionId: string,
  workflowRef: string,
  endpointRef: string,
  trigger: WorkflowScheduleTrigger,
  invocationPrompt: string,
  overlapPolicy: "skip" | "queue",
  maxRuns?: number | null,
  queueRef?: string | null,
) {
  return {
    CreateWorkflowSchedule: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      endpoint_ref: endpointRef,
      queue_ref: queueRef ?? null,
      trigger,
      invocation_prompt: invocationPrompt,
      overlap_policy: overlapPolicy,
      max_runs_configured: maxRuns !== undefined,
      max_runs: maxRuns ?? null,
    },
  }
}

export function setWorkflowFlushContextRequest(
  sessionId: string,
  workflowRef: string,
  flushAgentContextBeforeRun: boolean,
) {
  return {
    SetWorkflowFlushContext: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      flush_agent_context_before_run: flushAgentContextBeforeRun,
    },
  }
}

export function setWorkflowRunOutputSchemaRequest(
  sessionId: string,
  workflowRef: string,
  runOutputSchemaRef: string | null,
) {
  return {
    SetWorkflowRunOutputSchema: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      run_output_schema_ref: runOutputSchemaRef,
    },
  }
}

export function listWorkflowWatchdogsRequest(sessionId: string, workflowRef?: string | null) {
  return {
    ListWorkflowWatchdogs: {
      session_id: sessionId,
      workflow_ref: workflowRef ?? null,
    },
  }
}

export function listWorkflowSchedulesRequest(sessionId: string, workflowRef?: string | null) {
  return {
    ListWorkflowSchedules: {
      session_id: sessionId,
      workflow_ref: workflowRef ?? null,
    },
  }
}

export function setWorkflowWatchdogEnabledRequest(
  sessionId: string,
  watchdogRef: string,
  enabled: boolean,
) {
  return {
    SetWorkflowWatchdogEnabled: {
      session_id: sessionId,
      watchdog_ref: watchdogRef,
      enabled,
    },
  }
}

export function setWorkflowScheduleEnabledRequest(
  sessionId: string,
  scheduleRef: string,
  enabled: boolean,
) {
  return {
    SetWorkflowScheduleEnabled: {
      session_id: sessionId,
      schedule_ref: scheduleRef,
      enabled,
    },
  }
}

export function removeWorkflowWatchdogRequest(sessionId: string, watchdogRef: string) {
  return {
    RemoveWorkflowWatchdog: {
      session_id: sessionId,
      watchdog_ref: watchdogRef,
    },
  }
}

export function removeWorkflowScheduleRequest(sessionId: string, scheduleRef: string) {
  return {
    RemoveWorkflowSchedule: {
      session_id: sessionId,
      schedule_ref: scheduleRef,
    },
  }
}

export function previewWorkflowScheduleRequest(
  trigger: WorkflowScheduleTrigger,
  afterMs?: number | null,
  count?: number | null,
) {
  return {
    PreviewWorkflowSchedule: {
      trigger,
      after_ms: afterMs ?? null,
      count: count ?? null,
    },
  }
}

export function listWorkflowPromptQueuesRequest(sessionId: string, workflowRef?: string | null) {
  return {
    ListWorkflowPromptQueues: {
      session_id: sessionId,
      workflow_ref: workflowRef ?? null,
    },
  }
}

export function createWorkflowPromptQueueRequest(
  sessionId: string,
  workflowRef: string | null,
  alias: string,
  priority: number,
) {
  return {
    CreateWorkflowPromptQueue: {
      session_id: sessionId,
      workflow_ref: workflowRef ?? null,
      alias,
      priority,
    },
  }
}

export function updateWorkflowPromptQueueRequest(
  sessionId: string,
  workflowRef: string | null,
  queueRef: string,
  patch: { alias?: string | null; priority?: number | null; enabled?: boolean | null },
) {
  return {
    UpdateWorkflowPromptQueue: {
      session_id: sessionId,
      workflow_ref: workflowRef ?? null,
      queue_ref: queueRef,
      alias: patch.alias ?? null,
      priority: patch.priority ?? null,
      enabled: patch.enabled ?? null,
    },
  }
}

export function removeWorkflowPromptQueueRequest(
  sessionId: string,
  workflowRef: string | null,
  queueRef: string,
) {
  return {
    RemoveWorkflowPromptQueue: {
      session_id: sessionId,
      workflow_ref: workflowRef ?? null,
      queue_ref: queueRef,
    },
  }
}

export function listQueuedWorkflowPromptsRequest(sessionId: string) {
  return {
    ListQueuedWorkflowPrompts: {
      session_id: sessionId,
    },
  }
}

export function updateQueuedWorkflowPromptRequest(
  sessionId: string,
  queueItemRef: string,
  patch: { prompt?: string | null; queueRef?: string | null },
) {
  return {
    UpdateQueuedWorkflowPrompt: {
      session_id: sessionId,
      queue_item_ref: queueItemRef,
      prompt: patch.prompt ?? null,
      queue_ref: patch.queueRef ?? null,
    },
  }
}

export function removeQueuedWorkflowPromptRequest(sessionId: string, queueItemRef: string) {
  return {
    RemoveQueuedWorkflowPrompt: {
      session_id: sessionId,
      queue_item_ref: queueItemRef,
    },
  }
}

export function clearWorkflowPromptQueueRequest(
  sessionId: string,
  workflowRef: string | null,
  queueRef: string,
) {
  return {
    ClearWorkflowPromptQueue: {
      session_id: sessionId,
      workflow_ref: workflowRef ?? null,
      queue_ref: queueRef,
    },
  }
}

export function listWorkflowRunsRequest(sessionId: string, workflowRef?: string | null) {
  return {
    ListWorkflowRuns: {
      session_id: sessionId,
      workflow_ref: workflowRef ?? null,
    },
  }
}

export function getWorkflowRunRequest(sessionId: string, workflowRunRef: string) {
  return {
    GetWorkflowRun: {
      session_id: sessionId,
      workflow_run_ref: workflowRunRef,
    },
  }
}

export function cancelWorkflowRunRequest(sessionId: string, workflowRunRef: string) {
  return {
    CancelWorkflowRun: {
      session_id: sessionId,
      workflow_run_ref: workflowRunRef,
    },
  }
}

export function resumeWorkflowRunRequest(sessionId: string, workflowRunRef: string) {
  return {
    ResumeWorkflowRun: {
      session_id: sessionId,
      workflow_run_ref: workflowRunRef,
    },
  }
}

export function ackWorkflowTurnRequest(
  sessionId: string,
  workflowRunRef: string,
  workflowNodeRunId: string,
  deliveryToken: string,
) {
  return {
    AckWorkflowTurn: {
      session_id: sessionId,
      workflow_run_ref: workflowRunRef,
      workflow_node_run_id: workflowNodeRunId,
      delivery_token: deliveryToken,
    },
  }
}
