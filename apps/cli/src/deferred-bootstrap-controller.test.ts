import assert from "node:assert/strict"
import test from "node:test"

import type {
  BootstrapDeferredState,
  RuntimeProviderRun,
  SessionHistoryCursorState,
  TranscriptEntry,
} from "./cli-types.js"
import { createDeferredBootstrapController } from "./deferred-bootstrap-controller.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"

test("MP-08 / MP-10: delayed composer history belongs to its unchanged startup attachment and input", async () => {
  for (const change of ["none", "attachment", "input", "navigation"]) {
    let resolve!: (value: { sessionId: string; entries: string[] }) => void
    const harness = bootstrapHarness({ attachmentSessionId: "session-1", attachmentId: "attachment-1",
      deferred: { promptInputHistory: new Promise(r => { resolve = r }) } })
    harness.controller.apply()
    if (change === "attachment") harness.attachmentId = "attachment-2"
    if (change === "input") harness.promptHistoryEntries = ["fresh user prompt"]
    if (change === "navigation") { harness.promptHistoryIndex = 0; harness.promptHistoryDraft = "draft" }
    resolve({ sessionId: "session-1", entries: ["earlier input"] })
    await flushPromises()
    assert.deepEqual(harness.promptHistoryEntries, change === "none" ? ["earlier input"] : change === "input" ? ["fresh user prompt"] : [])
    assert.equal(harness.promptHistoryResetCount, change === "none" ? 1 : 0)
  }
})

test("MP-08 / MP-10 composer inventory stays lazy and rejects a superseded attachment", async () => {
  for (const replace of [false, true]) {
    let finish!: (value: Awaited<ReturnType<NonNullable<BootstrapDeferredState["loadComposer"]>>>) => void
    let calls = 0
    const h = bootstrapHarness({ attachmentSessionId: "session-1", attachmentId: "binding-1", deferred: {
      loadComposer: () => { calls++; return new Promise(resolve => { finish = resolve }) },
    } })
    h.controller.apply(); assert.equal(calls, 0)
    const pending = h.controller.hydrateComposer(); void h.controller.hydrateComposer(); assert.equal(calls, 1)
    if (replace) h.attachmentId = "binding-2"
    const catalog = { all: [], connected: [], default: {} } as ProviderCatalog
    finish({ sessionId: "session-1", providerCatalog: catalog, providerCommandCatalogs: {} as ProviderCommandCatalogs,
      terminalCommandCatalog: { revision: "one", nodes: [] }, promptHistoryEntries: ["earlier"] })
    await pending
    assert.equal(h.providerCatalog, replace ? null : catalog)
    assert.deepEqual(h.promptHistoryEntries, [])
  }
})

// MP-08 / MP-10 / MP-11: catalog refresh and prompt navigation own independent data.
test("a composer scope refresh preserves prompt history already loaded on this attachment", async () => {
  let scope = "codex:first", historyCalls = 0
  const catalog = {all:[],connected:[],default:{}} as ProviderCatalog
  const h = bootstrapHarness({attachmentSessionId:"session-1",attachmentId:"binding-1",deferred:{
    composerScopeKey:()=>scope,
    loadPromptInputHistory:async()=>{historyCalls++;return {sessionId:"session-1",entries:["earlier input"]}},
    loadComposer:async()=>({sessionId:"session-1",providerCatalog:catalog,providerCommandCatalogs:{} as ProviderCommandCatalogs,terminalCommandCatalog:{revision:"one",nodes:[]},promptHistoryEntries:[]}),
  }})
  await h.controller.hydratePromptInputHistory()
  assert.deepEqual(h.promptHistoryEntries,["earlier input"])
  await h.controller.hydrateComposer()
  scope="claude:second"
  await h.controller.hydrateComposer()
  await h.controller.hydratePromptInputHistory()
  assert.deepEqual(h.promptHistoryEntries,["earlier input"])
  assert.equal(historyCalls,1)
})

test("MP-08/MP-10 a provider launch starts after mount, once, for the attached session", async () => {
  let starts = 0
  const harness = bootstrapHarness({
    attachmentSessionId: "session-1",
    deferred: { providerLaunch: {
      sessionId: "session-1", agentId: "agent-1", isPending: () => true,
      start: async () => { starts++; return { providerRun: null, issue: null } },
    } },
  })
  assert.equal(starts, 0)
  harness.controller.apply()
  harness.controller.apply()
  await flushPromises()
  assert.equal(starts, 1)
  assert.equal(harness.chromeUpdates, 1)
})

