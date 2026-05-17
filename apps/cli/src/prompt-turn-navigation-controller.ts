import {
  findTurnPromptScrollTarget,
  promptTurnNavigationDirectionForKey,
} from "./history-viewport.js"

export type PromptTurnNavigationKeyEvent = {
  name: string
  eventType: string
  shift?: boolean
}

export type PromptTurnNavigationControllerDeps = {
  isAttached: () => boolean
  getPromptText: () => string | undefined
  getPromptOffsets: () => number[]
  getScrollState: () => { left: number; top: number } | null
  scrollTo: (position: { x: number; y: number }) => void
  requestRender: () => void
  setLastTranscriptScrollTop: (scrollTop: number) => void
}

export type PromptTurnNavigationController = {
  handleKey(event: PromptTurnNavigationKeyEvent): boolean
}

export function createPromptTurnNavigationController(
  deps: PromptTurnNavigationControllerDeps,
): PromptTurnNavigationController {
  return {
    handleKey(event) {
      const direction = promptTurnNavigationDirectionForKey({
        attached: deps.isAttached(),
        keyName: event.name,
        eventType: event.eventType,
        shift: event.shift,
        promptText: deps.getPromptText(),
      })
      if (!direction) {
        return false
      }
      const scrollState = deps.getScrollState()
      if (!scrollState) {
        return true
      }
      const promptOffsets = deps.getPromptOffsets().sort((left, right) => left - right)
      const target = findTurnPromptScrollTarget(promptOffsets, scrollState.top, direction)
      if (target === null || target === undefined) {
        return true
      }
      deps.scrollTo({ x: scrollState.left, y: target })
      deps.requestRender()
      deps.setLastTranscriptScrollTop(deps.getScrollState()?.top ?? target)
      return true
    },
  }
}
