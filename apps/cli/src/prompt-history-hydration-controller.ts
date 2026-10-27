import type { PromptInputHistoryPage } from "./cli-types.js"
import {
  extractPromptInputHistoryEntries,
  maxPromptInputHistorySequence,
} from "@chariox/kernel-client/prompt-history"

type PromptHistoryHydrationControllerOptions = {
  loadHistory: (sessionId: string) => Promise<PromptInputHistoryPage>
  isCurrentSession: (sessionId: string) => boolean
  applyHistory: (sessionId: string, entries: string[], latestSequence: number) => Promise<void> | void
}

export type PromptHistoryHydrationController = {
  begin(): number
  invalidate(): void
  hydrate(sessionId: string): Promise<void>
  loadAndApply(sessionId: string, generation: number): Promise<void>
}

export function createPromptHistoryHydrationController(
  options: PromptHistoryHydrationControllerOptions,
): PromptHistoryHydrationController {
  let currentGeneration = 0

  const controller: PromptHistoryHydrationController = {
    begin() {
      currentGeneration += 1
      return currentGeneration
    },
    invalidate() {
      currentGeneration += 1
    },
    async hydrate(sessionId) {
      const generation = controller.begin()
      await controller.loadAndApply(sessionId, generation)
    },
    async loadAndApply(sessionId, generation) {
      const history = await options.loadHistory(sessionId)
      if (generation !== currentGeneration || !options.isCurrentSession(sessionId)) {
        return
      }
      await options.applyHistory(
        sessionId,
        extractPromptInputHistoryEntries(history.entries),
        maxPromptInputHistorySequence(history.entries),
      )
    },
  }

  return controller
}