test("MP-08/MP-10 leaving a bootstrap session prevents its deferred provider launch", async () => {
  let starts = 0
  const harness = bootstrapHarness({
    attachmentSessionId: "other-session",
    deferred: { providerLaunch: {
      sessionId: "session-1", agentId: "agent-1", isPending: () => true,
      start: async () => { starts++; return { providerRun: null, issue: null } },
    } },
  })
  harness.controller.apply()
  await flushPromises()
  assert.equal(starts, 0)
})

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

test("deferred bootstrap controller keeps the session hydrating until attached history settles", async () => {
  let resolveHistory!: (value: Awaited<NonNullable<BootstrapDeferredState["attachedHistory"]>>) => void
  const harness = bootstrapHarness({
    attachmentSessionId: "session-1",
    deferred: {
      attachedHistory: new Promise((resolve) => {
        resolveHistory = resolve
      }),
    },
  })

  harness.controller.apply()
  await flushPromises()
  assert.deepEqual(harness.hydrating, [])

  resolveHistory({
    sessionId: "session-1",
    visibleAgentId: "agent-1",
    agentEntries: { "agent-1": [entry(1, "assistant", "hello")] },
    historyEntries: [entry(1, "assistant", "hello")],
    promptHistoryEntries: [],
    nextHistoryCursor: null,
  })
  await new Promise((resolve) => setImmediate(resolve))
  assert.deepEqual(harness.replaceCalls, [{ agentId: "agent-1", entries: ["hello"] }])
  assert.deepEqual(harness.hydrating, [false])

  const failed = bootstrapHarness({
    attachmentSessionId: "session-1",
    deferred: { attachedHistory: Promise.reject(new Error("offline")) },
  })
  failed.controller.apply()
  await new Promise((resolve) => setImmediate(resolve))
  assert.deepEqual(failed.hydrating, [false])
})

