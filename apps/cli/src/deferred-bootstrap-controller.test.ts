import assert from "node:assert/strict"
import test from "node:test"

import type {
  BootstrapDeferredState,
  SessionHistoryCursorState,
  TranscriptEntry,
} from "./cli-types.js"
import { createDeferredBootstrapController } from "./deferred-bootstrap-controller.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"

test("deferred bootstrap controller applies provider catalog hydration", async () => {
  const providerCatalog = {} as ProviderCatalog
  const providerCommandCatalogs = {} as ProviderCommandCatalogs
  const harness = bootstrapHarness({
    deferred: {
      providerCatalog: Promise.resolve(providerCatalog),
      providerCommandCatalogs: Promise.resolve(providerCommandCatalogs),
    },
  })

  harness.controller.apply()
  await flushPromises()

  assert.equal(harness.providerCatalog, providerCatalog)
  assert.equal(harness.providerCommandCatalogs, providerCommandCatalogs)
  assert.equal(harness.chromeUpdates, 1)
})

test("deferred bootstrap controller replaces matching attached history", async () => {
  const harness = bootstrapHarness({
    attachmentSessionId: "session-1",
    deferred: {
      attachedHistory: Promise.resolve({
        sessionId: "session-1",
        visibleAgentId: "agent-1",
        agentEntries: {
          "agent-1": [entry(1, "assistant", "hello")],
        },
        historyEntries: [entry(1, "assistant", "hello")],
        promptHistoryEntries: ["hello"],
        nextHistoryCursor: null,
      }),
    },
  })

  harness.controller.apply()
  await flushPromises()

  assert.deepEqual(harness.promptHistoryEntries, ["hello"])
  assert.equal(harness.promptHistoryResetCount, 1)
  assert.equal(harness.nextHistoryCursor, null)
  assert.deepEqual(harness.agentPaneEntries["agent-1"]?.map((item) => item.text), ["hello"])
  assert.equal(harness.agentPanePreviews["agent-1"], "Asst: hello")
  assert.deepEqual(harness.replaceCalls, [{ agentId: "agent-1", entries: ["hello"] }])
  assert.deepEqual(harness.prependCalls, [])
})

test("deferred bootstrap controller prepends attached history when transcript already exists", async () => {
  const harness = bootstrapHarness({
    attachmentSessionId: "session-1",
    currentTranscriptEntryCount: 1,
    entryCounter: 7,
    deferred: {
      attachedHistory: Promise.resolve({
        sessionId: "session-1",
        visibleAgentId: "agent-1",
        agentEntries: {
          "agent-1": [entry(1, "assistant", "older")],
        },
        historyEntries: [entry(1, "assistant", "older")],
        promptHistoryEntries: ["older"],
        nextHistoryCursor: null,
      }),
    },
  })

  harness.controller.apply()
  await flushPromises()

  assert.deepEqual(harness.replaceCalls, [])
  assert.deepEqual(harness.prependCalls, [[{ id: 8, text: "older" }]])
})

test("deferred bootstrap controller ignores stale attached history", async () => {
  const harness = bootstrapHarness({
    attachmentSessionId: "current-session",
    deferred: {
      attachedHistory: Promise.resolve({
        sessionId: "stale-session",
        visibleAgentId: "agent-1",
        agentEntries: {
          "agent-1": [entry(1, "assistant", "stale")],
        },
        historyEntries: [entry(1, "assistant", "stale")],
        promptHistoryEntries: ["stale"],
        nextHistoryCursor: null,
      }),
    },
  })

  harness.controller.apply()
  await flushPromises()

  assert.deepEqual(harness.promptHistoryEntries, [])
  assert.deepEqual(harness.replaceCalls, [])
  assert.deepEqual(harness.prependCalls, [])
})

function bootstrapHarness(options: {
  deferred?: BootstrapDeferredState
  attachmentSessionId?: string | null
  currentTranscriptEntryCount?: number
  entryCounter?: number
} = {}) {
  const harness = {
    deferred: options.deferred,
    attachmentSessionId: options.attachmentSessionId ?? null,
    currentTranscriptEntryCount: options.currentTranscriptEntryCount ?? 0,
    entryCounter: options.entryCounter ?? 0,
    providerCatalog: null as ProviderCatalog | null,
    providerCommandCatalogs: null as ProviderCommandCatalogs | null,
    chromeUpdates: 0,
    promptHistoryEntries: [] as string[],
    promptHistoryResetCount: 0,
    nextHistoryCursor: undefined as SessionHistoryCursorState | undefined,
    terminalCommandCatalog: null as unknown,
    agentPaneEntries: {} as Record<string, TranscriptEntry[]>,
    agentPanePreviews: {} as Record<string, string>,
    replaceCalls: [] as Array<{ agentId: string | null; entries: string[] }>,
    prependCalls: [] as Array<Array<{ id: number; text: string }>>,
    warnings: [] as string[],
    controller: null as ReturnType<typeof createDeferredBootstrapController> | null,
  }
  harness.controller = createDeferredBootstrapController({
    getDeferred: () => harness.deferred,
    currentAttachmentSessionId: () => harness.attachmentSessionId,
    currentTranscriptEntryCount: () => harness.currentTranscriptEntryCount,
    entryCounter: () => harness.entryCounter,
    setProviderCatalog: (catalog) => {
      harness.providerCatalog = catalog
    },
    setProviderCommandCatalogs: (catalogs) => {
      harness.providerCommandCatalogs = catalogs
    },
    setTerminalCommandCatalog: (catalog) => {
      harness.terminalCommandCatalog = catalog
    },
    updateSessionChrome: () => {
      harness.chromeUpdates += 1
    },
    setPromptHistoryEntries: (entries) => {
      harness.promptHistoryEntries = entries
    },
    resetPromptHistoryNavigation: () => {
      harness.promptHistoryResetCount += 1
    },
    setNextHistoryCursor: (cursor) => {
      harness.nextHistoryCursor = cursor
    },
    setAgentPaneEntries: (agentId, entries) => {
      harness.agentPaneEntries[agentId] = entries
    },
    setAgentPanePreview: (agentId, preview) => {
      harness.agentPanePreviews[agentId] = preview
    },
    replaceTranscriptEntries: (entries, agentId) => {
      harness.replaceCalls.push({ agentId, entries: entries.map((item) => item.text) })
    },
    prependTranscriptEntries: async (entries) => {
      harness.prependCalls.push(entries.map((item) => ({ id: item.id, text: item.text })))
    },
    logWarning: (message) => {
      harness.warnings.push(message)
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createDeferredBootstrapController>
  }
}

async function flushPromises() {
  await Promise.resolve()
  await Promise.resolve()
}

function entry(
  id: number,
  role: TranscriptEntry["role"],
  text: string,
  overrides: Partial<TranscriptEntry> = {},
): TranscriptEntry {
  return {
    id,
    role,
    text,
    ...overrides,
  }
}
