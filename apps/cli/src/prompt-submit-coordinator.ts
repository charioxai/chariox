import { SESSION_NEW_ERROR_HINT } from "./sessions.js"
import { isWorkspaceShellCommand } from "./workspace-shell.js"
import type { WaitingRoomPromptBootstrapResult } from "./waiting-room-prompt-bootstrap-controller.js"
import { isWorkflowCommandInput } from "@chariox/kernel-client/workflow-prompt-state"
import {
  detachedPromptSubmitDecision,
  promptSubmitPreparationDecision,
} from "@chariox/kernel-client/prompt-submission"

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
  submitDetachedSlashCommand?: (rawPrompt: string) => Promise<boolean>
  submitProviderNamespacePrompt: (rawPrompt: string) => Promise<boolean>
  bootstrapDetachedPrompt?: (rawPrompt: string) => Promise<WaitingRoomPromptBootstrapResult>
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

      const preparation = promptSubmitPreparationDecision({
        rawPrompt,
        pendingAttachmentCount: deps.getPendingAttachmentCount(),
        workflowScreenShowing: deps.workflowScreenShowing(),
        workspaceShellCommand: isWorkspaceShellCommand(rawPrompt),
        workflowNodeInstructionsEditorOpen: deps.workflowNodeInstructionsEditorOpen(),
        workflowCommandInput: isWorkflowCommandInput(rawPrompt),
      })
      if (preparation.action === "clear_empty") {
        deps.clearPromptText()
        return
      }
      if (preparation.action === "workspace_shell") {
        try {
          await deps.submitWorkspaceShellCommand(rawPrompt)
        } catch (error) {
          deps.flashFooter(formatError(error), "error")
        } finally {
          deps.clearPromptText()
        }
        return
      }
      if (preparation.action === "instructions_editor_open") {
        deps.flashFooter("instructions editor is open; type in the I/O panel and use /workflow node instructions save", "info")
        deps.clearPromptText()
        return
      }
      if (!deps.isAttached() && await deps.submitDetachedSlashCommand?.(rawPrompt)) {
        return
      }
      const handledCommand = await deps.submitSlashCommand(rawPrompt, {
        allowSlashCommandSubmission: preparation.allowSlashCommandSubmission,
        trimmedPrompt: preparation.trimmedPrompt,
      })
      if (handledCommand) {
        return
      }
      if (await deps.submitProviderNamespacePrompt(rawPrompt)) {
        return
      }
      if (!deps.isAttached()) {
        const detachedDecision = detachedPromptSubmitDecision({
          trimmedPrompt: preparation.trimmedPrompt,
          pendingAttachmentCount: deps.getPendingAttachmentCount(),
        })
        if (detachedDecision.action === "flash_start_or_join_session") {
          deps.flashFooter("start or join a session first", "error")
          return
        }
        if (detachedDecision.action === "flash_attachments_require_session") {
          deps.flashFooter("attachments require an open session", "error")
          return
        }
        const bootstrapResult = await deps.bootstrapDetachedPrompt?.(rawPrompt) ?? "unhandled"
        const bootstrapDecision = detachedPromptSubmitDecision({
          trimmedPrompt: preparation.trimmedPrompt,
          pendingAttachmentCount: deps.getPendingAttachmentCount(),
          bootstrapResult,
          attachedAfterBootstrap: deps.isAttached(),
        })
        if (bootstrapDecision.action === "keep_bootstrap_handled") {
          return
        }
        if (bootstrapDecision.action === "submit_bootstrapped_prompt") {
          await deps.submitNormalPrompt(rawPrompt)
          return
        }
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
