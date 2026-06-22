import type { CliOptions, PromptAttachmentPart, RuntimeAttachment, RuntimeSession } from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import type { ArrobaLogger } from "./logging.js"
import type { WorkspaceScreenMode } from "./workspace-screen.js"
import {
  automationSnapshotMatches,
  type CliAutomationRequest,
  type CliAutomationSnapshot,
} from "./cli-automation.js"
import {
  preparePromptAttachmentsForSubmit,
  promptAttachmentTransferIsForced,
} from "./prompt-attachment-transfer.js"
import { submitPromptWithRecovery } from "./prompt-runtime-api.js"
import { launchProviderRun } from "./provider-api.js"
import { resizeSessionTerminal } from "./session-runtime-api.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

export type CliAutomationActionDeps = {
  client: LocalIpcClient
  options: CliOptions
  appLogger: ArrobaLogger | null
  snapshot: () => CliAutomationSnapshot
  isAttached: () => boolean
  kernelConnected: () => boolean
  workflowScreenActive: () => boolean
  setWorkspaceScreenMode: (screen: WorkspaceScreenMode) => void
  rebuildTranscript: () => void
  applyResponseLayout: () => void
  showWorkflowScreen: () => void
  submitWorkspaceShellCommand: (rawPrompt: string) => Promise<unknown>
  attachmentState: () => RuntimeAttachment | null
  sessionState: () => RuntimeSession
  focusedAgentId: () => string | null
  setPromptText: (value: string) => void
  submitPrompt: () => Promise<void>
  activateWaitingRoom: () => Promise<void>
  connectDetachedKernelFromWaitingRoom: () => Promise<void>
  submitFocusedInteractionChoice: (choiceIndex?: number) => Promise<unknown>
  cycleFocusedInteractionChoice: (delta: number) => void
  setInteractionCustomReply: (interactionId: string, reply: string) => void
  setInteractionCustomEditing: (interactionId: string, editing: boolean) => void
  toggleBlob: (entryId: number, collapsed: boolean) => void
  restoreTerminalAndExit: (exitCode: number) => Promise<void>
  waitingRoomState: () => WaitingRoomState
  setWaitingRoomState: (state: WaitingRoomState) => void
  externalProviderSessionsState: () => Array<{ external_session_id: string }>
  sleep?: (ms: number) => Promise<void>
}

