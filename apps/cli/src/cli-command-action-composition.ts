import type { BootstrapState } from "./cli-types.js"
import { createCommandActionHandlers } from "./command-actions.js"
import { resolveConfiguredCloudRelayApiUrl } from "./cli-options.js"
import { bootstrapCloudRelayProfile } from "./cloud-relay.js"
import { importExternalProviderAgent } from "./external-provider-session-api.js"
import { openExternalUrl } from "./external-url.js"
import { formatAgentLabel } from "./agent-label.js"
import {
  aliasAgent,
  cycleAgentFocus as cycleAgentFocusApi,
  destroyAgent as destroyAgentApi,
  focusAgent as focusAgentApi,
  spawnAgent as spawnAgentApi,
  updateAgentConfig,
  updateAgentProfile,
  updateAgentSubstitutes,
} from "./agent-api.js"
import {
  acceptCloudSessionInvite,
  createCloudSessionInvite,
  createSessionInvite,
  joinSessionInvite,
  listCloudCollaborators,
  listCloudSessionMembers,
} from "./cloud-session-api.js"
import {
  getUserConfig,
  getUserConfigSchema,
  setCredentialSecret,
  setWorkspaceLiveSyncMode,
  setUserConfigValue,
  unsetUserConfigValue,
} from "./config-api.js"
import {
  getMcpServer,
  getConnector,
  getConnectorAdapter,
  getCredential,
  getEnvironment,
  getScript,
  getSkill,
  grantAgentMcp,
  grantAgentConnector,
  grantAgentScript,
  grantAgentSkill,
  importMcpServers,
  importSkills,
  installMcpServer,
  installSkill,
  listEnvironments,
  listConnectors,
  listConnectorAdapters,
  listCredentials,
  listMcpServers,
  listHomeExtensionAudit,
  listScripts,
  listSkills,
  registerEnvironment,
  registerConnector,
  registerConnectorAdapter,
  registerCredential,
  registerScript,
  removeEnvironment,
  removeConnector,
  removeConnectorAdapter,
  removeCredential,
  removeScript,
  revokeAgentMcp,
  revokeAgentConnector,
  revokeAgentScript,
  revokeAgentSkill,
  syncRemoteExtensionManifest,
  uninstallMcpServer,
  uninstallSkill,
  updateMcpServer,
  updateSkill,
  validateScript,
  testConnector,
} from "./extension-api.js"
import { deleteKernel, exportDebugBundle, getDaemonHealth } from "./kernel-api.js"
import {
  mergeRelayCloudProfile,
  mergeUiPreferences,
  relayCloudProfile,
  saveRelayCloudProfile,
  saveUiPreferences,
} from "./preferences.js"
import {
  getProviderAuthStatus,
  getProviderRun,
  launchProviderRun,
  listProviderProcesses,
  logoutProvider,
  startProviderLogin,
  teardownProviderProcesses,
  updateSessionConfig,
} from "./provider-api.js"
import {
  approveRemoteMachine,
  forgetRemoteMachine,
  listRemoteMachineKernels,
  listRemoteMachines,
  renameRemoteMachine,
} from "./remote-machine-api.js"
import {
  configureRelay,
  connectKernelCloudRelay,
  getRelayStatus,
  issueKernelCloudRelayClientToken,
  logoutCloudRelay,
  pairKernelCloudRelayClient,
  pairKernelCloudRelayMachine,
  pollCloudRelayLogin,
  startCloudRelayLogin,
} from "./relay-api.js"
import {
  aliasSession,
  createSession,
  deleteSessionByRef,
  getSessionState,
  listSessions,
  resolveSession,
} from "./session-api.js"
import { SESSION_CONFIG_RESPONSE_LAYOUT_KEY } from "./session-state.js"
import { formatSessionList } from "./sessions.js"
import {
  createSlice,
  createSliceBackup,
  deleteSlice,
  getSlice,
  getSliceDisplayEndpoint,
  getSliceLogs,
  getSliceStateStatus,
  importSliceProviderAuth,
  listSliceAudit,
  listSlices,
  removeSliceProviderAuth,
  resetSliceState,
  saveSliceState,
  setSliceProviderAuthAlias,
  startSliceProviderLogin,
  startSlice,
  stopSlice,
} from "./slice-api.js"
import {
  attachWorkspaceLink,
  createWorkspaceLink,
  detachWorkspaceLink,
  getWorkspaceLiveSyncStatus,
  listWorkspaceLiveSyncAudit,
  listWorkspaceLinks,
  showWorkspaceLink,
} from "./workspace-link-api.js"

