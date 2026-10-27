import type { WorkspaceScreenMode } from "./workspace-screen.js"

export type WorkflowNodeInstructionsEditor = {
  workflowId: string
  nodeId: string
  draft: string
}

export type WorkflowNodeInstructionsInputRef = {
  focus(): void
}

type WorkflowNodeInstructionsEditorControllerDeps<TimerHandle> = {
  getEditor: () => WorkflowNodeInstructionsEditor | null
  setEditor: (editor: WorkflowNodeInstructionsEditor | null) => void
  workflowScreenShowing: () => boolean
  setWorkspaceScreenMode: (mode: WorkspaceScreenMode) => void
  rebuildTranscript: () => void
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  focusPromptInput: () => void
}

export type WorkflowNodeInstructionsEditorController = {
  open(workflowId: string, nodeId: string, draft: string): void
  close(): void
  clear(): void
  updateDraft(draft: string): void
  context(): { workflowId: string; nodeId: string } | null
  draft(): string
  setInputRef(input: WorkflowNodeInstructionsInputRef | null): void
}

export function createWorkflowNodeInstructionsEditorController<TimerHandle>(
  deps: WorkflowNodeInstructionsEditorControllerDeps<TimerHandle>,
): WorkflowNodeInstructionsEditorController {
  let inputRef: WorkflowNodeInstructionsInputRef | undefined

  return {
    open(workflowId, nodeId, draft) {
      deps.setEditor({ workflowId, nodeId, draft })
      if (!deps.workflowScreenShowing()) {
        deps.setWorkspaceScreenMode("workflow")
      }
      deps.rebuildTranscript()
      deps.scheduleTimer(() => {
        inputRef?.focus()
      }, 0)
    },
    close() {
      if (!deps.getEditor()) {
        return
      }
      deps.setEditor(null)
      inputRef = undefined
      if (deps.workflowScreenShowing()) {
        deps.rebuildTranscript()
      }
      deps.focusPromptInput()
    },
    clear() {
      if (!deps.getEditor()) {
        return
      }
      deps.setEditor(null)
      inputRef = undefined
    },
    updateDraft(draft) {
      const editor = deps.getEditor()
      if (!editor) {
        return
      }
      deps.setEditor({ ...editor, draft })
    },
    context() {
      const editor = deps.getEditor()
      if (!editor) {
        return null
      }
      return { workflowId: editor.workflowId, nodeId: editor.nodeId }
    },
    draft() {
      return deps.getEditor()?.draft ?? ""
    },
    setInputRef(input) {
      inputRef = input ?? undefined
    },
  }
}
