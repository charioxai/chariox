export function createSessionRequest(workspaceId, worktreeId, alias) {
  return {
    CreateSession: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      alias: alias ?? null
    }
  };
}
export function listSessionsRequest() {
  return {
    ListSessions: null
  };
}
export function resolveSessionRequest(sessionRef, workspaceId) {
  return {
    ResolveSession: {
      session_ref: sessionRef,
      workspace_id: workspaceId ?? null
    }
  };
}
export function attachToSessionRequest(sessionId, clientId) {
  return {
    AttachToSession: {
      session_id: sessionId,
      client_id: clientId,
      capability_level: "FullTerminal"
    }
  };
}
export function detachFromSessionRequest(attachmentId) {
  return {
    DetachFromSession: {
      attachment_id: attachmentId
    }
  };
}
export function endSessionRequest(sessionId) {
  return {
    EndSession: {
      session_id: sessionId
    }
  };
}
export function deleteSessionRequest(sessionRef, workspaceId) {
  return {
    DeleteSession: {
      session_ref: sessionRef,
      workspace_id: workspaceId ?? null
    }
  };
}
export function aliasSessionRequest(sessionId, alias) {
  return {
    AliasSession: {
      session_id: sessionId,
      alias
    }
  };
}
export function getSessionStateRequest(sessionId) {
  return {
    GetSessionState: {
      session_id: sessionId
    }
  };
}
export function getDaemonHealthRequest() {
  return {
    GetDaemonHealth: null
  };
}
export function listProviderProcessesRequest(provider) {
  return {
    ListProviderProcesses: {
      provider: provider ?? null
    }
  };
}
export function teardownProviderProcessesRequest(provider) {
  return {
    TeardownProviderProcesses: {
      provider: provider ?? null
    }
  };
}
export function createWorkflowRequest(sessionId, alias) {
  return {
    CreateWorkflow: {
      session_id: sessionId,
      alias: alias ?? null
    }
  };
}
export function aliasWorkflowRequest(sessionId, workflowId, alias) {
  return {
    AliasWorkflow: {
      session_id: sessionId,
      workflow_ref: workflowId,
      alias
    }
  };
}
export function listWorkflowsRequest(sessionId) {
  return {
    ListWorkflows: {
      session_id: sessionId
    }
  };
}
export function resolveWorkflowRequest(sessionId, workflowRef) {
  return {
    ResolveWorkflow: {
      session_id: sessionId,
      workflow_ref: workflowRef
    }
  };
}
export function createWorkflowEndpointRequest(sessionId, workflowRef, entryNodeId, alias) {
  return {
    CreateWorkflowEndpoint: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      entry_node_id: entryNodeId,
      alias: alias ?? null
    }
  };
}
export function aliasWorkflowEndpointRequest(sessionId, workflowRef, endpointRef, alias) {
  return {
    AliasWorkflowEndpoint: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      endpoint_ref: endpointRef,
      alias
    }
  };
}
export function bindWorkflowEndpointRequest(sessionId, workflowRef, endpointRef, entryNodeId) {
  return {
    BindWorkflowEndpoint: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      endpoint_ref: endpointRef,
      entry_node_id: entryNodeId
    }
  };
}
export function addWorkflowNodeRequest(sessionId, workflowRef, agentId) {
  return {
    AddWorkflowNode: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      agent_id: agentId
    }
  };
}
export function removeWorkflowNodeRequest(sessionId, workflowRef, nodeId) {
  return {
    RemoveWorkflowNode: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId
    }
  };
}
export function updateWorkflowNodeInstructionsRequest(sessionId, workflowRef, nodeId, instructions) {
  return {
    UpdateWorkflowNodeInstructions: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      instructions
    }
  };
}
export function setWorkflowNodeCanCompleteRunRequest(sessionId, workflowRef, nodeId, canCompleteWorkflowRun) {
  return {
    SetWorkflowNodeCanCompleteRun: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      can_complete_workflow_run: canCompleteWorkflowRun
    }
  };
}
export function setWorkflowNodeMaxTurnsRequest(sessionId, workflowRef, nodeId, maxTurns) {
  return {
    SetWorkflowNodeMaxTurns: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      max_turns: maxTurns
    }
  };
}
export function setWorkflowNodeCanEmitIntermediateOutputRequest(sessionId, workflowRef, nodeId, canEmitIntermediateWorkflowRunOutput) {
  return {
    SetWorkflowNodeCanEmitIntermediateOutput: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      can_emit_intermediate_workflow_run_output: canEmitIntermediateWorkflowRunOutput
    }
  };
}
export function setWorkflowNodeIntermediateOutputSchemaRequest(sessionId, workflowRef, nodeId, intermediateOutputSchemaRef) {
  return {
    SetWorkflowNodeIntermediateOutputSchema: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      intermediate_output_schema_ref: intermediateOutputSchemaRef
    }
  };
}
export function addWorkflowEdgeRequest(sessionId, workflowRef, fromNodeId, toNodeId) {
  return {
    AddWorkflowEdge: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      from_node_id: fromNodeId,
      to_node_id: toNodeId
    }
  };
}
export function removeWorkflowEdgeRequest(sessionId, workflowRef, edgeId) {
  return {
    RemoveWorkflowEdge: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      edge_id: edgeId
    }
  };
}
export function invokeWorkflowEndpointRequest(sessionId, workflowRef, endpointRef, prompt) {
  return {
    InvokeWorkflowEndpoint: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      endpoint_ref: endpointRef,
      prompt: prompt ?? null
    }
  };
}
export function createWorkflowWatchdogRequest(sessionId, workflowRef, endpointRef, intervalSeconds, invocationPrompt, policy, maxWakeups) {
  return {
    CreateWorkflowWatchdog: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      endpoint_ref: endpointRef,
      interval_seconds: intervalSeconds,
      invocation_prompt: invocationPrompt,
      policy,
      max_wakeups_configured: maxWakeups !== undefined,
      max_wakeups: maxWakeups ?? null
    }
  };
}
export function setWorkflowFlushContextRequest(sessionId, workflowRef, flushAgentContextBeforeRun) {
  return {
    SetWorkflowFlushContext: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      flush_agent_context_before_run: flushAgentContextBeforeRun
    }
  };
}
export function setWorkflowRunOutputSchemaRequest(sessionId, workflowRef, runOutputSchemaRef) {
  return {
    SetWorkflowRunOutputSchema: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      run_output_schema_ref: runOutputSchemaRef
    }
  };
}
export function setWorkflowIntermediateOutputSchemaRequest(sessionId, workflowRef, intermediateOutputSchemaRef) {
  return {
    SetWorkflowIntermediateOutputSchema: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      intermediate_output_schema_ref: intermediateOutputSchemaRef
    }
  };
}
export function listWorkflowWatchdogsRequest(sessionId, workflowRef) {
  return {
    ListWorkflowWatchdogs: {
      session_id: sessionId,
      workflow_ref: workflowRef ?? null
    }
  };
}
export function setWorkflowWatchdogEnabledRequest(sessionId, watchdogRef, enabled) {
  return {
    SetWorkflowWatchdogEnabled: {
      session_id: sessionId,
      watchdog_ref: watchdogRef,
      enabled
    }
  };
}
export function removeWorkflowWatchdogRequest(sessionId, watchdogRef) {
  return {
    RemoveWorkflowWatchdog: {
      session_id: sessionId,
      watchdog_ref: watchdogRef
    }
  };
}
export function setWorkflowLaunchPolicyRequest(sessionId, policy) {
  return {
    SetWorkflowLaunchPolicy: {
      session_id: sessionId,
      policy
    }
  };
}
export function listQueuedWorkflowLaunchesRequest(sessionId) {
  return {
    ListQueuedWorkflowLaunches: {
      session_id: sessionId
    }
  };
}
export function removeQueuedWorkflowLaunchRequest(sessionId, queueItemRef) {
  return {
    RemoveQueuedWorkflowLaunch: {
      session_id: sessionId,
      queue_item_ref: queueItemRef
    }
  };
}
export function clearQueuedWorkflowLaunchesRequest(sessionId) {
  return {
    ClearQueuedWorkflowLaunches: {
      session_id: sessionId
    }
  };
}
export function listWorkflowRunsRequest(sessionId, workflowRef) {
  return {
    ListWorkflowRuns: {
      session_id: sessionId,
      workflow_ref: workflowRef ?? null
    }
  };
}
export function getWorkflowRunRequest(sessionId, workflowRunRef) {
  return {
    GetWorkflowRun: {
      session_id: sessionId,
      workflow_run_ref: workflowRunRef
    }
  };
}
export function cancelWorkflowRunRequest(sessionId, workflowRunRef) {
  return {
    CancelWorkflowRun: {
      session_id: sessionId,
      workflow_run_ref: workflowRunRef
    }
  };
}
export function resumeWorkflowRunRequest(sessionId, workflowRunRef) {
  return {
    ResumeWorkflowRun: {
      session_id: sessionId,
      workflow_run_ref: workflowRunRef
    }
  };
}
export function updateSessionConfigRequest(sessionId, attachmentId, values, requiresIdle = false) {
  return {
    UpdateSessionConfig: {
      session_id: sessionId,
      attachment_id: attachmentId,
      values,
      requires_idle: requiresIdle
    }
  };
}
export function getProviderRunRequest(providerRunId) {
  return {
    GetProviderRun: {
      provider_run_id: providerRunId
    }
  };
}
export function getProviderCatalogRequest() {
  return {
    GetProviderCatalog: null
  };
}
export function relayStatusRequest() {
  return {
    RelayStatus: null
  };
}
export function configureRelayRequest(relayUrl, relayToken) {
  return {
    ConfigureRelay: {
      relay_url: relayUrl,
      relay_token: relayToken
    }
  };
}
export function listRemoteMachinesRequest() {
  return {
    ListRemoteMachines: null
  };
}
export function listRemoteMachineKernelsRequest(machineRef) {
  return {
    ListRemoteMachineKernels: {
      machine_ref: machineRef
    }
  };
}
export function approveRemoteMachineRequest(machineRef) {
  return {
    ApproveRemoteMachine: {
      machine_ref: machineRef
    }
  };
}
export function forgetRemoteMachineRequest(machineRef) {
  return {
    ForgetRemoteMachine: {
      machine_ref: machineRef
    }
  };
}
export function renameRemoteMachineRequest(machineRef, alias) {
  return {
    RenameRemoteMachine: {
      machine_ref: machineRef,
      alias
    }
  };
}
export function getProviderCommandCatalogsRequest() {
  return {
    GetProviderCommandCatalogs: null
  };
}
export function getProviderAuthStatusRequest(provider) {
  return {
    GetProviderAuthStatus: {
      provider
    }
  };
}
export function startProviderLoginRequest(provider) {
  return {
    StartProviderLogin: {
      provider
    }
  };
}
export function logoutProviderRequest(provider) {
  return {
    LogoutProvider: {
      provider
    }
  };
}
export function readDirectoryTreeRequest(sessionId, attachmentId, treePath, maxDepth) {
  return {
    ReadDirectoryTree: {
      session_id: sessionId,
      attachment_id: attachmentId,
      path: treePath,
      max_depth: maxDepth
    }
  };
}
export function getSessionHistoryRequest(sessionId, roundCount, maxChars, cursor, agentId) {
  return {
    GetSessionHistory: {
      session_id: sessionId,
      agent_id: agentId ?? null,
      round_count: roundCount,
      max_chars: maxChars,
      before_entry_index: cursor?.before_entry_index ?? null,
      before_entry_char_offset: cursor?.before_entry_char_offset ?? null
    }
  };
}
export function launchProviderRunRequest(sessionId, provider, accountProfile, model, effort, agentId) {
  const normalizedModel = provider === "codex" && model.startsWith("codex/") ? model.slice("codex/".length) : model;
  return {
    LaunchProviderRun: {
      session_id: sessionId,
      agent_id: agentId ?? null,
      adapter_key: provider,
      provider,
      account_profile: accountProfile,
      model: normalizedModel,
      variant: effort.trim() || null
    }
  };
}
export function captureScreenshotRequest(sessionId, attachmentId) {
  return {
    CaptureScreenshot: {
      session_id: sessionId,
      attachment_id: attachmentId
    }
  };
}
export function storeTransferredFileRequest(sessionId, attachmentId, sourcePath, displayName) {
  return {
    StoreTransferredFile: {
      session_id: sessionId,
      attachment_id: attachmentId,
      source_path: sourcePath,
      display_name: displayName ?? null
    }
  };
}
export function resizeTerminalRequest(sessionId, cols, rows) {
  return {
    ResizeTerminal: {
      session_id: sessionId,
      cols,
      rows
    }
  };
}
export function pumpTerminalOutputRequest(sessionId, attachmentId) {
  return {
    PumpTerminalOutput: {
      session_id: sessionId,
      attachment_id: attachmentId
    }
  };
}
export function submitPromptRequest(sessionId, attachmentId, targetAgentId, prompt, attachments) {
  return {
    SubmitPrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
      target_agent_id: targetAgentId,
      prompt,
      attachments
    }
  };
}
export function completePromptRequest(sessionId) {
  return {
    CompletePrompt: {
      session_id: sessionId
    }
  };
}
export function cancelActivePromptRequest(sessionId, attachmentId) {
  return {
    CancelActivePrompt: {
      session_id: sessionId,
      attachment_id: attachmentId
    }
  };
}
export function ackWorkflowTurnRequest(sessionId, workflowRunRef, workflowNodeRunId, deliveryToken) {
  return {
    AckWorkflowTurn: {
      session_id: sessionId,
      workflow_run_ref: workflowRunRef,
      workflow_node_run_id: workflowNodeRunId,
      delivery_token: deliveryToken
    }
  };
}
export function pollRuntimeNoticesRequest(sessionId, attachmentId) {
  return {
    PollRuntimeNotices: {
      session_id: sessionId,
      attachment_id: attachmentId
    }
  };
}
export function spawnAgentRequest(sessionId, provider, alias, model, worktreeId, effort) {
  return {
    SpawnAgent: {
      session_id: sessionId,
      provider,
      alias: alias ?? null,
      model: model ?? null,
      effort: effort ?? null,
      worktree_id: worktreeId ?? null
    }
  };
}
export function destroyAgentRequest(sessionId, agentId) {
  return {
    DestroyAgent: {
      session_id: sessionId,
      agent_id: agentId
    }
  };
}
export function focusAgentRequest(sessionId, agentId) {
  return {
    FocusAgent: {
      session_id: sessionId,
      agent_id: agentId
    }
  };
}
export function cycleAgentFocusRequest(sessionId) {
  return {
    CycleAgentFocus: {
      session_id: sessionId
    }
  };
}
export function listAgentsRequest(sessionId) {
  return {
    ListAgents: {
      session_id: sessionId
    }
  };
}