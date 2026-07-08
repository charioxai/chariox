import {
  listWorkflowRegistryRequest,
} from "@arroba/kernel-client"

import { getLogger } from "./cli-runtime-singletons.js"
import { workflowRegistrySuggestionEntriesFromResponse } from "./workflow-registry-command-center-entries.js"
import type { CommandCenterWorkflowRegistryEntry } from "./command-center-context.js"

type AnyFn = (...args: any[]) => any

export type WorkflowRegistrySuggestionControllerDeps = Record<string, any> & {
  formatError: AnyFn
}

export function createWorkflowRegistrySuggestionController(
  deps: WorkflowRegistrySuggestionControllerDeps,
) {
  let entries: CommandCenterWorkflowRegistryEntry[] = []
  let sessionId: string | null = null
  let fetchedAtMs = 0
  let fetchInFlight = false
  let resync: (() => void) | null = null

  const invalidate = () => {
    entries = []
    sessionId = null
    fetchedAtMs = 0
  }

  const refresh = (input: string) => {
    if (!shouldRefreshWorkflowRegistrySuggestions(input)) {
      return
    }
    const currentSessionId = deps.sessionState().id
    const nowMs = Date.now()
    if (
      fetchInFlight
      || (sessionId === currentSessionId && nowMs - fetchedAtMs < 5000)
    ) {
      return
    }
    fetchInFlight = true
    void deps.client.send(listWorkflowRegistryRequest(currentSessionId))
      .then((response: Record<string, unknown>) => {
        entries = workflowRegistrySuggestionEntriesFromResponse(response)
        sessionId = currentSessionId
        fetchedAtMs = Date.now()
        resync?.()
      })
      .catch((error: unknown) => {
        getLogger("workflow-registry-suggestions")?.debug(
          "workflow registry suggestion refresh failed",
          { error: deps.formatError(error) },
        )
      })
      .finally(() => {
        fetchInFlight = false
      })
  }

  return {
    entries: () => entries,
    invalidate,
    refresh,
    setResync: (callback: (() => void) | null) => {
      resync = callback
    },
    shouldRefresh: shouldRefreshWorkflowRegistrySuggestions,
    shouldInvalidate: shouldInvalidateWorkflowRegistrySuggestions,
  }
}

function shouldRefreshWorkflowRegistrySuggestions(input: string): boolean {
  const normalized = input.trimStart()
  return normalized.startsWith("/workflow load ")
    || normalized.startsWith("/workflow run ")
    || normalized.startsWith("/workflow registry get ")
    || normalized.startsWith("/workflow registry delete ")
}

function shouldInvalidateWorkflowRegistrySuggestions(command: string): boolean {
  const normalized = command.trimStart()
  return normalized.startsWith("/workflow load ")
    || normalized.startsWith("/workflow run ")
    || normalized.startsWith("/workflow registry add ")
    || normalized.startsWith("/workflow registry add-from-workflow ")
    || normalized.startsWith("/workflow registry delete ")
}