export function createCliAutomationActionHandler(deps: CliAutomationActionDeps) {
  return async function handleAutomationRequest(request: CliAutomationRequest): Promise<unknown> {
    const action = typeof request.action === "string" ? request.action : ""
    switch (action) {
      case "ping":
        return { status: "ok" }
      case "switch_screen": {
        const screen = typeof request.screen === "string" ? request.screen : ""
        if (screen !== "agents" && screen !== "workflow") {
          throw new Error("usage: switch_screen screen=agents|workflow")
        }
        if (!deps.isAttached()) {
          throw new Error("cannot switch screen without an attached session")
        }
        deps.setWorkspaceScreenMode(screen)
        deps.rebuildTranscript()
        deps.applyResponseLayout()
        return deps.snapshot()
      }
      case "workspace_shell_exec": {
        const command = typeof request.command === "string" ? request.command : ""
        if (!command.trim()) {
          throw new Error("usage: workspace_shell_exec command=<arroba-shell command>")
        }
        if (!deps.workflowScreenActive()) {
          deps.showWorkflowScreen()
        }
        const result = await deps.submitWorkspaceShellCommand(`@ ${command}`)
        return { result, snapshot: deps.snapshot() }
      }
      case "submit_prompt": {
        const prompt = typeof request.prompt === "string" ? request.prompt : ""
        if (!prompt.trim()) {
          throw new Error("usage: submit_prompt prompt=<text>")
        }
        const requestAttachments = automationPromptAttachments(request.attachments)
        if (requestAttachments.length > 0) {
          if (!deps.isAttached()) {
            throw new Error("cannot submit prompt attachments without an attached session")
          }
          const attachment = deps.attachmentState()
          if (!attachment) {
            throw new Error("cannot submit prompt attachments without an attached client")
          }
          const attachments = await preparePromptAttachmentsForSubmit(requestAttachments, {
            inlineLocalFiles: Boolean(deps.options.relayUrl) || promptAttachmentTransferIsForced(),
          })
          await submitPromptWithRecovery(
            deps.client,
            deps.sessionState().id,
            attachment.id,
            deps.focusedAgentId(),
            prompt.endsWith("\n") ? prompt : `${prompt}\n`,
            attachments,
            deps.options,
            deps.appLogger,
          )
          return deps.snapshot()
        }
        const session = deps.sessionState()
        if (deps.isAttached() && !session.active_provider_run_id) {
          await launchProviderRun(
            deps.client,
            session.id,
            deps.options.provider ?? "opencode",
            deps.options.accountProfile,
            deps.options.model,
            deps.options.effort,
            deps.focusedAgentId(),
          )
          await resizeSessionTerminal(deps.client, session.id)
        }
        deps.setPromptText(prompt)
        await deps.submitPrompt()
        return deps.snapshot()
      }
      case "activate_waiting_room": {
        if (deps.isAttached()) {
          throw new Error("cannot activate waiting room while attached")
        }
        await deps.activateWaitingRoom()
        return deps.snapshot()
      }
      case "activate_orphan_agent": {
        if (deps.isAttached()) {
          throw new Error("cannot activate orphan agent while attached")
        }
        const sessions = deps.externalProviderSessionsState()
        const externalSessionId = typeof request.externalSessionId === "string" ? request.externalSessionId : ""
        const requestedIndex = typeof request.externalSessionIndex === "number" ? request.externalSessionIndex : null
        const candidateIndex = externalSessionId
          ? sessions.findIndex((session) => session.external_session_id === externalSessionId)
          : requestedIndex
        if (
          typeof candidateIndex !== "number"
          || !Number.isInteger(candidateIndex)
          || candidateIndex < 0
          || candidateIndex >= sessions.length
        ) {
          throw new Error("usage: activate_orphan_agent externalSessionId=<id> or externalSessionIndex=<index>")
        }
        deps.setWaitingRoomState({
          ...deps.waitingRoomState(),
          focus: "external-session",
          externalSessionIndex: candidateIndex,
        })
        await deps.activateWaitingRoom()
        return deps.snapshot()
      }
      case "connect_detached_kernel": {
        if (deps.kernelConnected()) {
          return deps.snapshot()
        }
        await deps.connectDetachedKernelFromWaitingRoom()
        return deps.snapshot()
      }
      case "snapshot":
        return deps.snapshot()
      case "interaction_submit": {
        const choiceIndex = typeof request.choiceIndex === "number" ? request.choiceIndex : undefined
        await deps.submitFocusedInteractionChoice(choiceIndex)
        return deps.snapshot()
      }
      case "interaction_move": {
        const delta = typeof request.delta === "number" ? request.delta : 0
        if (!delta) {
          throw new Error("usage: interaction_move delta=<signed integer>")
        }
        deps.cycleFocusedInteractionChoice(delta)
        return deps.snapshot()
      }
      case "interaction_custom_reply": {
        const reply = typeof request.reply === "string" ? request.reply : ""
        const interactionId = typeof request.interactionId === "string"
          ? request.interactionId
          : focusedInteractionId(deps.snapshot(), deps.focusedAgentId())
        if (!interactionId) {
          throw new Error("usage: interaction_custom_reply reply=<text> requires a focused interaction or interactionId")
        }
        deps.setInteractionCustomReply(interactionId, reply)
        deps.setInteractionCustomEditing(interactionId, Boolean(request.editing))
        return deps.snapshot()
      }
      case "toggle_blob": {
        const entryId = typeof request.entryId === "number" ? request.entryId : Number.NaN
        if (!Number.isInteger(entryId)) {
          throw new Error("usage: toggle_blob entryId=<integer> collapsed=<boolean>")
        }
        deps.toggleBlob(entryId, request.collapsed === true)
        return deps.snapshot()
      }
      case "wait_for": {
        const timeoutMs = typeof request.timeoutMs === "number" ? request.timeoutMs : 10_000
        const intervalMs = typeof request.intervalMs === "number" ? request.intervalMs : 100
        const deadline = Date.now() + Math.max(1, timeoutMs)
        const wait = deps.sleep ?? defaultSleep
        let snapshot = deps.snapshot()
        while (!automationSnapshotMatches(snapshot, request) && Date.now() < deadline) {
          await wait(Math.max(10, intervalMs))
          snapshot = deps.snapshot()
        }
        if (!automationSnapshotMatches(snapshot, request)) {
          throw new Error("timed out waiting for CLI automation condition")
        }
        return snapshot
      }
      case "exit":
        void deps.restoreTerminalAndExit(0)
        return { exiting: true }
      default:
        throw new Error(`unknown automation action '${action || String(request.action)}'`)
    }
  }
}

function focusedInteractionId(snapshot: CliAutomationSnapshot, focusedAgentId: string | null): string | null {
  const interactions = Array.isArray(snapshot.interactions) ? snapshot.interactions : []
  const interaction = interactions.find((entry) => entry.agentId === focusedAgentId) ?? interactions[0]
  return typeof interaction?.id === "string" ? interaction.id : null
}

function automationPromptAttachments(attachments: unknown): PromptAttachmentPart[] {
  return Array.isArray(attachments)
    ? attachments.map((entry) => {
      if (!entry || typeof entry !== "object") {
        throw new Error("submit_prompt attachments must be objects")
      }
      const attachment = entry as Record<string, unknown>
      if (typeof attachment.url !== "string" || typeof attachment.mime !== "string") {
        throw new Error("submit_prompt attachments require url and mime")
      }
      return {
        url: attachment.url,
        mime: attachment.mime,
        filename: typeof attachment.filename === "string" ? attachment.filename : null,
      }
    })
    : []
}

function defaultSleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}
