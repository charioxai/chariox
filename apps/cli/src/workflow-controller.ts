import type { RuntimeSession } from "./cli-types.js"
import type { WorkspaceScreenMode } from "./workspace-screen.js"
import type { WorkflowInspectorMode } from "./workflow-inspector-projection.js"
import type { WorkflowComponentSelection } from "./workflow-component-selection.js"
import { createWorkflowDefinitionController } from "./workflow-definition-controller.js"
import { createWorkflowRuntimeController } from "./workflow-runtime-controller.js"
import { createWorkflowScreenController } from "./workflow-screen-controller.js"
import { createWorkflowSessionStateController } from "./workflow-session-state.js"
import { createWorkflowSettingsController } from "./workflow-settings-controller.js"
import { createWorkflowTopologyController } from "./workflow-topology-controller.js"
import { createWorkflowWatchdogController } from "./workflow-watchdog-controller.js"

export {
  createWorkflowSelectionSyncController,
  deriveWorkflowSelectionState,
} from "./workflow-selection-sync.js"

type WorkflowControllerDeps = {
  sendRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  applySessionState: (session: RuntimeSession) => void
  selectedWorkflowId: () => string | null
  setSelectedWorkflowId: (value: string | null) => void
  selectedWorkflowNodeId: () => string | null
  setSelectedWorkflowNodeId: (value: string | null) => void
  setSelectedWorkflowComponent?: (value: WorkflowComponentSelection | null) => void
  setWorkflowInspectorMode?: (value: WorkflowInspectorMode) => void
  workspaceScreenMode: () => WorkspaceScreenMode
  setWorkspaceScreenMode: (value: WorkspaceScreenMode) => void
  rebuildTranscript: () => void
  applyResponseLayout: () => void
}

export function createWorkflowController(deps: WorkflowControllerDeps) {
  const workflowScreen = createWorkflowScreenController({
    isAttached: deps.isAttached,
    workflows: () => deps.sessionState().workflows ?? [],
    selectedWorkflowId: deps.selectedWorkflowId,
    setSelectedWorkflowId: deps.setSelectedWorkflowId,
    selectedWorkflowNodeId: deps.selectedWorkflowNodeId,
    setSelectedWorkflowNodeId: deps.setSelectedWorkflowNodeId,
    ...(deps.setSelectedWorkflowComponent ? { setSelectedWorkflowComponent: deps.setSelectedWorkflowComponent } : {}),
    workspaceScreenMode: deps.workspaceScreenMode,
    setWorkspaceScreenMode: deps.setWorkspaceScreenMode,
    rebuildTranscript: deps.rebuildTranscript,
    applyResponseLayout: deps.applyResponseLayout,
    ...(deps.setWorkflowInspectorMode ? { setWorkflowInspectorMode: deps.setWorkflowInspectorMode } : {}),
  })
  const workflowSessionState = createWorkflowSessionStateController({
    sessionState: deps.sessionState,
    applySessionState: deps.applySessionState,
    rebuildTranscript: deps.rebuildTranscript,
    applyResponseLayout: deps.applyResponseLayout,
  })
  const workflowTopology = createWorkflowTopologyController({
    sendRequest: deps.sendRequest,
    sessionId: () => deps.sessionState().id,
  })
  const workflowRuntime = createWorkflowRuntimeController({
    sendRequest: deps.sendRequest,
    sessionId: () => deps.sessionState().id,
    applyWorkflowSessionRefresh: workflowSessionState.applyWorkflowSessionRefresh,
  })
  const workflowWatchdogs = createWorkflowWatchdogController({
    sendRequest: deps.sendRequest,
    sessionId: () => deps.sessionState().id,
    applyWorkflowSessionRefresh: workflowSessionState.applyWorkflowSessionRefresh,
  })
  const workflowSettings = createWorkflowSettingsController({
    sendRequest: deps.sendRequest,
    sessionId: () => deps.sessionState().id,
    applyWorkflowSessionRefresh: workflowSessionState.applyWorkflowSessionRefresh,
  })
  const workflowDefinitions = createWorkflowDefinitionController({
    sendRequest: deps.sendRequest,
    sessionId: () => deps.sessionState().id,
    applySessionState: deps.applySessionState,
    setSelectedWorkflowId: deps.setSelectedWorkflowId,
    setSelectedWorkflowNodeId: deps.setSelectedWorkflowNodeId,
    rebuildTranscript: deps.rebuildTranscript,
    applyResponseLayout: deps.applyResponseLayout,
  })

  return {
    ...workflowScreen,
    ...workflowRuntime,
    ...workflowTopology,
    ...workflowWatchdogs,
    ...workflowSettings,
    ...workflowDefinitions,
    replaceWorkflowDefinitions: workflowSessionState.replaceWorkflowDefinitions,
    upsertWorkflowDefinition: workflowSessionState.upsertWorkflowDefinition,
  }
}
