import {
  externalProviderSessionSelectionIndex,
  externalProviderSessionsSorted,
} from "@arroba/kernel-client/external-provider-sessions"
import { queuedPromptActionState } from "@arroba/kernel-client/queued-prompt-controls"
import type {
  CliOptions,
  ExternalProviderSessionRecord,
  PromptAttachmentPart,
  RuntimeAttachment,
  RuntimeSession,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import type { ArrobaLogger } from "./logging.js"
import type { QueuedPromptStripItem } from "@arroba/kernel-client/queued-prompt-strip-state"
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
import { resolveAttachTimeProviderLaunch } from "@arroba/kernel-client/session-lifecycle-state"
import type { WaitingRoomState } from "./waiting-room-types.js"
import type { WaitingRoomProjectSummary } from "./waiting-room-projects.js"
import { waitingRoomProjectsForNavigation } from "./waiting-room-project-rows.js"

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
  requestWaitingRoom?: (() => Promise<boolean>) | undefined
  connectDetachedKernelFromWaitingRoom: () => Promise<void>
  refreshWaitingRoomData: () => Promise<void>
  submitFocusedInteractionChoice: (choiceIndex?: number) => Promise<unknown>
  cycleFocusedInteractionChoice: (delta: number) => void
  setInteractionCustomReply: (interactionId: string, reply: string) => void
  setInteractionCustomEditing: (interactionId: string, editing: boolean) => void
  toggleTurn: (turnId: number, toggleEntryId?: number) => void
  toggleAgentPaneTurn?: ((agentId: string, turnId: number, toggleEntryId?: number) => void) | undefined
  toggleBlob: (entryId: number, collapsed: boolean) => void
  toggleAgentPaneBlob?: ((agentId: string, entryId: number, collapsed: boolean) => void) | undefined
  queuedPromptStripItemsForAgent: (agentId: string | null | undefined) => readonly QueuedPromptStripItem[]
  selectedQueuedPromptIndexForAgent: (agentId: string | null | undefined) => number
  onQueuedPromptAction: (item: QueuedPromptStripItem, action: "steer" | "cancel") => void | Promise<void>
  restoreTerminalAndExit: (exitCode: number) => Promise<void>
  waitingRoomState: () => WaitingRoomState
  setWaitingRoomState: (state: WaitingRoomState) => void
  externalProviderSessionsState: () => ExternalProviderSessionRecord[]
  waitingRoomProjects?: () => WaitingRoomProjectSummary[]
  applyWaitingRoomSessionLifecycleAction?: (action: "archive" | "delete", state?: WaitingRoomState) => Promise<void>
  restoreWaitingRoomProject?: (projectId: string) => Promise<void>
  renameWaitingRoomProject?: (projectId: string, name: string) => Promise<void>
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
            deps.sessionState,
            deps.options,
            deps.appLogger,
          )
          return deps.snapshot()
        }
        const session = deps.sessionState()
        if (deps.isAttached()) {
          const launchDecision = resolveAttachTimeProviderLaunch(session, {
            provider: deps.options.provider ?? "opencode",
            model: deps.options.model,
            effort: deps.options.effort,
          }, false)
          if (launchDecision.action === "launch_provider_run") {
            await launchProviderRun(
              deps.client,
              session.id,
              launchDecision.launch.provider,
              deps.options.accountProfile,
              launchDecision.launch.model,
              launchDecision.launch.effort,
              launchDecision.targetAgentId,
            )
            await resizeSessionTerminal(deps.client, session.id)
          }
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
      case "request_waiting_room": {
        if (!deps.isAttached()) {
          await deps.activateWaitingRoom()
          return deps.snapshot()
        }
        if (!deps.requestWaitingRoom) {
          throw new Error("request_waiting_room is not available in this CLI")
        }
        await deps.requestWaitingRoom()
        return deps.snapshot()
      }
      case "activate_unattached_agent": {
        if (deps.isAttached()) {
          throw new Error("cannot activate unattached agent while attached")
        }
        const sessions = externalProviderSessionsSorted(deps.externalProviderSessionsState())
        const externalSessionId = typeof request.externalSessionId === "string" ? request.externalSessionId : ""
        const requestedIndex = typeof request.externalSessionIndex === "number" ? request.externalSessionIndex : null
        const candidateIndex = externalSessionId
          ? sessions.findIndex((session) => session.external_session_id === externalSessionId)
          : requestedIndex === null
            ? null
            : externalProviderSessionSelectionIndex(sessions, {
                selectedExternalProviderSessionIndex: requestedIndex,
              })
        if (
          typeof candidateIndex !== "number"
          || !Number.isInteger(candidateIndex)
          || candidateIndex < 0
          || candidateIndex >= sessions.length
        ) {
          throw new Error("usage: activate_unattached_agent externalSessionId=<id> or externalSessionIndex=<index>")
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
          await deps.refreshWaitingRoomData()
          return deps.snapshot()
        }
        await deps.connectDetachedKernelFromWaitingRoom()
        return deps.snapshot()
      }
      case "refresh_waiting_room": {
        await deps.refreshWaitingRoomData()
        return deps.snapshot()
      }
      case "set_waiting_room_launch": {
        if (deps.isAttached()) {
          throw new Error("cannot set waiting room launch while attached")
        }
        const machineRef = typeof request.machineRef === "string" ? request.machineRef : undefined
        const kernelRef = typeof request.kernelRef === "string" ? request.kernelRef : undefined
        const providerId = typeof request.providerId === "string" ? request.providerId : undefined
        const modelId = typeof request.modelId === "string" ? request.modelId : undefined
        const effort = typeof request.effort === "string" ? request.effort : undefined
        const projectSelectionId = typeof request.projectSelectionId === "string" ? request.projectSelectionId : undefined
        const showArchivedProjects = typeof request.showArchivedProjects === "boolean" ? request.showArchivedProjects : undefined
        const focus = request.focus === "launch-machine" || request.focus === "launch-kernel" || request.focus === "project" || request.focus === "new"
          ? request.focus
          : undefined
        if (
          machineRef === undefined
          && kernelRef === undefined
          && providerId === undefined
          && modelId === undefined
          && effort === undefined
          && projectSelectionId === undefined
          && showArchivedProjects === undefined
          && focus === undefined
        ) {
          throw new Error("usage: set_waiting_room_launch machineRef=<id> kernelRef=<id> providerId=<id> modelId=<id> effort=<level> projectSelectionId=default|new|existing:<id> showArchivedProjects=<boolean> focus=new|launch-machine|launch-kernel|project")
        }
        deps.setWaitingRoomState({
          ...deps.waitingRoomState(),
          ...(machineRef !== undefined ? { selectedMachineRef: machineRef } : {}),
          ...(kernelRef !== undefined ? { selectedKernelRef: kernelRef } : {}),
          ...(providerId !== undefined ? { providerId: providerId as WaitingRoomState["providerId"] } : {}),
          ...(modelId !== undefined ? { modelId } : {}),
          ...(effort !== undefined ? { effort } : {}),
          ...(projectSelectionId !== undefined ? { projectSelectionId } : {}),
          ...(showArchivedProjects !== undefined ? { showArchivedProjects } : {}),
          ...(focus !== undefined ? { focus } : {}),
        })
        return deps.snapshot()
      }
      case "select_waiting_room_project": {
        if (deps.isAttached()) throw new Error("cannot select a waiting-room project while attached")
        const projectId = typeof request.projectId === "string" ? request.projectId : ""
        const projectIndex = waitingRoomProjectsForNavigation(deps.waitingRoomProjects?.() ?? []).findIndex((project) => project.id === projectId)
        if (projectIndex < 0) throw new Error("usage: select_waiting_room_project projectId=<id>")
        deps.setWaitingRoomState({ ...deps.waitingRoomState(), focus: "project-entry", projectIndex })
        return deps.snapshot()
      }
      case "waiting_room_project_action": {
        if (deps.isAttached()) throw new Error("cannot mutate a waiting-room project while attached")
        const projectId = typeof request.projectId === "string" ? request.projectId : ""
        const projectIndex = waitingRoomProjectsForNavigation(deps.waitingRoomProjects?.() ?? []).findIndex((project) => project.id === projectId)
        if (projectIndex < 0) throw new Error("usage: waiting_room_project_action projectId=<id> projectAction=rename|archive|delete|restore")
        const projectAction = request.projectAction
        if (projectAction === "rename") {
          const name = typeof request.projectName === "string" ? request.projectName.trim() : ""
          if (!name) throw new Error("waiting_room_project_action rename requires projectName=<name>")
          if (!deps.renameWaitingRoomProject) throw new Error("project rename is unavailable in this CLI")
          await deps.renameWaitingRoomProject(projectId, name)
        } else if (projectAction === "restore") {
          if (!deps.restoreWaitingRoomProject) throw new Error("project restore is unavailable in this CLI")
          await deps.restoreWaitingRoomProject(projectId)
        } else if (projectAction === "archive" || projectAction === "delete") {
          if (!deps.applyWaitingRoomSessionLifecycleAction) throw new Error("project lifecycle is unavailable in this CLI")
          const state = { ...deps.waitingRoomState(), focus: "project-entry" as const, projectIndex }
          deps.setWaitingRoomState(state)
          await deps.applyWaitingRoomSessionLifecycleAction(projectAction, state)
          await deps.applyWaitingRoomSessionLifecycleAction(projectAction, state)
        } else {
          throw new Error("waiting_room_project_action projectAction must be rename|archive|delete|restore")
        }
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
        const agentId = typeof request.agentId === "string" ? request.agentId : null
        if (agentId) {
          if (!deps.toggleAgentPaneBlob) {
            throw new Error("toggle_blob agentId=<id> is not available in this CLI")
          }
          deps.toggleAgentPaneBlob(agentId, entryId, request.collapsed === true)
          return deps.snapshot()
        }
        deps.toggleBlob(entryId, request.collapsed === true)
        return deps.snapshot()
      }
      case "toggle_turn": {
        const turnId = typeof request.turnId === "number" ? request.turnId : Number.NaN
        const toggleEntryId = typeof request.entryId === "number" ? request.entryId : undefined
        if (!Number.isInteger(turnId)) {
          throw new Error("usage: toggle_turn turnId=<integer> [entryId=<integer>]")
        }
        const agentId = typeof request.agentId === "string" ? request.agentId : null
        if (agentId) {
          if (!deps.toggleAgentPaneTurn) {
            throw new Error("toggle_turn agentId=<id> is not available in this CLI")
          }
          deps.toggleAgentPaneTurn(agentId, turnId, toggleEntryId)
          return deps.snapshot()
        }
        deps.toggleTurn(turnId, toggleEntryId)
        return deps.snapshot()
      }
      case "queued_prompt_action": {
        const queuedPromptAction = request.queuedPromptAction === "steer" || request.queuedPromptAction === "cancel"
          ? request.queuedPromptAction
          : null
        if (!queuedPromptAction) {
          throw new Error("usage: queued_prompt_action queuedPromptAction=steer|cancel [agentId=<id>] [promptId=<id>]")
        }
        const agentId = typeof request.agentId === "string" ? request.agentId : deps.focusedAgentId()
        const items = deps.queuedPromptStripItemsForAgent(agentId)
        const requestedPromptId = typeof request.promptId === "string" ? request.promptId : null
        const item = requestedPromptId
          ? items.find((candidate) => candidate.promptId === requestedPromptId)
          : items[deps.selectedQueuedPromptIndexForAgent(agentId)] ?? items[0]
        if (!item) {
          throw new Error("queued_prompt_action could not find a queued prompt strip item")
        }
        const actionState = queuedPromptActionState(item, queuedPromptAction)
        if (actionState.disabled) {
          throw new Error(actionState.disabledReason ?? "queued prompt action is unavailable")
        }
        await deps.onQueuedPromptAction(item, queuedPromptAction)
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
