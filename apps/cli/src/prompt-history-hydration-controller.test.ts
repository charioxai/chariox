import assert from "node:assert/strict"
import test from "node:test"

import type { PromptInputHistoryPage } from "./cli-types.js"
import { createPromptHistoryHydrationController } from "./prompt-history-hydration-controller.js"

function promptInputHistoryPage(texts: string[]): PromptInputHistoryPage {
  return {
    entries: texts.map((text, index) => ({
      sequence: index + 1,
      timestamp_ms: index,
      session_id: "session-1",
      kind: "prompt",
      text,
    })),
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((promiseResolve) => {
    resolve = promiseResolve
  })
  return { promise, resolve }
}

test("hydrate loads current session prompt history and applies latest sequence", async () => {
  let applied: { sessionId: string, entries: string[], latestSequence: number } | null = null
  const controller = createPromptHistoryHydrationController({
    loadHistory: async () => promptInputHistoryPage(["one", "two"]),
    isCurrentSession: (sessionId) => sessionId === "session-1",
    applyHistory: (sessionId, entries, latestSequence) => {
      applied = { sessionId, entries, latestSequence }
    },
  })

  await controller.hydrate("session-1")

  assert.deepEqual(applied, {
    sessionId: "session-1",
    entries: ["one", "two"],
    latestSequence: 2,
  })
})

test("loadAndApply drops results when a newer generation has started", async () => {
  let applied = false
  const controller = createPromptHistoryHydrationController({
    loadHistory: async () => promptInputHistoryPage(["stale"]),
    isCurrentSession: () => true,
    applyHistory: () => {
      applied = true
    },
  })

  const generation = controller.begin()
  controller.begin()
  await controller.loadAndApply("session-1", generation)

  assert.equal(applied, false)
})

test("loadAndApply drops results when the session is no longer current", async () => {
  let applied = false
  const controller = createPromptHistoryHydrationController({
    loadHistory: async () => promptInputHistoryPage(["detached"]),
    isCurrentSession: () => false,
    applyHistory: () => {
      applied = true
    },
  })

  await controller.hydrate("session-1")

  assert.equal(applied, false)
})

test("invalidate prevents an in-flight hydration from applying", async () => {
  const pending = deferred<PromptInputHistoryPage>()
  let applied = false
  const controller = createPromptHistoryHydrationController({
    loadHistory: async () => pending.promise,
    isCurrentSession: () => true,
    applyHistory: () => {
      applied = true
    },
  })

  const hydrated = controller.hydrate("session-1")
  controller.invalidate()
  pending.resolve(promptInputHistoryPage(["late"]))
  await hydrated

  assert.equal(applied, false)
})
