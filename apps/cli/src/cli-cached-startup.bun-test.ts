import assert from "node:assert/strict"
import test from "node:test"
import { createComponent, createRoot } from "solid-js"
import { testRender } from "@opentui/solid"

// MP-08 / MP-10 PANE-03: the bundled attach history is on screen before the ~100 ms app mount.
let mounted = false
// Bun-only API, resolved at run time because tsc has no bun:test types.
const bunTest = "bun:test"
const { mock } = await import(bunTest) as { mock: { module(path: string, factory: () => object): void } }
mock.module("./cli-app-root.js", () => ({ CharioxCliApp: () => { mounted = true; return null } }))
const { CliCachedStartup, createCliCachedStartup } = await import("./cli-cached-startup.js")

const entry = (id: number, role: string, text: string) => ({ id, role, text, hidden: false })

test("the first transcript is drawn from the attach reply before the full app mounts", async () => {
  const view = createRoot(() => createCliCachedStartup())
  const historyEntries = [entry(0, "user", "PREVIEW_QUESTION"), entry(1, "assistant", "PREVIEW_ANSWER")]
  const session = { id: "s1", alias: "demo", agents: [] }
  const pending = Promise.resolve({
    kind: "ready",
    bootstrap: {
      binding: { session },
      client: { requestStats: () => ({ count: 1, kinds: { AttachSessionView: 1 }, timeline: [] }) },
      deferred: {
        historyIsBundled: true,
        attachedHistory: Promise.resolve({ sessionId: "s1", visibleAgentId: null, agentEntries: {}, historyEntries, promptHistoryEntries: [], nextHistoryCursor: null }),
      },
    },
  })
  const harness = await testRender(() => createComponent(CliCachedStartup, { pending: pending as never, view }), { width: 80, height: 20, useThread: false })
  // The app's mount state and the text captured at each frame's post-processing; the capture shows the
  // previous frame's buffer, so a frame's text belongs to the frame before it.
  const frames: Array<{ mounted: boolean; text: string }> = []
  harness.renderer.addPostProcessFn(() => { frames.push({ mounted, text: harness.captureCharFrame() }) })
  try {
    for (let attempt = 0; attempt < 40 && !frames.some(frame => frame.text.includes("PREVIEW_ANSWER")); attempt++) {
      await harness.renderOnce()
      await new Promise(resolve => setTimeout(resolve, 5))
    }
    const firstShown = frames.findIndex(frame => frame.text.includes("PREVIEW_ANSWER"))
    const mountedAtFirstPaint = firstShown > 0 ? frames[firstShown - 1]!.mounted : null
    assert.notEqual(mountedAtFirstPaint, null, "the attach history was never drawn")
    assert.equal(mountedAtFirstPaint, false, "the app mounted before the first transcript frame")
    for (let attempt = 0; attempt < 40 && !mounted; attempt++) { await harness.renderOnce(); await new Promise(resolve => setTimeout(resolve, 5)) }
    assert.equal(mounted, true, "the app still mounts after the preview frame")
  } finally { harness.renderer.destroy(); view.finish() }
})

test("a history that is not bundled in the attach reply is not waited for", async () => {
  mounted = false
  const view = createRoot(() => createCliCachedStartup())
  const pending = Promise.resolve({
    kind: "ready",
    bootstrap: {
      binding: { session: { id: "s2", alias: "demo", agents: [] } },
      client: { requestStats: () => ({ count: 0, kinds: {}, timeline: [] }) },
      deferred: { attachedHistory: new Promise(() => {}) },
    },
  })
  const harness = await testRender(() => createComponent(CliCachedStartup, { pending: pending as never, view }), { width: 80, height: 20, useThread: false })
  try {
    for (let attempt = 0; attempt < 40 && !mounted; attempt++) { await harness.renderOnce(); await new Promise(resolve => setTimeout(resolve, 5)) }
    assert.equal(mounted, true)
  } finally { harness.renderer.destroy(); view.finish() }
})

test("fresh attach history replaces a stale cached view before the full app mounts", async () => {
  mounted = false
  const cachedView = { scope: { owner: "o", kernel: "k" }, sessionId: "s3", agentId: "agent-1", alias: "demo", sequence: 1, savedAt: 1, entries: [entry(0, "assistant", "CACHED_STALE_ROW")] }
  const view = { cached: () => cachedView, finish() {}, onClientReady() {} }
  const session = { id: "s3", alias: "demo", agents: [{ id: "agent-1" }] }
  const pending = Promise.resolve({
    kind: "ready",
    bootstrap: {
      binding: { session },
      client: { requestStats: () => ({ count: 1, kinds: {}, timeline: [] }) },
      deferred: {
        historyIsBundled: true,
        attachedHistory: Promise.resolve({ sessionId: "s3", visibleAgentId: "agent-1", agentEntries: {}, historyEntries: [entry(0, "assistant", "FRESH_KERNEL_ROW")], promptHistoryEntries: [], nextHistoryCursor: null }),
      },
    },
  })
  const harness = await testRender(() => createComponent(CliCachedStartup, { pending: pending as never, view: view as never }), { width: 80, height: 20, useThread: false })
  const frames: Array<{ mounted: boolean; text: string }> = []
  harness.renderer.addPostProcessFn(() => { frames.push({ mounted, text: harness.captureCharFrame() }) })
  try {
    for (let attempt = 0; attempt < 40 && !frames.some(frame => frame.text.includes("FRESH_KERNEL_ROW")); attempt++) {
      await harness.renderOnce()
      await new Promise(resolve => setTimeout(resolve, 5))
    }
    const firstShown = frames.findIndex(frame => frame.text.includes("FRESH_KERNEL_ROW"))
    assert.ok(firstShown > 0, "the fresh attach history was never drawn")
    assert.equal(frames[firstShown - 1]!.mounted, false, "the app mounted before the fresh history frame")
  } finally { harness.renderer.destroy() }
})

// MP-08 / MP-10: the hosted pairing-link TUI once showed only "Unable to attach to this session."
// and the reason was dropped, so nobody could tell what failed.
test("a failed attach says why", async () => {
  const view = createRoot(() => createCliCachedStartup())
  const pending = Promise.reject(new Error("session 18e1666aae78beab is not visible to this user"))
  const harness = await testRender(() => createComponent(CliCachedStartup, { pending: pending as never, view }), { width: 100, height: 20, useThread: false })
  try {
    let text = ""
    for (let attempt = 0; attempt < 40 && !text.includes("Unable to attach"); attempt++) {
      await harness.renderOnce()
      text = harness.captureCharFrame()
      await new Promise(resolve => setTimeout(resolve, 5))
    }
    assert.match(text, /Unable to attach to this session: session 18e1666aae78beab is not visible to this user/)
  } finally { harness.renderer.destroy(); view.finish() }
})
