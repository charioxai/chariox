import type { HistoryQueryPayload, PromptAttachmentPart, SessionHistoryCursor } from "./kernel-types.js"

export function createSessionRequest(workspaceId: string, worktreeId: string, alias?: string) {
  return {
    CreateSession: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      alias: alias ?? null,
    },
  }
}

export function listSessionsRequest() {
  return { ListSessions: null }
}

export function resolveSessionRequest(sessionRef: string, workspaceId?: string) {
  return {
    ResolveSession: {
      session_ref: sessionRef,
      workspace_id: workspaceId ?? null,
    },
  }
}

export function attachToSessionRequest(sessionId: string, clientId: string) {
  return {
    AttachToSession: {
      session_id: sessionId,
      client_id: clientId,
      capability_level: "FullTerminal",
    },
  }
}

export function detachFromSessionRequest(attachmentId: string) {
  return {
    DetachFromSession: {
      attachment_id: attachmentId,
    },
  }
}

export function listSessionMembersRequest(sessionId: string) {
  return {
    ListSessionMembers: {
      session_id: sessionId,
    },
  }
}

export function createSessionInviteRequest(
  sessionId: string,
  expiresInMs: number | null = null,
  maxUses: number | null = null,
) {
  return {
    CreateSessionInvite: {
      session_id: sessionId,
      expires_in_ms: expiresInMs,
      max_uses: maxUses,
    },
  }
}

export function joinSessionInviteRequest(inviteToken: string, userId: string) {
  return {
    JoinSessionInvite: {
      invite_token: inviteToken,
      user_id: userId,
    },
  }
}

export function revokeSessionInviteRequest(sessionId: string, inviteRef: string) {
  return {
    RevokeSessionInvite: {
      session_id: sessionId,
      invite_ref: inviteRef,
    },
  }
}

export function createWorkspaceLinkRequest(sessionId: string, name: string) {
  return {
    CreateWorkspaceLink: {
      session_id: sessionId,
      name,
    },
  }
}

export function listWorkspaceLinksRequest(sessionId: string) {
  return {
    ListWorkspaceLinks: {
      session_id: sessionId,
    },
  }
}

export function showWorkspaceLinkRequest(sessionId: string, linkRef: string) {
  return {
    ShowWorkspaceLink: {
      session_id: sessionId,
      link_ref: linkRef,
    },
  }
}

export function attachWorkspaceLinkRequest(
  sessionId: string,
  linkRef: string,
  repoRoot?: string | null,
  branch?: string | null,
  repoFingerprint?: string | null,
) {
  return {
    AttachWorkspaceLink: {
      session_id: sessionId,
      link_ref: linkRef,
      repo_root: repoRoot ?? null,
      branch: branch ?? null,
      repo_fingerprint: repoFingerprint ?? null,
    },
  }
}

export function detachWorkspaceLinkRequest(sessionId: string, linkRef: string, repoRoot?: string | null) {
  return {
    DetachWorkspaceLink: {
      session_id: sessionId,
      link_ref: linkRef,
      repo_root: repoRoot ?? null,
    },
  }
}

export function endSessionRequest(sessionId: string) {
  return {
    EndSession: {
      session_id: sessionId,
    },
  }
}

export function deleteSessionRequest(sessionRef: string, workspaceId?: string) {
  return {
    DeleteSession: {
      session_ref: sessionRef,
      workspace_id: workspaceId ?? null,
    },
  }
}

export function aliasSessionRequest(sessionId: string, alias: string) {
  return {
    AliasSession: {
      session_id: sessionId,
      alias,
    },
  }
}

export function getSessionStateRequest(sessionId: string) {
  return {
    GetSessionState: {
      session_id: sessionId,
    },
  }
}

export function getDaemonHealthRequest() {
  return { GetDaemonHealth: null }
}


export function installMcpServerRequest(workspaceId: string | null, config: Record<string, unknown>) {
  return {
    InstallMcpServer: {
      workspace_id: workspaceId ?? null,
      config,
    },
  }
}

