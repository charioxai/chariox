import {
  startCliAutomationServer,
  stopCliAutomationServer,
} from "./cli-automation.js"
import { createCliAutomationActionHandler } from "./cli-automation-handler.js"
import {
  createCliAutomationServerController,
  type CliAutomationServerLogger,
} from "./cli-automation-server-controller.js"
import { createCliAutomationSnapshotController } from "./cli-automation-snapshot-controller.js"
import { createCliProcessLifecycleController } from "./cli-process-lifecycle-controller.js"
import type { CliOptions } from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import type { CharioxLogger } from "./logging.js"

type AnyFn = (...args: any[]) => any

export type CliAutomationProcessCompositionDeps = {
  client: LocalIpcClient
  options: CliOptions
  appLogger: (CharioxLogger & CliAutomationServerLogger) | null
  formatError: AnyFn
  flashFooter: AnyFn
  handleSigint: AnyFn
  handleStdinData: AnyFn
  onSigint: AnyFn
  offSigint: AnyFn
  onStdinData: AnyFn
  offStdinData: AnyFn
  clearTerminalOutputRecordTimer: AnyFn
  workspaceScreenMode: AnyFn
  workflowScreenActive: AnyFn
  daemonDisconnected: AnyFn
  statusLine: AnyFn
  sessionState: AnyFn
  focusedAgentId: AnyFn
  agentActivityLabels: AnyFn
  streamingAgentId: AnyFn
  agentBusyLatch: AnyFn
  isAttached: AnyFn
  waitingRoomState: AnyFn
  setWaitingRoomState: AnyFn
  availableSessions: AnyFn
  waitingRoomProjects: AnyFn
  applyWaitingRoomSessionLifecycleAction: AnyFn
  restoreWaitingRoomProject: AnyFn
  renameWaitingRoomProject: AnyFn
  providerCatalogState: AnyFn
  waitingRoomCloudNotice: AnyFn
  waitingRoomInventoryStatus: AnyFn
  relayStatusState: AnyFn
  remoteMachinesState: AnyFn
  remoteKernelsState: AnyFn
  terminalsState: AnyFn
  externalProviderSessionsState: AnyFn
  externalProviderSessionsPageState: AnyFn
  slicesState: AnyFn
  providerAccountsState: AnyFn
  waitingRoomTargets: AnyFn
  themeRegistryState: AnyFn
  selectedWorkflowId: AnyFn
  selectedWorkflowNodeId: AnyFn
  workspaceShellContext: AnyFn
  workspaceShellEntries: AnyFn
  transcriptEntries: AnyFn
  visibleTranscriptAgentId: AnyFn
  agentPaneEntries: AnyFn
  queuedPromptStripItemsForAgent: AnyFn
  selectedQueuedPromptIndexForAgent: AnyFn
  onQueuedPromptAction: AnyFn
  footerFlash: AnyFn
  getInteractionChoiceSelection: AnyFn
  getInteractionCustomReply: AnyFn
  isInteractionCustomEditing: AnyFn
  setInteractionCustomReply: AnyFn
  setInteractionCustomEditing: AnyFn
  kernelConnected: AnyFn
  setWorkspaceScreenMode: AnyFn
  rebuildTranscript: AnyFn
  applyResponseLayout: AnyFn
  showWorkflowScreen: AnyFn
  submitWorkspaceShellCommand: AnyFn
  attachmentState: AnyFn
  setPromptText: AnyFn
  submitPrompt: AnyFn
  activateWaitingRoom: AnyFn
  requestWaitingRoom?: AnyFn
  connectDetachedKernelFromWaitingRoom: AnyFn
  refreshWaitingRoomData: AnyFn
  submitFocusedInteractionChoice: AnyFn
  cycleFocusedInteractionChoice: AnyFn
  toggleTurn: AnyFn
  toggleAgentPaneTurn?: AnyFn
  toggleBlob: AnyFn
  toggleAgentPaneBlob?: AnyFn
  restoreTerminalAndExit: AnyFn
  sleep: AnyFn
}

