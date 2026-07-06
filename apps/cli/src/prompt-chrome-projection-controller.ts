import {
  derivePromptAreaBackground,
  derivePromptPlaceholder,
} from "@arroba/kernel-client/prompt-surface-state"
import {
  sessionChromeProjection,
} from "@arroba/kernel-client/shell-session-footer"
import type { WorkflowPromptState } from "@arroba/kernel-client/workflow-prompt-state"

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
  const chromeProjection = () => sessionChromeProjection({
    daemonDisconnected: deps.daemonDisconnected(),
    working: deps.working(),
    hasActivePrompt: deps.hasActivePrompt(),
    submitting: deps.submitting(),
    queueDepth: deps.queueDepth(),
    fatalError: deps.fatalError(),
    activePromptId: deps.activePromptId(),
    statusLine: deps.statusLine(),
  })

  return {
    sessionStatusMode: () => chromeProjection().sessionStatusMode,
    footerHint: () => chromeProjection().footerHint,
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
