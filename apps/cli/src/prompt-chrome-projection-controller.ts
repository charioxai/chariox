import {
  derivePromptAreaBackground,
  derivePromptPlaceholder,
} from "@arroba/kernel-client/prompt-surface-state"
import {
  deriveFooterHint,
  deriveSessionStatusMode,
} from "./session-chrome-state.js"
import type { WorkflowPromptState } from "./workflow-prompt-state.js"

export type PromptChromeProjectionControllerDeps<Color> = {
  daemonDisconnected: () => boolean
  working: () => boolean
  hasActivePrompt: () => boolean
  submitting: () => boolean
  queueDepth: () => number
  fatalError: () => string | null
  activePromptId: () => string | null
  statusLine: () => string
  isAttached: () => boolean
  workflowScreenActive: () => boolean
  workflowPromptState: () => WorkflowPromptState
  attachedPlaceholder: string
  detachedPlaceholder: string
  trackThemeRevision?: () => unknown
  attachedBackground: () => Color
  detachedBackground: () => Color
  workflowBackground: () => Color
}

export function createPromptChromeProjectionController<Color>(
  deps: PromptChromeProjectionControllerDeps<Color>,
) {
  return {
    sessionStatusMode: () => deriveSessionStatusMode({
      daemonDisconnected: deps.daemonDisconnected(),
      working: deps.working(),
      hasActivePrompt: deps.hasActivePrompt(),
      submitting: deps.submitting(),
      queueDepth: deps.queueDepth(),
    }),
    footerHint: () => deriveFooterHint({
      fatalError: deps.fatalError(),
      activePromptId: deps.activePromptId(),
      queueDepth: deps.queueDepth(),
      statusLine: deps.statusLine(),
    }),
    promptPlaceholder: () => derivePromptPlaceholder({
      attached: deps.isAttached(),
      workflowScreenActive: deps.workflowScreenActive(),
      workflowPromptState: deps.workflowPromptState(),
      attachedPlaceholder: deps.attachedPlaceholder,
      detachedPlaceholder: deps.detachedPlaceholder,
    }),
    promptAreaBackground: () => {
      deps.trackThemeRevision?.()
      return derivePromptAreaBackground({
        attached: deps.isAttached(),
        workflowScreenActive: deps.workflowScreenActive(),
        attachedBackground: deps.attachedBackground(),
        detachedBackground: deps.detachedBackground(),
        workflowBackground: deps.workflowBackground(),
      })
    },
  }
}
