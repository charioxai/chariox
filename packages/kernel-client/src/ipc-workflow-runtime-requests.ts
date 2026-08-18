import type {
  WorkflowPublicationSnapshot,
  WorkflowScheduleTrigger,
} from "./kernel-types.js"

export type CreateWorkflowPublicationOptions = {
  expectedWorkflowRevision?: number | null
  operationKey?: string | null
  alias?: string | null
  queueRef?: string | null
  kind?: "ingress" | "schedule_only" | "event_based" | string | null
  route?: string | null
  methods?: string[]
  transport?: unknown | null
  parser?: unknown | null
  inputSchema?: unknown | null
  traceExposure?: unknown | null
  mode?: string | null
  syncTimeoutMs?: number | null
  pollMs?: number | null
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
      expected_workflow_revision: options.expectedWorkflowRevision ?? null,
      operation_key: options.operationKey ?? null,
      queue_ref: options.queueRef ?? null,
      alias: options.alias ?? null,
      kind: options.kind ?? null,
      route: options.route ?? null,
      methods: options.methods ?? [],
      transport: options.transport ?? null,
      parser: options.parser ?? null,
      input_schema: options.inputSchema ?? null,
      trace_exposure: options.traceExposure ?? null,
      mode: options.mode ?? null,
      sync_timeout_ms: options.syncTimeoutMs ?? null,
      poll_ms: options.pollMs ?? null,
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

export type WorkflowPublicationRuntimeControlAction = "start" | "stop" | "restart" | "inspect"

export function controlWorkflowPublicationRuntimeRequest(
  sessionId: string,
  publicationRef: string,
  action: WorkflowPublicationRuntimeControlAction,
  options: {
    host?: string | null
    port?: number | null
    kernelUrl?: string | null
  } = {},
) {
  return {
    ControlWorkflowPublicationRuntime: {
      session_id: sessionId,
      publication_ref: publicationRef,
      action,
      host: options.host ?? null,
      port: options.port ?? null,
      kernel_url: options.kernelUrl ?? null,
    },
  }
}

export function bindWorkflowPublicationDeploymentRequest(
  sessionId: string,
  publicationRef: string,
  input: {
    setupId: string
    operationKey: string
    deploymentId: string
    releaseId: string
    packageDigest: string
    desiredRevision: number
  },
) {
  return {
    BindWorkflowPublicationDeployment: {
      session_id: sessionId,
      publication_ref: publicationRef,
      setup_id: input.setupId,
      operation_key: input.operationKey,
      deployment_id: input.deploymentId,
      release_id: input.releaseId,
      package_digest: input.packageDigest,
      desired_revision: input.desiredRevision,
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

export function listWorkflowRunsRequest(
  sessionId: string,
  workflowRef?: string | null,
  page: { cursor?: string | null; limit?: number | null } = {},
) {
  return {
    ListWorkflowRuns: {
      session_id: sessionId,
      workflow_ref: workflowRef ?? null,
      ...(page.cursor ? { cursor: page.cursor } : {}),
      ...(page.limit == null ? {} : { limit: page.limit }),
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

export function pauseWorkflowRunRequest(sessionId: string, workflowRunRef: string) {
  return {
    PauseWorkflowRun: {
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