export function updateMcpServerRequest(workspaceId: string | null, config: Record<string, unknown>) {
  return {
    UpdateMcpServer: {
      workspace_id: workspaceId ?? null,
      config,
    },
  }
}

export function uninstallMcpServerRequest(workspaceId: string | null, name: string) {
  return {
    UninstallMcpServer: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function getMcpServerRequest(workspaceId: string | null, name: string) {
  return {
    GetMcpServer: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function importMcpServersRequest(workspaceId: string | null, provider: string, name?: string | null) {
  return {
    ImportMcpServers: {
      workspace_id: workspaceId ?? null,
      provider,
      name: name ?? null,
    },
  }
}

export function getSkillRequest(workspaceId: string | null, name: string) {
  return {
    GetSkill: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function installSkillRequest(workspaceId: string | null, sourcePath: string) {
  return {
    InstallSkill: {
      workspace_id: workspaceId ?? null,
      source_path: sourcePath,
    },
  }
}

export function updateSkillRequest(workspaceId: string | null, sourcePath: string) {
  return {
    UpdateSkill: {
      workspace_id: workspaceId ?? null,
      source_path: sourcePath,
    },
  }
}

export function uninstallSkillRequest(workspaceId: string | null, name: string) {
  return {
    UninstallSkill: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function importSkillsRequest(workspaceId: string | null, provider: string, name?: string | null) {
  return {
    ImportSkills: {
      workspace_id: workspaceId ?? null,
      provider,
      name: name ?? null,
    },
  }
}

export function grantAgentCapabilityRequest(
  workspaceId: string | null,
  agentRef: string,
  kind: "mcp" | "skill",
  name: string,
) {
  return {
    GrantAgentCapability: {
      workspace_id: workspaceId ?? null,
      agent_ref: agentRef,
      kind,
      name,
    },
  }
}

export function revokeAgentCapabilityRequest(agentRef: string, kind: "mcp" | "skill", name: string) {
  return {
    RevokeAgentCapability: {
      agent_ref: agentRef,
      kind,
      name,
    },
  }
}

export function listMcpServersRequest(workspaceId?: string | null) {
  return {
    ListMcpServers: {
      workspace_id: workspaceId ?? null,
    },
  }
}

export function listSkillsRequest(workspaceId?: string | null) {
  return {
    ListSkills: {
      workspace_id: workspaceId ?? null,
    },
  }
}

export function listProviderProcessesRequest(provider?: string | null) {
  return {
    ListProviderProcesses: {
      provider: provider ?? null,
    },
  }
}

export function teardownProviderProcessesRequest(provider?: string | null, force = false) {
  return {
    TeardownProviderProcesses: {
      provider: provider ?? null,
      force,
    },
  }
}

export function createWorkflowRequest(sessionId: string, alias?: string | null) {
  return {
    CreateWorkflow: {
      session_id: sessionId,
      alias: alias ?? null,
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
  route?: string | null
  methods?: string[]
  transport?: unknown | null
  auth?: unknown | null
  parser?: unknown | null
  inputSchema?: unknown | null
  mode?: string | null
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
      alias: options.alias ?? null,
      route: options.route ?? null,
      methods: options.methods ?? [],
      transport: options.transport ?? null,
      auth: options.auth ?? null,
      parser: options.parser ?? null,
      input_schema: options.inputSchema ?? null,
      mode: options.mode ?? null,
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

export function disableWorkflowPublicationRequest(sessionId: string, publicationRef: string) {
  return {
    DisableWorkflowPublication: {
      session_id: sessionId,
      publication_ref: publicationRef,
    },
  }
}

export function createWorkflowPublicationPairCodeRequest(
  sessionId: string,
  publicationRef: string,
  expiresInMs: number | null = null,
  maxUses: number | null = null,
) {
  return {
    CreateWorkflowPublicationPairCode: {
      session_id: sessionId,
      publication_ref: publicationRef,
      expires_in_ms: expiresInMs,
      max_uses: maxUses,
    },
  }
}

export function redeemWorkflowPublicationPairCodeRequest(
  sessionId: string,
  publicationRef: string,
  pairCode: string,
  displayName: string | null = null,
  allowedTransports: string[] = [],
  expiresInMs: number | null = null,
) {
  return {
    RedeemWorkflowPublicationPairCode: {
      session_id: sessionId,
      publication_ref: publicationRef,
      pair_code: pairCode,
      display_name: displayName,
      allowed_transports: allowedTransports,
      expires_in_ms: expiresInMs,
    },
  }
}

export function listWorkflowPublicationSendersRequest(sessionId: string, publicationRef: string) {
  return {
    ListWorkflowPublicationSenders: {
      session_id: sessionId,
      publication_ref: publicationRef,
    },
  }
}

export function revokeWorkflowPublicationSenderRequest(sessionId: string, publicationRef: string, senderRef: string) {
  return {
    RevokeWorkflowPublicationSender: {
      session_id: sessionId,
      publication_ref: publicationRef,
      sender_ref: senderRef,
    },
  }
}

export function authenticateWorkflowPublicationSenderRequest(
  sessionId: string,
  publicationRef: string,
  credential: string,
  transport: string,
) {
  return {
    AuthenticateWorkflowPublicationSender: {
      session_id: sessionId,
      publication_ref: publicationRef,
      credential,
      transport,
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
) {
  return {
    AddWorkflowEdge: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      from_node_id: fromNodeId,
      to_node_id: toNodeId,
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

export function invokeWorkflowEndpointRequest(
  sessionId: string,
  workflowRef: string,
  endpointRef: string,
  prompt?: string | null,
) {
  return {
    InvokeWorkflowEndpoint: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      endpoint_ref: endpointRef,
      prompt: prompt ?? null,
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

export function setWorkflowIntermediateOutputSchemaRequest(
  sessionId: string,
  workflowRef: string,
  intermediateOutputSchemaRef: string | null,
) {
  return {
    SetWorkflowIntermediateOutputSchema: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      intermediate_output_schema_ref: intermediateOutputSchemaRef,
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

export function removeWorkflowWatchdogRequest(sessionId: string, watchdogRef: string) {
  return {
    RemoveWorkflowWatchdog: {
      session_id: sessionId,
      watchdog_ref: watchdogRef,
    },
  }
}

export function setWorkflowLaunchPolicyRequest(
  sessionId: string,
  policy: "reject" | "queue",
) {
  return {
    SetWorkflowLaunchPolicy: {
      session_id: sessionId,
      policy,
    },
  }
}

export function listQueuedWorkflowLaunchesRequest(sessionId: string) {
  return {
    ListQueuedWorkflowLaunches: {
      session_id: sessionId,
    },
  }
}

export function removeQueuedWorkflowLaunchRequest(sessionId: string, queueItemRef: string) {
  return {
    RemoveQueuedWorkflowLaunch: {
      session_id: sessionId,
      queue_item_ref: queueItemRef,
    },
  }
}

export function clearQueuedWorkflowLaunchesRequest(sessionId: string) {
  return {
    ClearQueuedWorkflowLaunches: {
      session_id: sessionId,
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

export function updateSessionConfigRequest(
  sessionId: string,
  attachmentId: string,
  values: Record<string, string>,
  requiresIdle = false,
) {
  return {
    UpdateSessionConfig: {
      session_id: sessionId,
      attachment_id: attachmentId,
      values,
      requires_idle: requiresIdle,
    },
  }
}

export function getProviderRunRequest(providerRunId: string) {
  return {
    GetProviderRun: {
      provider_run_id: providerRunId,
    },
  }
}

export function getProviderCatalogRequest() {
  return { GetProviderCatalog: null }
}

export function relayStatusRequest() {
  return { RelayStatus: null }
}

export function configureRelayRequest(relayUrl: string | null, relayToken: string | null) {
  return {
    ConfigureRelay: {
      relay_url: relayUrl,
      relay_token: relayToken,
    },
  }
}

export function cloudRelayStatusRequest() {
  return { CloudRelayStatus: null }
}

export function startCloudRelayLoginRequest(apiUrl: string, input: {
  clientId?: string
  clientAlias?: string
  machineId?: string
  machineAlias?: string
}) {
  return {
    StartCloudRelayLogin: {
      api_url: apiUrl,
      client_id: input.clientId,
      client_alias: input.clientAlias,
      machine_id: input.machineId,
      machine_alias: input.machineAlias,
    },
  }
}

export function pollCloudRelayLoginRequest(apiUrl: string, deviceCode: string) {
  return {
    PollCloudRelayLogin: {
      api_url: apiUrl,
      device_code: deviceCode,
    },
  }
}

export function logoutCloudRelayRequest(options: { revokeClient?: boolean; revokeMachine?: boolean } = {}) {
  return {
    LogoutCloudRelay: {
      revoke_client: options.revokeClient ?? false,
      revoke_machine: options.revokeMachine ?? false,
    },
  }
}

export function pairCloudRelayClientRequest(clientId: string, alias?: string) {
  return {
    PairCloudRelayClient: {
      client_id: clientId,
      alias,
    },
  }
}

export function pairCloudRelayMachineRequest(machineId: string, alias?: string) {
  return {
    PairCloudRelayMachine: {
      machine_id: machineId,
      alias,
    },
  }
}

export function connectCloudRelayRequest() {
  return { ConnectCloudRelay: null }
}

export function issueCloudRelayClientTokenRequest(targetDaemonAlias: string, clientId: string, sessionId?: string | null) {
  return {
    IssueCloudRelayClientToken: {
      target_daemon_alias: targetDaemonAlias,
      client_id: clientId,
      session_id: sessionId ?? null,
    },
  }
}

export function createCloudSessionInviteRequest(
  sessionId: string,
  options: { displayName?: string | null; expiresInMs?: number | null; maxUses?: number | null } = {},
) {
  return {
    CreateCloudSessionInvite: {
      session_id: sessionId,
      display_name: options.displayName ?? null,
      expires_in_ms: options.expiresInMs ?? null,
      max_uses: options.maxUses ?? null,
    },
  }
}

export function showCloudSessionInviteRequest(inviteToken: string) {
  return {
    ShowCloudSessionInvite: {
      invite_token: inviteToken,
    },
  }
}

export function acceptCloudSessionInviteRequest(inviteToken: string) {
  return {
    AcceptCloudSessionInvite: {
      invite_token: inviteToken,
    },
  }
}

export function revokeCloudSessionInviteRequest(sessionId: string, inviteId: string) {
  return {
    RevokeCloudSessionInvite: {
      session_id: sessionId,
      invite_id: inviteId,
    },
  }
}

export function listCloudSessionMembersRequest(sessionId: string) {
  return {
    ListCloudSessionMembers: {
      session_id: sessionId,
    },
  }
}

export function listCloudCollaboratorsRequest() {
  return { ListCloudCollaborators: null }
}

export function getUserConfigRequest() {
  return { GetUserConfig: null }
}

export function setUserConfigValueRequest(path: string, value: string) {
  return {
    SetUserConfigValue: {
      path,
      value,
    },
  }
}

export function unsetUserConfigValueRequest(path: string) {
  return {
    UnsetUserConfigValue: {
      path,
    },
  }
}

export function setCredentialSecretRequest(key: string, value: string) {
  return {
    SetCredentialSecret: {
      key,
      value,
    },
  }
}

export function deleteCredentialSecretRequest(key: string) {
  return {
    DeleteCredentialSecret: {
      key,
    },
  }
}

export function listRemoteMachinesRequest() {
  return { ListRemoteMachines: null }
}

export function listRemoteMachineKernelsRequest(machineRef: string) {
  return {
    ListRemoteMachineKernels: {
      machine_ref: machineRef,
    },
  }
}

export function approveRemoteMachineRequest(machineRef: string) {
  return {
    ApproveRemoteMachine: {
      machine_ref: machineRef,
    },
  }
}

export function forgetRemoteMachineRequest(machineRef: string) {
  return {
    ForgetRemoteMachine: {
      machine_ref: machineRef,
    },
  }
}

export function renameRemoteMachineRequest(machineRef: string, alias: string) {
  return {
    RenameRemoteMachine: {
      machine_ref: machineRef,
      alias,
    },
  }
}

export function createPairingInviteRequest(
  intent: "client" | "machine",
  alias: string | null = null,
  expiresInMs: number | null = null,
) {
  return {
    CreatePairingInvite: {
      intent,
      alias,
      expires_in_ms: expiresInMs,
    },
  }
}

export function joinPairingInviteRequest(
  inviteToken: string,
  subjectId: string | null = null,
  publicKeyThumbprint: string | null = null,
  alias: string | null = null,
) {
  return {
    JoinPairingInvite: {
      invite_token: inviteToken,
      subject_id: subjectId,
      public_key_thumbprint: publicKeyThumbprint,
      alias,
    },
  }
}

export function listPairedClientsRequest() {
  return { ListPairedClients: null }
}

export function recordPairedClientRequest(
  clientId: string,
  publicKeyThumbprint: string,
  alias: string | null = null,
  pairedAtMs: number | null = null,
) {
  return {
    RecordPairedClient: {
      client_id: clientId,
      public_key_thumbprint: publicKeyThumbprint,
      alias,
      paired_at_ms: pairedAtMs,
    },
  }
}

export function revokePairedClientRequest(clientId: string) {
  return {
    RevokePairedClient: {
      client_id: clientId,
    },
  }
}

export function getProviderCommandCatalogsRequest() {
  return { GetProviderCommandCatalogs: null }
}

export function getProviderAuthStatusRequest(provider: string) {
  return {
    GetProviderAuthStatus: {
      provider,
    },
  }
}

export function startProviderLoginRequest(provider: string) {
  return {
    StartProviderLogin: {
      provider,
    },
  }
}

export function logoutProviderRequest(provider: string) {
  return {
    LogoutProvider: {
      provider,
    },
  }
}

export function readDirectoryTreeRequest(sessionId: string, attachmentId: string, treePath: string | null, maxDepth: number) {
  return {
    ReadDirectoryTree: {
      session_id: sessionId,
      attachment_id: attachmentId,
      path: treePath,
      max_depth: maxDepth,
    },
  }
}

export function getSessionHistoryRequest(
  sessionId: string,
  roundCount: number,
  maxChars: number,
  cursor?: SessionHistoryCursor | null,
  agentId?: string | null,
) {
  return {
    GetSessionHistory: {
      session_id: sessionId,
      agent_id: agentId ?? null,
      round_count: roundCount,
      max_chars: maxChars,
      before_entry_index: cursor?.before_entry_index ?? null,
      before_entry_char_offset: cursor?.before_entry_char_offset ?? null,
    },
  }
}

export function queryHistoryRequest(query: HistoryQueryPayload) {
  return {
    QueryHistory: {
      session_id: query.session_id ?? null,
      agent_id: query.agent_id ?? null,
      provider: query.provider ?? null,
      model: query.model ?? null,
      workflow_id: query.workflow_id ?? null,
      machine_id: query.machine_id ?? null,
      repo_root: query.repo_root ?? null,
      worktree_path: query.worktree_path ?? null,
      kind: query.kind ?? null,
      text: query.text ?? null,
      after_sequence: query.after_sequence ?? null,
      limit: query.limit ?? null,
    },
  }
}

export function searchHistoryRequest(query: string, filters: Omit<HistoryQueryPayload, "text"> = {}) {
  return {
    SearchHistory: {
      query,
      session_id: filters.session_id ?? null,
      agent_id: filters.agent_id ?? null,
      provider: filters.provider ?? null,
      model: filters.model ?? null,
      workflow_id: filters.workflow_id ?? null,
      machine_id: filters.machine_id ?? null,
      repo_root: filters.repo_root ?? null,
      worktree_path: filters.worktree_path ?? null,
      kind: filters.kind ?? null,
      after_sequence: filters.after_sequence ?? null,
      limit: filters.limit ?? null,
    },
  }
}

export function launchProviderRunRequest(
  sessionId: string,
  provider: string,
  accountProfile: string,
  model: string,
  effort: string,
  agentId?: string | null,
) {
  const normalizedModel = provider === "codex" && model.startsWith("codex/")
    ? model.slice("codex/".length)
    : model
  return {
    LaunchProviderRun: {
      session_id: sessionId,
      agent_id: agentId ?? null,
      adapter_key: provider,
      provider,
      account_profile: accountProfile,
      model: normalizedModel,
      variant: effort.trim() || null,
    },
  }
}

export function captureScreenshotRequest(sessionId: string, attachmentId: string) {
  return {
    CaptureScreenshot: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

export function storeTransferredFileRequest(sessionId: string, attachmentId: string, sourcePath: string, displayName?: string) {
  return {
    StoreTransferredFile: {
      session_id: sessionId,
      attachment_id: attachmentId,
      source_path: sourcePath,
      display_name: displayName ?? null,
    },
  }
}

export function resizeTerminalRequest(sessionId: string, cols: number, rows: number) {
  return {
    ResizeTerminal: {
      session_id: sessionId,
      cols,
      rows,
    },
  }
}

export function pumpTerminalOutputRequest(sessionId: string, attachmentId: string) {
  return {
    PumpTerminalOutput: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

export function submitPromptRequest(
  sessionId: string,
  attachmentId: string,
  targetAgentId: string | null,
  prompt: string,
  attachments: PromptAttachmentPart[],
) {
  return {
    SubmitPrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
      target_agent_id: targetAgentId,
      prompt,
      attachments,
    },
  }
}

export function completePromptRequest(sessionId: string) {
  return {
    CompletePrompt: {
      session_id: sessionId,
    },
  }
}

export function cancelActivePromptRequest(sessionId: string, attachmentId: string) {
  return {
    CancelActivePrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
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

export function pollRuntimeNoticesRequest(sessionId: string, attachmentId: string) {
  return {
    PollRuntimeNotices: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

export function spawnAgentRequest(
  sessionId: string,
  provider: string,
  alias?: string,
  model?: string,
  worktreeId?: string,
  effort?: string,
  machineRef?: string,
  worktreePlacement?: Record<string, unknown>,
) {
  return {
    SpawnAgent: {
      session_id: sessionId,
      provider,
      alias: alias ?? null,
      model: model ?? null,
      effort: effort ?? null,
      worktree_id: worktreeId ?? null,
      machine_ref: machineRef ?? null,
      worktree_placement: worktreePlacement ?? null,
    },
  }
}

export function moveAgentToRemoteRequest(sessionId: string, agentRef: string, machineRef: string) {
  return {
    MoveAgentToRemote: {
      session_id: sessionId,
      agent_ref: agentRef,
      machine_ref: machineRef,
    },
  }
}

export function destroyAgentRequest(sessionId: string, agentId: string) {
  return {
    DestroyAgent: {
      session_id: sessionId,
      agent_id: agentId,
    },
  }
}

export function focusAgentRequest(sessionId: string, agentId: string) {
  return {
    FocusAgent: {
      session_id: sessionId,
      agent_id: agentId,
    },
  }
}

export function cycleAgentFocusRequest(sessionId: string) {
  return {
    CycleAgentFocus: {
      session_id: sessionId,
    },
  }
}

export function listAgentsRequest(sessionId: string) {
  return {
    ListAgents: {
      session_id: sessionId,
    },
  }
}
