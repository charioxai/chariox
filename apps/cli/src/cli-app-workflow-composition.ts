import { createEffect } from "solid-js"

import { createWorkflowController, createWorkflowSelectionSyncController } from "./workflow-controller.js"
import { createWorkflowInspectorController } from "./workflow-inspector-controller.js"
import { createWorkflowNodeInstructionsEditorController } from "./workflow-node-instructions-editor-controller.js"
import { createWorkflowTerminalPanelController } from "./workflow-terminal-panel-controller.js"

type AnyFn = (...args: any[]) => any

type WorkflowNodeInstructionsEditorBridge = {
  updateDraft: AnyFn
  setInputRef: AnyFn
}

export type CliAppWorkflowProjectionCompositionDeps = Record<string, any> & {
  sessionState: AnyFn
  selectedWorkflowId: AnyFn
  selectedWorkflowNodeId: AnyFn
  selectedWorkflowComponent: AnyFn
  workflowInspectorMode: AnyFn
  workflowNodeInstructionsEditor: AnyFn
  agentPaneEntries: AnyFn
  setSelectedWorkflowId: AnyFn
  setSelectedWorkflowNodeId: AnyFn
}

export function createCliAppWorkflowProjectionComposition(
  deps: CliAppWorkflowProjectionCompositionDeps,
) {
  const nodeInstructionsEditorBridge: WorkflowNodeInstructionsEditorBridge = {
    updateDraft: () => undefined,
    setInputRef: () => undefined,
  }
  const workflowSelectionSyncController = createWorkflowSelectionSyncController({
    workflows: () => deps.sessionState().workflows ?? [],
    selectedWorkflowId: deps.selectedWorkflowId,
    selectedWorkflowNodeId: deps.selectedWorkflowNodeId,
    setSelectedWorkflowId: deps.setSelectedWorkflowId,
    setSelectedWorkflowNodeId: deps.setSelectedWorkflowNodeId,
  })
  createEffect(() => {
    workflowSelectionSyncController.sync()
  })
  const workflowInspectorController = createWorkflowInspectorController({
    getSession: deps.sessionState,
    getSelectedWorkflowId: deps.selectedWorkflowId,
    getSelectedWorkflowNodeId: deps.selectedWorkflowNodeId,
    getSelectedWorkflowComponent: deps.selectedWorkflowComponent,
    getInspectorMode: deps.workflowInspectorMode,
    getNodeInstructionsEditor: deps.workflowNodeInstructionsEditor,
    getAgentPaneEntries: deps.agentPaneEntries,
    updateNodeInstructionsDraft: (draft) => {
      nodeInstructionsEditorBridge.updateDraft(draft)
    },
    setNodeInstructionsInputRef: (editorRef) => {
      nodeInstructionsEditorBridge.setInputRef(editorRef)
    },
  })

  return {
    workflowInspector: workflowInspectorController.project,
    bindWorkflowNodeInstructionsEditor: (controller: WorkflowNodeInstructionsEditorBridge) => {
      nodeInstructionsEditorBridge.updateDraft = controller.updateDraft
      nodeInstructionsEditorBridge.setInputRef = controller.setInputRef
    },
  }
}

export type CliAppWorkflowActionCompositionDeps = Record<string, any> & {
  client: any
  bindWorkflowNodeInstructionsEditor: AnyFn
  workflowNodeInstructionsEditor: AnyFn
  setWorkflowNodeInstructionsEditor: AnyFn
  workflowScreenShowing: AnyFn
  setWorkspaceScreenMode: AnyFn
  rebuildTranscript: AnyFn
  scheduleTimer: AnyFn
  focusPromptInput: AnyFn
  setWorkflowInspectorMode: AnyFn
  setSelectedWorkflowId: AnyFn
  isAttached: AnyFn
  sessionState: AnyFn
  applySessionState: AnyFn
  selectedWorkflowId: AnyFn
  selectedWorkflowNodeId: AnyFn
  setSelectedWorkflowNodeId: AnyFn
  setSelectedWorkflowComponent: AnyFn
  workspaceScreenMode: AnyFn
  applyResponseLayout: AnyFn
}

export function createCliAppWorkflowActionComposition(
  deps: CliAppWorkflowActionCompositionDeps,
) {
  const workflowNodeInstructionsEditorController = createWorkflowNodeInstructionsEditorController({
    getEditor: deps.workflowNodeInstructionsEditor,
    setEditor: deps.setWorkflowNodeInstructionsEditor,
    workflowScreenShowing: deps.workflowScreenShowing,
    setWorkspaceScreenMode: deps.setWorkspaceScreenMode,
    rebuildTranscript: deps.rebuildTranscript,
    scheduleTimer: deps.scheduleTimer,
    focusPromptInput: deps.focusPromptInput,
  })
  deps.bindWorkflowNodeInstructionsEditor({
    updateDraft: workflowNodeInstructionsEditorController.updateDraft,
    setInputRef: workflowNodeInstructionsEditorController.setInputRef,
  })

  const openWorkflowTerminalPanel = createWorkflowTerminalPanelController({
    clearNodeInstructionsEditor: workflowNodeInstructionsEditorController.clear,
    setWorkflowInspectorMode: deps.setWorkflowInspectorMode,
    setSelectedWorkflowId: deps.setSelectedWorkflowId,
    workflowScreenShowing: deps.workflowScreenShowing,
    setWorkspaceScreenMode: deps.setWorkspaceScreenMode,
    rebuildTranscript: deps.rebuildTranscript,
  }).open

  const workflowController = createWorkflowController({
    sendRequest: (request) => deps.client.send(request) as Promise<Record<string, unknown>>,
    isAttached: deps.isAttached,
    sessionState: deps.sessionState,
    applySessionState: deps.applySessionState,
    selectedWorkflowId: deps.selectedWorkflowId,
    setSelectedWorkflowId: deps.setSelectedWorkflowId,
    selectedWorkflowNodeId: deps.selectedWorkflowNodeId,
    setSelectedWorkflowNodeId: deps.setSelectedWorkflowNodeId,
    setSelectedWorkflowComponent: deps.setSelectedWorkflowComponent,
    setWorkflowInspectorMode: deps.setWorkflowInspectorMode,
    workspaceScreenMode: deps.workspaceScreenMode,
    setWorkspaceScreenMode: deps.setWorkspaceScreenMode,
    rebuildTranscript: deps.rebuildTranscript,
    applyResponseLayout: deps.applyResponseLayout,
  })

  return {
    openWorkflowNodeInstructionsEditor: workflowNodeInstructionsEditorController.open,
    closeWorkflowNodeInstructionsEditor: workflowNodeInstructionsEditorController.close,
    getWorkflowNodeInstructionsContext: workflowNodeInstructionsEditorController.context,
    getWorkflowNodeInstructionsDraft: workflowNodeInstructionsEditorController.draft,
    openWorkflowTerminalPanel,
    ...workflowController,
  }
}
