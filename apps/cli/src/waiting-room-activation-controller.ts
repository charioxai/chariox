import type { RuntimeSession } from "./cli-types.js"
import type {
  BackendProviderId,
  ProviderCatalog,
} from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import {
  deriveWaitingRoomActivationDecision,
  deriveWaitingRoomControlActivationDecision,
  type WaitingRoomActivationDecision,
  type WaitingRoomControlActivationDecision,
  type WaitingRoomLaunchConfig,
} from "./waiting-room-controller.js"
import type {
  WaitingRoomRemoteState,
  WaitingRoomState,
} from "./waiting-room-types.js"

export type WaitingRoomCreateSessionLaunch = WaitingRoomLaunchConfig & {
  account_profile: string | null
  execution_mode: "build"
  permission_level: "yolo"
}

export type WaitingRoomActivationControllerDeps = {
  isKernelConnected: () => boolean
  connectKernel: () => Promise<void>
  getWaitingRoomState: () => WaitingRoomState
  getRemoteState: () => WaitingRoomRemoteState
  getWorkspaceTarget: () => string
  getWorktreeTarget: () => string
  getAvailableSessions: () => SessionListEntry[]
  getProviderCatalog: () => ProviderCatalog
  getCurrentProvider: () => BackendProviderId
  getCurrentModel: () => string
  getAccountProfile: () => string | null | undefined
  handleCloudCommand: () => Promise<void>
  setPromptText: (text: string) => void
  focusPrompt: () => void
  syncCommandCenter: (text: string) => void
  openTerminalPairingDialog: () => Promise<void>
  openSessionBrowserDialog: () => void
  createSession: (
    workspacePath: string,
    worktreePath: string,
    launch: WaitingRoomCreateSessionLaunch,
  ) => Promise<Pick<RuntimeSession, "id"> & Partial<RuntimeSession>>
  attachBinding: (
    session: Pick<RuntimeSession, "id"> & Partial<RuntimeSession>,
    createdSession: boolean,
    launch: WaitingRoomLaunchConfig,
  ) => Promise<void>
  flashFooter: (message: string, tone: "info" | "error") => void
  warn: (message: string, fields: Record<string, unknown>) => void
  formatError: (error: unknown) => string
  deriveControlDecision?: typeof deriveWaitingRoomControlActivationDecision
  deriveActivationDecision?: typeof deriveWaitingRoomActivationDecision
}

export function createWaitingRoomActivationController(
  deps: WaitingRoomActivationControllerDeps,
) {
  const deriveControlDecision = deps.deriveControlDecision ?? deriveWaitingRoomControlActivationDecision
  const deriveActivationDecision = deps.deriveActivationDecision ?? deriveWaitingRoomActivationDecision

  const activate = async () => {
    try {
      if (!deps.isKernelConnected()) {
        await deps.connectKernel()
      }

      const remote = deps.getRemoteState()
      const controlDecision = deriveControlDecision({
        state: deps.getWaitingRoomState(),
        workspacePath: deps.getWorkspaceTarget(),
        worktreePath: deps.getWorktreeTarget(),
        remote,
      })
      if (await applyControlDecision(controlDecision)) {
        return
      }

      const decision = deriveActivationDecision({
        state: deps.getWaitingRoomState(),
        sessions: deps.getAvailableSessions(),
        catalog: deps.getProviderCatalog(),
        currentProvider: deps.getCurrentProvider(),
        currentModel: deps.getCurrentModel(),
        remote,
      })
      await applyActivationDecision(decision)
    } catch (error) {
      deps.warn("waiting room activation failed", {
        error: deps.formatError(error),
      })
      deps.flashFooter(deps.formatError(error), "error")
    }
  }

  const applyControlDecision = async (
    decision: WaitingRoomControlActivationDecision,
  ) => {
    if (decision.action === "none") {
      return false
    }
    if (decision.action === "cloud") {
      await deps.handleCloudCommand()
      return true
    }
    if (decision.action === "stage-command") {
      deps.setPromptText(decision.command)
      deps.focusPrompt()
      deps.syncCommandCenter(decision.command)
      deps.flashFooter(decision.message, "info")
      return true
    }
    if (decision.action === "open-terminal-pairing") {
      await deps.openTerminalPairingDialog()
      return true
    }
    if (decision.action === "open-session-browser") {
      deps.openSessionBrowserDialog()
      return true
    }
    deps.flashFooter(decision.message, decision.action === "error" ? "error" : "info")
    return true
  }

  const applyActivationDecision = async (
    decision: WaitingRoomActivationDecision,
  ) => {
    if (decision.action === "create") {
      const session = await deps.createSession(
        deps.getWorkspaceTarget(),
        deps.getWorktreeTarget(),
        {
          provider: decision.launch.provider,
          model: decision.launch.model,
          effort: decision.launch.effort,
          account_profile: deps.getAccountProfile() ?? null,
          execution_mode: "build",
          permission_level: "yolo",
          ...(decision.launch.sliceRef ? { sliceRef: decision.launch.sliceRef } : {}),
        },
      )
      await deps.attachBinding(session, true, decision.launch)
      deps.flashFooter(`created session ${session.alias ?? session.id}`, "info")
      return
    }
    if (decision.action === "join") {
      await deps.attachBinding(decision.session, false, decision.launch)
      deps.flashFooter(`attached to session ${decision.session.alias ?? decision.session.id}`, "info")
      return
    }
    if (decision.action === "error") {
      deps.flashFooter(decision.message, "error")
    }
  }

  return {
    activate,
  }
}
