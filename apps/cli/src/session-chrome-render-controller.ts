import type { RuntimeSession } from "./cli-types.js"
import type { WorkspaceLiveSyncStatus } from "./cli-types.js"
import type { FooterFlash } from "./footer-flash-controller.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import type { PromptMetaPart } from "@arroba/kernel-client/prompt-meta"
import {
  sessionAttachedFooterSummary,
} from "@arroba/kernel-client/shell-session-footer"
import type {
  SessionStatusMode,
} from "@arroba/kernel-client/session-runtime-status"
import { compactFooterSummary } from "./footer-summary-compact.js"
import {
  SESSION_NEW_FOOTER_HINT,
} from "./sessions.js"

export type SessionChromeSummaryRenderOptions<TState, TBox = unknown> = {
  renderer: unknown
  state: TState
  promptStateBox: TBox | undefined
  footerSummaryBox: TBox | undefined
  promptStateLabel: string
  promptStateTone: "error" | "thinking" | "muted"
  footerSummary: string
  footerFlash: FooterFlash | null
}

export type SessionChromeRenderControllerDeps<TState, TBox = unknown> = {
  renderer: unknown
  createSummaryRenderState: () => TState
  renderSummary: (options: SessionChromeSummaryRenderOptions<TState, TBox>) => void
  syncPromptPlaceholder: () => void
  getFatalError: () => string | null
  getSubmitting: () => boolean
  getFooterHint: () => string
  isAttached: () => boolean
  getSession: () => RuntimeSession
  getConnectedClientCount: () => number
  getMultiAgentMode: () => boolean
  getResponseLayout: () => MultiAgentResponseLayout
  getSessionStatusMode: () => SessionStatusMode
  getWorkspaceLiveSyncStatus: () => WorkspaceLiveSyncStatus | null
  getHotkeyToggleLabel: () => string
  getTerminalWidth?: () => number
  getFooterFlash: () => FooterFlash | null
  getPromptMetaParts: () => PromptMetaPart[]
  setPromptMetaRenderables: (parts: PromptMetaPart[]) => void
  renderStatusIndicator: () => void
  renderSplitPaneFooters: () => void
  renderAgentInteractions: () => void
  getWorking: () => boolean
  getActiveStatusLabel: () => string | null
  getProviderActivityLabel: () => string | null
  getStreamingAgentId: () => string | null
}

export function createSessionChromeRenderController<TState, TBox = unknown>(
  deps: SessionChromeRenderControllerDeps<TState, TBox>,
) {
  const state = deps.createSummaryRenderState()
  let promptStateBox: TBox | undefined
  let footerSummaryBox: TBox | undefined

  const apply = () => {
    const footerFlash = deps.getFooterFlash()
    const footerSummary = deps.isAttached()
      ? sessionAttachedFooterSummary({
        session: deps.getSession(),
        connectedClientCount: deps.getConnectedClientCount(),
        multiAgentMode: deps.getMultiAgentMode(),
        sessionStatusMode: deps.getSessionStatusMode(),
        hotkeyToggleLabel: deps.getHotkeyToggleLabel(),
        workspaceLiveSyncStatus: deps.getWorkspaceLiveSyncStatus(),
      })
      : SESSION_NEW_FOOTER_HINT
    const footerFlashWidth = footerFlash ? ` • ${footerFlash.message}`.length : 0
    const compactFooterWidth = deps.getTerminalWidth
      ? Math.max(0, deps.getTerminalWidth() - footerFlashWidth)
      : null
    deps.syncPromptPlaceholder()
    deps.renderSummary({
      renderer: deps.renderer,
      state,
      promptStateBox,
      footerSummaryBox,
      promptStateLabel: deps.getFatalError()
        ? "error"
        : deps.getSubmitting()
          ? "thinking"
          : deps.getFooterHint(),
      promptStateTone: deps.getFatalError()
        ? "error"
        : deps.getSubmitting()
          ? "thinking"
          : "muted",
      footerSummary: compactFooterSummary(footerSummary, compactFooterWidth),
      footerFlash,
    })
    deps.setPromptMetaRenderables(deps.isAttached() ? deps.getPromptMetaParts() : [])
    deps.renderStatusIndicator()
    deps.renderSplitPaneFooters()
    deps.renderAgentInteractions()
  }

  const shouldThrottle = () => (
    deps.getWorking()
    || Boolean(deps.getActiveStatusLabel())
    || Boolean(deps.getProviderActivityLabel())
    || Boolean(deps.getStreamingAgentId())
  )

  return {
    assignPromptStateBox(value: TBox | undefined) {
      promptStateBox = value
    },
    assignFooterSummaryBox(value: TBox | undefined) {
      footerSummaryBox = value
    },
    apply,
    shouldThrottle,
  }
}