type AnyFn = (...args: any[]) => any

export type CliCommandActionCompositionDeps = {
  client: BootstrapState["client"]
  options: BootstrapState["options"]
  preferencesState: AnyFn
  setPreferencesState: AnyFn
  initialWorkspaceTarget: string
  initialWorktreeTarget: string
  pendingWorkspaceTarget: AnyFn
  pendingWorktreeTarget: AnyFn
  setPendingWorkspaceTarget: AnyFn
  setPendingWorktreeTarget: AnyFn
  isAttached: AnyFn
  sessionState: AnyFn
  attachmentState: AnyFn
  providerRunState: AnyFn
  currentModelId: AnyFn
  currentVariantId: AnyFn
  focusedAgentId: AnyFn
  multiAgentResponseLayout: AnyFn
  maxAgentsPerScreen: AnyFn
  flashFooter: AnyFn
  appendNotice: AnyFn
  readSecret?: AnyFn
  appendCloudNotice: AnyFn
  formatError: AnyFn
  attachBinding: AnyFn
  transitionToNoSession: AnyFn
  applyProviderSelection: AnyFn
  applyModelSelection: AnyFn
  applyVariantSelection: AnyFn
  refreshWaitingRoomData: AnyFn
  setSlicesState: AnyFn
  appLogger: { info: AnyFn } | null | undefined
  setMultiAgentResponseLayout: AnyFn
  applyResponseLayout: AnyFn
  applySessionState: AnyFn
  refreshAgentPanes: AnyFn
  setWorkspaceLiveSyncStatus?: AnyFn
  openWorkflowNodeInstructionsEditor: AnyFn
  closeWorkflowNodeInstructionsEditor: AnyFn
  getWorkflowNodeInstructionsDraft: AnyFn
  getWorkflowNodeInstructionsContext: AnyFn
  openWorkflowTerminalPanel: AnyFn
  rebuildTranscript: AnyFn
  requestRootRender: AnyFn
  scheduleTimer: (callback: () => void, delayMs: number) => unknown
  logViewDebug: AnyFn
  describeRenderableDebug: AnyFn
  currentFocusedRenderable: AnyFn
  trackAgentFocusTransition: AnyFn
  setProviderRunState: AnyFn
  resolveSessionAgent: AnyFn
  workflowScreenActive: AnyFn
  showWorkflowScreen: AnyFn
  selectedWorkflowId: AnyFn
  selectWorkflowCanvas: AnyFn
  replaceWorkflowDefinitions: AnyFn
  upsertWorkflowDefinition: AnyFn
  createWorkflow: AnyFn
  listWorkflows: AnyFn
  resolveWorkflow: AnyFn
  assignWorkflowAlias: AnyFn
  createWorkflowEndpoint: AnyFn
  assignWorkflowEndpointAlias: AnyFn
  bindWorkflowEndpoint: AnyFn
  addWorkflowNode: AnyFn
  removeWorkflowNode: AnyFn
  addWorkflowEdge: AnyFn
  removeWorkflowEdge: AnyFn
  updateWorkflowNodeInstructions: AnyFn
  setWorkflowNodeCanCompleteRun: AnyFn
  setWorkflowNodeCanEmitIntermediateOutput: AnyFn
  setWorkflowNodeIntermediateOutputSchema: AnyFn
  setWorkflowNodeMaxTurns: AnyFn
  invokeWorkflowEndpoint: AnyFn
  createWorkflowWatchdog: AnyFn
  listWorkflowWatchdogs: AnyFn
  setWorkflowWatchdogEnabled: AnyFn
  removeWorkflowWatchdog: AnyFn
  setWorkflowFlushContext: AnyFn
  setWorkflowRunOutputSchema: AnyFn
  setWorkflowIntermediateOutputSchema: AnyFn
  listWorkflowRuns: AnyFn
  cancelWorkflowRun: AnyFn
  resumeWorkflowRun: AnyFn
  refreshSplitPaneFocusRepaint: AnyFn
}