// MP-08 / MP-10 (panes R1 review): a startup history that settles after the
// user attached elsewhere must not clear the newer attach's hydrating state.
test("deferred bootstrap controller leaves hydration to a newer attachment", async () => {
  let resolveHistory!: (value: Awaited<NonNullable<BootstrapDeferredState["attachedHistory"]>>) => void
  const harness = bootstrapHarness({
    attachmentSessionId: "session-a",
    attachmentId: "attachment-a",
    deferred: {
      attachedHistory: new Promise((resolve) => {
        resolveHistory = resolve
      }),
    },
  })

  harness.controller.apply()
  harness.attachmentSessionId = "session-b"
  harness.attachmentId = "attachment-b"
  resolveHistory({
    sessionId: "session-a",
    visibleAgentId: "agent-1",
    agentEntries: { "agent-1": [entry(1, "assistant", "stale")] },
    historyEntries: [entry(1, "assistant", "stale")],
    promptHistoryEntries: [],
    nextHistoryCursor: null,
  })
  await new Promise((resolve) => setImmediate(resolve))

  assert.deepEqual(harness.replaceCalls, [])
  assert.deepEqual(harness.hydrating, [])
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

test("deferred bootstrap controller applies the attach-time launch only to its session", async () => {
  const locked = { sessionId: "session-1", session: null, providerRun: null, issue: "credential_vault_locked" as const }
  const harness = bootstrapHarness({ attachmentSessionId: "session-1", deferred: { attachLaunch: Promise.resolve(locked) } })
  harness.controller.apply()
  await flushPromises()
  assert.deepEqual(harness.launches, [locked])
  const switched = bootstrapHarness({ attachmentSessionId: "session-2", deferred: { attachLaunch: Promise.resolve(locked) } })
  switched.controller.apply()
  await flushPromises()
  assert.deepEqual(switched.launches, [])
})

test("MP-08 / MP-10 PANE-03: the attach-time launch result is applied to the attachment that started it", async () => {
  const launch = { sessionId: "s", session: null, providerRun: null, issue: "credential_vault_locked" as const }
  const harness = bootstrapHarness({ deferred: { attachLaunch: Promise.resolve(launch) }, attachmentSessionId: "s", attachmentId: "a" })
  harness.controller!.apply()
  await flushPromises()
  assert.deepEqual(harness.launches, [launch])

  const moved = bootstrapHarness({ deferred: { attachLaunch: Promise.resolve(launch) }, attachmentSessionId: "s", attachmentId: "a" })
  moved.controller!.apply()
  moved.attachmentId = "b"
  await flushPromises()
  assert.deepEqual(moved.launches, [])
})

test("MP-08 / MP-10 PANE-03: a failed attach-time launch is reported, never swallowed", async () => {
  const harness = bootstrapHarness({ deferred: { attachLaunch: Promise.reject(new Error("launch refused")) }, attachmentSessionId: "s", attachmentId: "a" })
  harness.controller!.apply()
  await flushPromises()
  assert.deepEqual(harness.launches, [])
  assert.equal(harness.launchFailures.length, 1)
  assert.match(harness.launchFailures[0]!, /launch refused/)
})

// MP-08/MP-10/MP-11 F02: a late protected launch cannot overwrite another Room.
test("deferred provider launch is attachment scoped", async () => {
  let finish!: (value: { providerRun: RuntimeProviderRun; issue: null }) => void
  const result = new Promise<{ providerRun: RuntimeProviderRun; issue: null }>(resolve => { finish = resolve })
  const h = bootstrapHarness({ attachmentSessionId: "room", deferred: {
    providerLaunch: { sessionId: "room", agentId: null, isPending: () => true, start: () => result },
  } })
  h.controller.apply()
  h.attachmentSessionId = "other-room"
  finish({ providerRun: { id: "old-run" } as RuntimeProviderRun, issue: null })
  await flushPromises()
  assert.deepEqual(h.providerRuns, [])
})

// MP-08/MP-10/MP-11: a locked vault keeps its recovery guidance after deferred startup.
test("deferred provider launch reports a locked vault only for the current attachment", async () => {
  const start = () => Promise.resolve({ providerRun: null, issue: "credential_vault_locked" as const })
  const current = bootstrapHarness({ attachmentSessionId: "room", deferred: { providerLaunch: { sessionId: "room", agentId: null, isPending: () => true, start } } })
  current.controller.apply()
  await flushPromises()
  assert.deepEqual(current.launches, [{ sessionId: "room", session: null, providerRun: null, issue: "credential_vault_locked" }])
  const stale = bootstrapHarness({ attachmentSessionId: "room", deferred: { providerLaunch: { sessionId: "room", agentId: null, isPending: () => true,
    start: async () => { stale.attachmentSessionId = "other"; return start() } } } })
  stale.controller.apply()
  await flushPromises()
  assert.deepEqual(stale.launches, [])
})

function bootstrapHarness(options: {
  deferred?: BootstrapDeferredState
  attachmentSessionId?: string | null
  attachmentId?: string | null
  currentTranscriptEntryCount?: number
  entryCounter?: number
} = {}) {
  const harness = {
    deferred: options.deferred,
    attachmentSessionId: options.attachmentSessionId ?? null,
    attachmentId: options.attachmentId ?? null,
    currentTranscriptEntryCount: options.currentTranscriptEntryCount ?? 0,
    entryCounter: options.entryCounter ?? 0,
    providerCatalog: null as ProviderCatalog | null,
    providerCommandCatalogs: null as ProviderCommandCatalogs | null,
    chromeUpdates: 0,
    providerRuns: [] as unknown[],
    promptHistoryEntries: [] as string[],
    promptHistoryResetCount: 0,
    promptHistoryIndex: null as number | null,
    promptHistoryDraft: null as string | null,
    nextHistoryCursor: undefined as SessionHistoryCursorState | undefined,
    terminalCommandCatalog: null as unknown,
    agentPaneEntries: {} as Record<string, TranscriptEntry[]>,
    agentPanePreviews: {} as Record<string, string>,
    replaceCalls: [] as Array<{ agentId: string | null; entries: string[] }>,
    prependCalls: [] as Array<Array<{ id: number; text: string }>>,
    warnings: [] as string[],
    launches: [] as unknown[],
    launchFailures: [] as string[],
    hydrating: [] as boolean[],
    controller: null as ReturnType<typeof createDeferredBootstrapController> | null,
  }
  harness.controller = createDeferredBootstrapController({
    getDeferred: () => harness.deferred,
    setProviderRun: (run) => { harness.providerRuns.push(run) },
    currentAttachmentSessionId: () => harness.attachmentSessionId,
    currentAttachmentId: () => harness.attachmentId,
    currentPromptHistoryEntries: () => harness.promptHistoryEntries,
    currentPromptHistoryIndex: () => harness.promptHistoryIndex,
    currentPromptHistoryDraft: () => harness.promptHistoryDraft,
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
    applyAttachLaunch: (launch) => { harness.launches.push(launch) },
    reportAttachLaunchFailure: (message) => { harness.launchFailures.push(message) },
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
    setSessionHydratingState: (next) => {
      harness.hydrating.push(next)
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
