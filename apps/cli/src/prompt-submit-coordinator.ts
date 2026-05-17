import { SESSION_NEW_ERROR_HINT } from "./sessions.js"
import { isWorkspaceShellCommand } from "./workspace-shell.js"
import { isWorkflowCommandInput } from "./workflow-prompt-state.js"

export type PromptSubmitCoordinatorDeps = {
  getPromptText: () => string | null | undefined
  ensureBackgroundPollersStarted: () => void
  getPendingAttachmentCount: () => number
  clearPromptText: () => void
  workflowScreenShowing: () => boolean
  submitWorkspaceShellCommand: (rawPrompt: string) => Promise<void>
  workflowNodeInstructionsEditorOpen: () => boolean
  submitSlashCommand: (
    rawPrompt: string,
    options: {
      allowSlashCommandSubmission: boolean
      trimmedPrompt: string
    },
  ) => Promise<boolean>
  submitProviderNamespacePrompt: (rawPrompt: string) => Promise<boolean>
  isAttached: () => boolean
  submitWorkflowPrompt: (rawPrompt: string) => Promise<void>
  submitNormalPrompt: (rawPrompt: string) => Promise<void>
  flashFooter: (message: string, tone: "info" | "error") => void
  formatError?: (error: unknown) => string
}

export type PromptSubmitCoordinator = {
  submit(): Promise<void>
}

export function createPromptSubmitCoordinator(
  deps: PromptSubmitCoordinatorDeps,
): PromptSubmitCoordinator {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  return {
    async submit() {
      const rawPrompt = deps.getPromptText()
      if (rawPrompt === null || rawPrompt === undefined) {
        return
      }

      deps.ensureBackgroundPollersStarted()

      const trimmed = rawPrompt.trim()
      if (!trimmed && deps.getPendingAttachmentCount() === 0) {
        deps.clearPromptText()
        return
      }
      if (deps.workflowScreenShowing() && isWorkspaceShellCommand(rawPrompt)) {
        try {
          await deps.submitWorkspaceShellCommand(rawPrompt)
        } catch (error) {
          deps.flashFooter(formatError(error), "error")
        } finally {
          deps.clearPromptText()
        }
        return
      }
      if (deps.workflowNodeInstructionsEditorOpen() && !trimmed.startsWith("/")) {
        deps.flashFooter("instructions editor is open; type in the I/O panel and use /workflow node instructions save", "info")
        deps.clearPromptText()
        return
      }
      const allowSlashCommandSubmission = !deps.workflowScreenShowing() || isWorkflowCommandInput(rawPrompt)
      const handledCommand = await deps.submitSlashCommand(rawPrompt, {
        allowSlashCommandSubmission,
        trimmedPrompt: trimmed,
      })
      if (handledCommand) {
        return
      }
      if (await deps.submitProviderNamespacePrompt(rawPrompt)) {
        return
      }
      if (!deps.isAttached()) {
        deps.flashFooter(SESSION_NEW_ERROR_HINT, "error")
        deps.clearPromptText()
        return
      }

      if (deps.workflowScreenShowing()) {
        await deps.submitWorkflowPrompt(rawPrompt)
        return
      }

      await deps.submitNormalPrompt(rawPrompt)
    },
  }
}