export function createCliCommandActionComposition(deps: CliCommandActionCompositionDeps) {
  const {
    client,
    options,
    preferencesState,
    setPreferencesState,
    initialWorkspaceTarget,
    initialWorktreeTarget,
    pendingWorkspaceTarget,
    pendingWorktreeTarget,
    setPendingWorkspaceTarget,
    setPendingWorktreeTarget,
    isAttached,
    sessionState,
    attachmentState,
    providerRunState,
    currentModelId,
    currentVariantId,
    focusedAgentId,
    multiAgentResponseLayout,
    maxAgentsPerScreen,
    flashFooter,
    appendNotice,
    appendCloudNotice,
    formatError,
    attachBinding,
    transitionToNoSession,
    applyProviderSelection,
    applyModelSelection,
    applyVariantSelection,
    refreshWaitingRoomData,
    setSlicesState,
    appLogger,
    setMultiAgentResponseLayout,
    applyResponseLayout,
    applySessionState,
    refreshAgentPanes,
    setWorkspaceLiveSyncStatus,
    openWorkflowNodeInstructionsEditor,
    closeWorkflowNodeInstructionsEditor,
    getWorkflowNodeInstructionsDraft,
    getWorkflowNodeInstructionsContext,
    openWorkflowTerminalPanel,
    rebuildTranscript,
    requestRootRender,
    scheduleTimer,
    logViewDebug,
    describeRenderableDebug,
    currentFocusedRenderable,
    trackAgentFocusTransition,
    setProviderRunState,
    resolveSessionAgent,
    workflowScreenActive,
    showWorkflowScreen,
    selectedWorkflowId,
    selectWorkflowCanvas,
    replaceWorkflowDefinitions,
    upsertWorkflowDefinition,
    createWorkflow,
    listWorkflows,
    resolveWorkflow,
    assignWorkflowAlias,
    createWorkflowEndpoint,
    assignWorkflowEndpointAlias,
    bindWorkflowEndpoint,
    addWorkflowNode,
    removeWorkflowNode,
    addWorkflowEdge,
    removeWorkflowEdge,
    updateWorkflowNodeInstructions,
    setWorkflowNodeCanCompleteRun,
    setWorkflowNodeCanEmitIntermediateOutput,
    setWorkflowNodeIntermediateOutputSchema,
    setWorkflowNodeMaxTurns,
    invokeWorkflowEndpoint,
    createWorkflowWatchdog,
    listWorkflowWatchdogs,
    setWorkflowWatchdogEnabled,
    removeWorkflowWatchdog,
    setWorkflowFlushContext,
    setWorkflowRunOutputSchema,
    setWorkflowIntermediateOutputSchema,
    listWorkflowRuns,
    cancelWorkflowRun,
    resumeWorkflowRun,
    refreshSplitPaneFocusRepaint,
  } = deps

  return createCommandActionHandlers({
    ...(resolveConfiguredCloudRelayApiUrl(preferencesState())
      ? { cloudRelayApiUrl: resolveConfiguredCloudRelayApiUrl(preferencesState()) }
      : {}),
    workspace: initialWorkspaceTarget,
    worktree: initialWorktreeTarget,
    getWorkspaceTarget: pendingWorkspaceTarget,
    getWorktreeTarget: pendingWorktreeTarget,
    setWorkspaceTarget: setPendingWorkspaceTarget,
    setWorktreeTarget: setPendingWorktreeTarget,
    accountProfile: options.accountProfile,
    clientId: options.clientId,
    isAttached,
    sessionState,
    attachmentState,
    providerRunState,
    currentModelId,
    currentVariantId,
    currentProviderId: () => options.provider ?? "opencode",
    focusedAgentId,
    multiAgentResponseLayout,
    maxAgentsPerScreen,
    isRelayConnection: () => Boolean(options.relayUrl),
    flashFooter,
    appendNotice,
    appendCloudNotice,
    formatError,
    createSession: (workspace, worktree, alias, agentDefaults) => createSession(client, workspace, worktree, alias, agentDefaults),
    createSessionInvite: (sessionId, expiresInMs, maxUses, collaborationLevel) =>
      createSessionInvite(client, sessionId, expiresInMs, maxUses, collaborationLevel),
    joinSessionInvite: (inviteToken, userId) => joinSessionInvite(client, inviteToken, userId),
    attachBinding: (session, createdSession) => attachBinding(session, createdSession),
    resolveSession: (reference, workspace) => resolveSession(client, reference, workspace),
    listSessions: () => listSessions(client),
    deleteSessionByRef: (reference, workspace) => deleteSessionByRef(client, reference, workspace),
    deleteKernel: () => deleteKernel(client),
    getDaemonHealth: () => getDaemonHealth(client),
    exportDebugBundle: (sessionId, label) => exportDebugBundle(client, sessionId, label),
    assignSessionAlias: (sessionId, alias) => aliasSession(client, sessionId, alias),
    aliasAgent: (sessionId, agentId, alias) => aliasAgent(client, sessionId, agentId, alias),
    updateAgentProfile: (sessionId, agentId, options) =>
      updateAgentProfile(client, sessionId, agentId, options),
    transitionToNoSession,
    applyProviderSelection,
    applyModelSelection,
    applyVariantSelection,
    getProviderAuthStatus: (provider) => getProviderAuthStatus(client, provider),
    startProviderLogin: (provider) => startProviderLogin(client, provider),
    logoutProvider: (provider) => logoutProvider(client, provider),
    getRelayStatus: () => getRelayStatus(client),
    configureRelay: (relayUrl, relayToken) => configureRelay(client, relayUrl, relayToken),
    getCloudRelayProfile: () => relayCloudProfile(preferencesState()),
    saveCloudRelayProfile: async (profile) => {
      await saveRelayCloudProfile(profile)
      setPreferencesState((current: any) => mergeRelayCloudProfile(current, profile))
    },
    bootstrapCloudRelay: (apiUrl, email, accountSlug) =>
      bootstrapCloudRelayProfile({
        apiUrl,
        email,
        ...(accountSlug ? { accountSlug } : {}),
      }),
    startCloudDeviceLogin: (apiUrl, input) => startCloudRelayLogin(client, apiUrl, input),
    pollCloudDeviceLogin: (apiUrl, deviceCode) => pollCloudRelayLogin(client, apiUrl, deviceCode),
    openExternalUrl,
    logoutCloudRelay: (_profile, options) => logoutCloudRelay(client, options),
    pairCloudRelayClient: (_profile, clientId, alias) =>
      pairKernelCloudRelayClient(client, clientId, alias),
    pairCloudRelayMachine: (_profile, machineId, alias) =>
      pairKernelCloudRelayMachine(client, machineId, alias),
    issueCloudKernelRelayToken: async () => connectKernelCloudRelay(client),
    issueCloudMachineRelayToken: async () => connectKernelCloudRelay(client),
    issueCloudClientRelayToken: async (_profile, targetDaemonAlias, tokenOptions) =>
      issueKernelCloudRelayClientToken(
        client,
        targetDaemonAlias,
        options.clientId ?? "arroba-cli",
        tokenOptions?.sessionId ?? null,
      ),
    createCloudSessionInvite: (sessionId, inviteOptions) =>
      createCloudSessionInvite(client, sessionId, inviteOptions),
    acceptCloudSessionInvite: (inviteToken) => acceptCloudSessionInvite(client, inviteToken),
    listCloudSessionMembers: (sessionId) => listCloudSessionMembers(client, sessionId),
    listCloudCollaborators: () => listCloudCollaborators(client),
    getUserConfig: () => getUserConfig(client),
    getUserConfigSchema: () => getUserConfigSchema(client),
    setUserConfigValue: (path, value) => setUserConfigValue(client, path, value),
    setWorkspaceLiveSyncMode: (sessionId, mode) => setWorkspaceLiveSyncMode(client, sessionId, mode),
    unsetUserConfigValue: (path) => unsetUserConfigValue(client, path),
    refreshWaitingRoomData,
    listRemoteMachines: () => listRemoteMachines(client),
    listRemoteMachineKernels: (machineRef) => listRemoteMachineKernels(client, machineRef),
    approveRemoteMachine: (machineRef) => approveRemoteMachine(client, machineRef),
    forgetRemoteMachine: (machineRef) => forgetRemoteMachine(client, machineRef),
    renameRemoteMachine: (machineRef, alias) => renameRemoteMachine(client, machineRef, alias),
    listSlices: async () => {
      const slices = await listSlices(client)
      setSlicesState(slices)
      return slices
    },
    createSlice: async (sliceOptions) => {
      const slice = await createSlice(client, sliceOptions)
      setSlicesState(await listSlices(client))
      return slice
    },
    getSlice: async (sliceRef) => getSlice(client, sliceRef),
    startSlice: async (sliceRef) => {
      const slice = await startSlice(client, sliceRef)
      setSlicesState(await listSlices(client))
      return slice
    },
    stopSlice: async (sliceRef) => {
      const slice = await stopSlice(client, sliceRef)
      setSlicesState(await listSlices(client))
      return slice
    },
    deleteSlice: async (sliceRef) => {
      const slice = await deleteSlice(client, sliceRef)
      setSlicesState(await listSlices(client))
      return slice
    },
    importSliceProviderAuth: async (sliceRef, provider) => {
      const result = await importSliceProviderAuth(client, sliceRef, provider)
      setSlicesState(await listSlices(client))
      return result
    },
    removeSliceProviderAuth: async (sliceRef, provider) => {
      const result = await removeSliceProviderAuth(client, sliceRef, provider)
      setSlicesState(await listSlices(client))
      return result
    },
    startSliceProviderLogin: async (sliceRef, provider) => {
      const result = await startSliceProviderLogin(client, sliceRef, provider)
      setSlicesState(await listSlices(client))
      return result
    },
    setSliceProviderAuthAlias: async (sliceRef, provider, alias) => {
      const result = await setSliceProviderAuthAlias(client, sliceRef, provider, alias)
      setSlicesState(await listSlices(client))
      return result
    },
    getSliceDisplayEndpoint: async (sliceRef) => getSliceDisplayEndpoint(client, sliceRef),
    getSliceLogs: async (sliceRef, tailLines) => getSliceLogs(client, sliceRef, tailLines),
    listSliceAudit: async (sliceRef, limit) => listSliceAudit(client, sliceRef, limit),
    saveSliceState: async (sliceRef, mode) => {
      const result = await saveSliceState(client, sliceRef, mode)
      setSlicesState(await listSlices(client))
      return result
    },
    getSliceStateStatus: async (sliceRef) => getSliceStateStatus(client, sliceRef),
    resetSliceState: async (sliceRef) => {
      const result = await resetSliceState(client, sliceRef)
      setSlicesState(await listSlices(client))
      return result
    },
    createSliceBackup: async (sliceRef, name) => {
      const result = await createSliceBackup(client, sliceRef, name)
      setSlicesState(await listSlices(client))
      return result
    },
    listProviderProcesses: (provider) => listProviderProcesses(client, provider),
    teardownProviderProcesses: (provider) => teardownProviderProcesses(client, provider),
    listMcpServers: () => listMcpServers(client, pendingWorkspaceTarget()),
    installMcpServer: (config) => installMcpServer(client, pendingWorkspaceTarget(), config),
    updateMcpServer: (config) => updateMcpServer(client, pendingWorkspaceTarget(), config),
    uninstallMcpServer: (name) => uninstallMcpServer(client, pendingWorkspaceTarget(), name),
    importMcpServers: (provider, name) => importMcpServers(client, pendingWorkspaceTarget(), provider, name),
    getMcpServer: (name) => getMcpServer(client, pendingWorkspaceTarget(), name),
    grantAgentMcp: (agentRef, name) => grantAgentMcp(client, pendingWorkspaceTarget(), agentRef, name),
    revokeAgentMcp: (agentRef, name) => revokeAgentMcp(client, agentRef, name),
    listSkills: () => listSkills(client, pendingWorkspaceTarget()),
    installSkill: (sourcePath) => installSkill(client, pendingWorkspaceTarget(), sourcePath),
    updateSkill: (sourcePath) => updateSkill(client, pendingWorkspaceTarget(), sourcePath),
    uninstallSkill: (name) => uninstallSkill(client, pendingWorkspaceTarget(), name),
    importSkills: (provider, name) => importSkills(client, pendingWorkspaceTarget(), provider, name),
    getSkill: (name) => getSkill(client, pendingWorkspaceTarget(), name),
    grantAgentSkill: (agentRef, name) => grantAgentSkill(client, pendingWorkspaceTarget(), agentRef, name),
    revokeAgentSkill: (agentRef, name) => revokeAgentSkill(client, agentRef, name),
    listEnvironments: () => listEnvironments(client, pendingWorkspaceTarget()),
    getEnvironment: (name) => getEnvironment(client, pendingWorkspaceTarget(), name),
    registerEnvironment: (config) => registerEnvironment(client, pendingWorkspaceTarget(), config),
    removeEnvironment: (name) => removeEnvironment(client, pendingWorkspaceTarget(), name),
    listScripts: () => listScripts(client, pendingWorkspaceTarget()),
    getScript: (name) => getScript(client, pendingWorkspaceTarget(), name),
    validateScript: (sourcePath, environment, name) => validateScript(client, pendingWorkspaceTarget(), sourcePath, environment, name),
    registerScript: (sourcePath, environment, name) => registerScript(client, pendingWorkspaceTarget(), sourcePath, environment, name),
    removeScript: (name) => removeScript(client, pendingWorkspaceTarget(), name),
    grantAgentScript: (agentRef, name, environment) => grantAgentScript(client, pendingWorkspaceTarget(), agentRef, name, environment),
    revokeAgentScript: (agentRef, name) => revokeAgentScript(client, agentRef, name),
    listCredentials: () => listCredentials(client),
    getCredential: (id) => getCredential(client, id),
    setCredentialSecret: (key, value) => setCredentialSecret(client, key, value),
    registerCredential: (sourcePath) => registerCredential(client, sourcePath),
    removeCredential: (id) => removeCredential(client, id),
    listConnectors: () => listConnectors(client),
    getConnector: (name) => getConnector(client, name),
    registerConnector: (sourcePath) => registerConnector(client, sourcePath),
    removeConnector: (name) => removeConnector(client, name),
    listConnectorAdapters: () => listConnectorAdapters(client),
    getConnectorAdapter: (name) => getConnectorAdapter(client, name),
    registerConnectorAdapter: (sourcePath) => registerConnectorAdapter(client, sourcePath),
    removeConnectorAdapter: (name) => removeConnectorAdapter(client, name),
    testConnector: (name, operation, input, credential, allow) => testConnector(client, name, operation, input, credential, allow),
    grantAgentConnector: (agentRef, name, credential, maxSafety) => grantAgentConnector(client, pendingWorkspaceTarget(), agentRef, name, credential, maxSafety),
    revokeAgentConnector: (agentRef, name) => revokeAgentConnector(client, agentRef, name),
    syncRemoteExtensionManifest: (agentRef) => syncRemoteExtensionManifest(client, agentRef),
    listHomeExtensionAudit: (agentRef, limit) => listHomeExtensionAudit(client, agentRef, limit),
    logViewCommand: (fields) => {
      appLogger?.info("handling view command", fields)
      logViewDebug("view command:after set layout", fields)
    },
    setMultiAgentResponseLayout,
    applyResponseLayout,
    updateSessionResponseLayout: (sessionId, attachmentId, layout) =>
      updateSessionConfig(
        client,
        sessionId,
        attachmentId,
        { [SESSION_CONFIG_RESPONSE_LAYOUT_KEY]: layout },
        false,
      ),
    updateSessionConfig: (sessionId, attachmentId, values, requiresIdle) =>
      updateSessionConfig(client, sessionId, attachmentId, values, requiresIdle),
    updateAgentConfig: (sessionId, agentId, options) =>
      updateAgentConfig(client, sessionId, agentId, options),
    updateAgentSubstitutes: (sessionId, agentId, action) =>
      updateAgentSubstitutes(client, sessionId, agentId, action),
    applySessionState,
    refreshAgentPanes,
    createWorkspaceLink: (name) => createWorkspaceLink(client, sessionState().id, name),
    listWorkspaceLinks: () => listWorkspaceLinks(client, sessionState().id),
    showWorkspaceLink: (linkRef) => showWorkspaceLink(client, sessionState().id, linkRef),
    attachWorkspaceLink: (linkRef, repoRoot) => attachWorkspaceLink(client, sessionState().id, linkRef, repoRoot),
    detachWorkspaceLink: (linkRef, repoRoot) => detachWorkspaceLink(client, sessionState().id, linkRef, repoRoot),
    getWorkspaceLiveSyncStatus: () => getWorkspaceLiveSyncStatus(client, sessionState().id),
    listWorkspaceLiveSyncAudit: (sessionId, limit) => listWorkspaceLiveSyncAudit(client, sessionId, limit),
    ...(setWorkspaceLiveSyncStatus ? { setWorkspaceLiveSyncStatus } : {}),
    openWorkflowNodeInstructionsEditor,
    closeWorkflowNodeInstructionsEditor,
    getWorkflowNodeInstructionsDraft,
    getWorkflowNodeInstructionsContext,
    openWorkflowTerminalPanel,
    saveUiPreferences: async (prefs) => {
      await saveUiPreferences(prefs)
      setPreferencesState((current: any) => mergeUiPreferences(current, prefs))
    },
    rebuildTranscript,
    requestRender: requestRootRender,
    afterViewRender: (layout) => {
      scheduleTimer(() => {
        logViewDebug("view command:post render tick", {
          requested_layout: layout,
          current_focus: describeRenderableDebug(currentFocusedRenderable()),
        })
      }, 0)
    },
    cycleAgentFocus: async () => {
      return trackAgentFocusTransition(async () => {
        const agent = await cycleAgentFocusApi(client, sessionState().id)
        const session = await getSessionState(client, sessionState().id)
        if (session.active_provider_run_id) {
          setProviderRunState(await getProviderRun(client, session.active_provider_run_id))
        } else {
          setProviderRunState(null)
        }
        return {
          agent,
          session,
        }
      })
    },
    launchAgentProviderRun: (provider, model, variant, agentId) =>
      launchProviderRun(
        client,
        sessionState().id,
        provider,
        options.accountProfile,
        model,
        variant,
        agentId,
      ),
    setProviderRunState,
    refreshSessionState: (sessionId) => getSessionState(client, sessionId),
    spawnAgent: async (provider, alias, model, effort, worktreeId, machineRef, worktreePlacement, sliceRef) => {
      const agent = await spawnAgentApi(
        client,
        sessionState().id,
        {
          provider,
          alias,
          model,
          effort,
          worktreeId,
          kernelRef: machineRef,
          worktreePlacement,
          sliceRef,
        },
      )
      return {
        agent,
        session: await getSessionState(client, sessionState().id),
      }
    },
    importExternalProviderAgent: async (externalSessionId) => {
      const payload = await importExternalProviderAgent(client, sessionState().id, externalSessionId)
      if (payload.providerRun) {
        setProviderRunState(payload.providerRun)
      }
      return {
        agent: payload.agent,
        session: payload.session,
        providerRun: payload.providerRun,
      }
    },
    destroyAgent: async (agentId) => {
      await destroyAgentApi(client, sessionState().id, agentId)
      return getSessionState(client, sessionState().id)
    },
    focusAgent: async (agentId) => {
      return trackAgentFocusTransition(async () => {
        const agent = await focusAgentApi(client, sessionState().id, agentId)
        const session = await getSessionState(client, sessionState().id)
        if (session.active_provider_run_id) {
          setProviderRunState(await getProviderRun(client, session.active_provider_run_id))
        } else {
          setProviderRunState(null)
        }
        return {
          agent,
          session,
        }
      })
    },
    resolveSessionAgent: (reference) => {
      const resolved = resolveSessionAgent(reference)
      return resolved.error
        ? { agent: resolved.agent ?? null, error: resolved.error }
        : { agent: resolved.agent ?? null }
    },
    workflowScreenActive,
    showWorkflowScreen,
    selectedWorkflowId,
    selectWorkflowCanvas,
    replaceWorkflowDefinitions,
    upsertWorkflowDefinition,
    createWorkflow,
    listWorkflows,
    resolveWorkflow,
    assignWorkflowAlias,
    createWorkflowEndpoint,
    assignWorkflowEndpointAlias,
    bindWorkflowEndpoint,
    addWorkflowNode,
    removeWorkflowNode,
    addWorkflowEdge,
    removeWorkflowEdge,
    updateWorkflowNodeInstructions,
    setWorkflowNodeCanCompleteRun,
    setWorkflowNodeCanEmitIntermediateOutput,
    setWorkflowNodeIntermediateOutputSchema,
    setWorkflowNodeMaxTurns,
    invokeWorkflowEndpoint,
    createWorkflowWatchdog,
    listWorkflowWatchdogs,
    setWorkflowWatchdogEnabled,
    removeWorkflowWatchdog,
    setWorkflowFlushContext,
    setWorkflowRunOutputSchema,
    setWorkflowIntermediateOutputSchema,
    listWorkflowRuns,
    cancelWorkflowRun,
    resumeWorkflowRun,
    formatAgentLabel,
    refreshSplitPaneFocusRepaint,
    formatSessionList: (sessions, currentSessionId) => formatSessionList(sessions, currentSessionId ?? undefined),
  })
}