export function createCliAutomationProcessComposition(deps: CliAutomationProcessCompositionDeps) {
  const automationSnapshotController = createCliAutomationSnapshotController({
    attachmentId: () => deps.attachmentState()?.id ?? null,
    workspaceScreenMode: deps.workspaceScreenMode,
    workflowScreenActive: deps.workflowScreenActive,
    daemonDisconnected: deps.daemonDisconnected,
    statusLine: deps.statusLine,
    sessionState: deps.sessionState,
    focusedAgentId: deps.focusedAgentId,
    agentActivityLabels: deps.agentActivityLabels,
    streamingAgentId: deps.streamingAgentId,
    agentBusyLatch: deps.agentBusyLatch,
    isAttached: deps.isAttached,
    waitingRoomState: deps.waitingRoomState,
    availableSessions: deps.availableSessions,
    waitingRoomProjects: deps.waitingRoomProjects,
    providerCatalogState: deps.providerCatalogState,
    waitingRoomCloudNotice: deps.waitingRoomCloudNotice,
    waitingRoomInventoryStatus: deps.waitingRoomInventoryStatus,
    relayStatusState: deps.relayStatusState,
    remoteMachinesState: deps.remoteMachinesState,
    remoteKernelsState: deps.remoteKernelsState,
    terminalsState: deps.terminalsState,
    externalProviderSessionsState: deps.externalProviderSessionsState,
    externalProviderSessionsPageState: deps.externalProviderSessionsPageState,
    slicesState: deps.slicesState,
    providerAccountsState: deps.providerAccountsState,
    waitingRoomTargets: deps.waitingRoomTargets,
    themeRegistryState: deps.themeRegistryState,
    selectedWorkflowId: deps.selectedWorkflowId,
    selectedWorkflowNodeId: deps.selectedWorkflowNodeId,
    workspaceShellContext: deps.workspaceShellContext,
    workspaceShellEntries: deps.workspaceShellEntries,
    transcriptEntries: deps.transcriptEntries,
    visibleTranscriptAgentId: deps.visibleTranscriptAgentId,
    agentPaneEntries: deps.agentPaneEntries,
    queuedPromptStripItemsForAgent: deps.queuedPromptStripItemsForAgent,
    selectedQueuedPromptIndexForAgent: deps.selectedQueuedPromptIndexForAgent,
    footerFlash: deps.footerFlash,
    getInteractionChoiceSelection: deps.getInteractionChoiceSelection,
    getInteractionCustomReply: deps.getInteractionCustomReply,
    isInteractionCustomEditing: deps.isInteractionCustomEditing,
  })
  const automationSnapshot = automationSnapshotController.snapshot

  const handleAutomationRequest = createCliAutomationActionHandler({
    client: deps.client,
    options: deps.options,
    appLogger: deps.appLogger,
    snapshot: automationSnapshot,
    isAttached: deps.isAttached,
    kernelConnected: deps.kernelConnected,
    workflowScreenActive: deps.workflowScreenActive,
    setWorkspaceScreenMode: deps.setWorkspaceScreenMode,
    rebuildTranscript: deps.rebuildTranscript,
    applyResponseLayout: deps.applyResponseLayout,
    showWorkflowScreen: deps.showWorkflowScreen,
    submitWorkspaceShellCommand: deps.submitWorkspaceShellCommand,
    attachmentState: deps.attachmentState,
    sessionState: deps.sessionState,
    focusedAgentId: deps.focusedAgentId,
    setPromptText: deps.setPromptText,
    submitPrompt: deps.submitPrompt,
    activateWaitingRoom: deps.activateWaitingRoom,
    requestWaitingRoom: deps.requestWaitingRoom,
    waitingRoomState: deps.waitingRoomState,
    setWaitingRoomState: deps.setWaitingRoomState,
    externalProviderSessionsState: deps.externalProviderSessionsState,
    waitingRoomProjects: deps.waitingRoomProjects,
    applyWaitingRoomSessionLifecycleAction: deps.applyWaitingRoomSessionLifecycleAction,
    restoreWaitingRoomProject: deps.restoreWaitingRoomProject,
    renameWaitingRoomProject: deps.renameWaitingRoomProject,
    connectDetachedKernelFromWaitingRoom: deps.connectDetachedKernelFromWaitingRoom,
    refreshWaitingRoomData: deps.refreshWaitingRoomData,
    submitFocusedInteractionChoice: deps.submitFocusedInteractionChoice,
    cycleFocusedInteractionChoice: deps.cycleFocusedInteractionChoice,
    setInteractionCustomReply: deps.setInteractionCustomReply,
    setInteractionCustomEditing: deps.setInteractionCustomEditing,
    toggleTurn: deps.toggleTurn,
    toggleAgentPaneTurn: deps.toggleAgentPaneTurn,
    toggleBlob: deps.toggleBlob,
    toggleAgentPaneBlob: deps.toggleAgentPaneBlob,
    queuedPromptStripItemsForAgent: deps.queuedPromptStripItemsForAgent,
    selectedQueuedPromptIndexForAgent: deps.selectedQueuedPromptIndexForAgent,
    onQueuedPromptAction: deps.onQueuedPromptAction,
    restoreTerminalAndExit: deps.restoreTerminalAndExit,
    sleep: deps.sleep,
  })

  const automationServerController = createCliAutomationServerController({
    socketPath: deps.options.automationSocket ?? undefined,
    handleRequest: handleAutomationRequest,
    startServer: startCliAutomationServer,
    stopServer: stopCliAutomationServer,
    formatError: deps.formatError,
    logger: deps.appLogger,
    flashFooter: deps.flashFooter,
  })
  const processLifecycleController = createCliProcessLifecycleController({
    handleSigint: deps.handleSigint,
    handleStdinData: deps.handleStdinData,
    startAutomationServer: () => automationServerController.start(),
    stopAutomationServer: () => automationServerController.stop(),
    onSigint: deps.onSigint,
    offSigint: deps.offSigint,
    onStdinData: deps.onStdinData,
    offStdinData: deps.offStdinData,
    clearTerminalOutputRecordTimer: deps.clearTerminalOutputRecordTimer,
  })

  return {
    start: processLifecycleController.start,
    stop: processLifecycleController.stop,
  }
}
